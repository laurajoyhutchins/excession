//! Replay externally resolved Maven/Gradle JVM inputs without invoking a build.
//! The provider label is producer-supplied, NEVER an attested Maven/Gradle run.
//! All byte identities and source selections are independently rechecked.
use crate::json::{self, Value, obj};
use crate::{Result, digest, git, refuse};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_JARS: usize = 16;
const MAX_JAR_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CLASSPATH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024;

pub struct ResolvedInputs {
    pub document: Value,
    pub classpath_paths: Vec<PathBuf>,
    pub release: Option<String>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// A path in an external Maven/Gradle cache is not a Git object. To use it
/// safely, insist on an immutable content digest and re-read exact bytes on
/// import, audit and selection. The local path must remain available.
fn jar(path: &str, expected: &str, size: u64) -> Result<PathBuf> {
    if !valid_sha256(expected) || size == 0 || size > MAX_JAR_BYTES {
        return refuse("invalid or oversized external JVM artifact identity");
    }
    let literal = Path::new(path);
    if !literal.is_absolute()
        || !path.ends_with(".jar")
        || path.chars().any(char::is_control)
        || path.len() > 4096
    {
        return refuse("JVM artifact must be an absolute regular .jar path");
    }
    let metadata = fs::symlink_metadata(literal)
        .map_err(|e| format!("JVM artifact not available for replay: {e}"))?;
    if !metadata.file_type().is_file() || metadata.len() != size {
        return refuse("JVM artifact is not a regular file or size changed");
    }
    let real = literal
        .canonicalize()
        .map_err(|e| format!("JVM artifact path: {e}"))?;
    if real != literal || !real.is_file() {
        return refuse("JVM artifact path must be canonical and not a symlink");
    }
    let data = fs::read(&real).map_err(|e| format!("JVM artifact read: {e}"))?;
    if data.len() as u64 != size || digest::sha256(&data) != expected {
        return refuse("JVM artifact SHA-256 differs from resolved-input receipt");
    }
    Ok(real)
}

pub struct Capture<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub source_prefix: &'a str,
    pub provider_claim: &'a str,
    pub classpath_file: &'a Path,
    pub release: Option<&'a str>,
}

/// Capture an already exported classpath from trusted CI. This operation
/// executes Git read-only and hashes artifacts: no Maven/Gradle invocation.
pub fn capture(request: &Capture<'_>) -> Result<Value> {
    if request.source_prefix != "." {
        git::validate_path(request.source_prefix)?;
    }
    if !matches!(request.provider_claim, "maven" | "gradle") {
        return refuse("JVM classpath provider claim must be maven or gradle");
    }
    let repo = request
        .repo
        .canonicalize()
        .map_err(|e| format!("resolved JVM checkout: {e}"))?;
    git::verify_repo(&repo)?;
    git::clean(&repo)?;
    let commit = git::rev(&repo, request.revision)?;
    if git::rev(&repo, "HEAD")? != commit {
        return refuse("JVM classpath capture requires selected clean HEAD");
    }
    let tree = git::rev(&repo, &format!("{commit}^{{tree}}"))?;
    let mut sources = git::entries(&repo, &commit)?
        .into_iter()
        .filter(|entry| {
            entry.kind == "blob"
                && matches!(entry.mode.as_str(), "100644" | "100755")
                && entry.path.ends_with(".java")
                && (request.source_prefix == "."
                    || entry
                        .path
                        .starts_with(&format!("{}/", request.source_prefix)))
        })
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    sources.sort();
    if sources.is_empty() || sources.len() > 128 {
        return refuse("resolved JVM source set requires 1..=128 tracked Java files");
    }
    let size = fs::metadata(request.classpath_file)
        .map_err(|e| format!("JVM classpath export metadata: {e}"))?
        .len();
    if size > MAX_MANIFEST_BYTES {
        return refuse("JVM classpath export exceeds 128 KiB");
    }
    let bytes =
        fs::read(request.classpath_file).map_err(|e| format!("JVM classpath export read: {e}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return refuse("JVM classpath export grew beyond 128 KiB");
    }
    let raw = std::str::from_utf8(&bytes).map_err(|_| "JVM classpath export must be UTF-8")?;
    let line = raw.trim_end_matches(&['\r', '\n'][..]);
    if line.contains('\n') || line.contains('\r') || line.is_empty() {
        return refuse("JVM classpath export must contain one platform-separated path list");
    }
    let paths = std::env::split_paths(line).collect::<Vec<_>>();
    if paths.is_empty() || paths.len() > MAX_JARS {
        return refuse("JVM classpath export requires 1..=16 JARs");
    }
    let mut classpath = Vec::new();
    for path in paths {
        let path = path
            .to_str()
            .ok_or("JVM classpath export contains non-UTF8 path")?;
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| format!("JVM classpath artifact metadata: {e}"))?;
        if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_JAR_BYTES
        {
            return refuse("JVM classpath artifact is not a bounded regular JAR file");
        }
        let data = fs::read(path).map_err(|e| format!("JVM classpath artifact read: {e}"))?;
        if data.len() as u64 != metadata.len() {
            return refuse("JVM classpath artifact size changed while hashing");
        }
        classpath.push(obj(&[
            ("path", Value::string(path)),
            ("size_bytes", Value::number(data.len())),
            ("sha256", Value::string(digest::sha256(&data))),
        ]));
    }
    let document = obj(&[
        ("schema", Value::string("excession.jvm-resolved-inputs.v1")),
        ("provider_claim", Value::string(request.provider_claim)),
        ("source_commit", Value::string(&commit)),
        ("source_tree", Value::string(&tree)),
        ("source_prefix", Value::string(request.source_prefix)),
        ("sources", Value::array(sources.iter().map(Value::string))),
        ("classpath", Value::array(classpath)),
        (
            "release",
            request.release.map_or(Value::Null, Value::string),
        ),
    ]);
    verify(&document, &commit, &tree, request.source_prefix, &sources)?;
    Ok(document)
}

pub fn load(path: &Path) -> Result<Value> {
    let size = fs::metadata(path)
        .map_err(|e| format!("resolved JVM manifest metadata: {e}"))?
        .len();
    if size > MAX_MANIFEST_BYTES {
        return refuse("resolved JVM manifest exceeds 128 KiB");
    }
    let bytes = fs::read(path).map_err(|e| format!("resolved JVM manifest read: {e}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return refuse("resolved JVM manifest grew beyond 128 KiB");
    }
    json::parse(&bytes)
}

/// Validate the previously recorded, externally supplied compiler inputs.
/// No Maven/Gradle executable or project build script is started here.
pub fn verify(
    document: &Value,
    commit: &str,
    tree: &str,
    prefix: &str,
    sources: &[String],
) -> Result<ResolvedInputs> {
    if document.entries()?.len() != 8
        || document.get("schema")?.str()? != "excession.jvm-resolved-inputs.v1"
        || !matches!(document.get("provider_claim")?.str()?, "maven" | "gradle")
        || document.get("source_commit")?.str()? != commit
        || document.get("source_tree")?.str()? != tree
        || document.get("source_prefix")?.str()? != prefix
    {
        return refuse("resolved JVM inputs have wrong schema, provider claim or Git revision");
    }
    let input_sources = document.get("sources")?.array_ref()?;
    if input_sources.len() != sources.len() || input_sources.is_empty() {
        return refuse("resolved JVM inputs do not match the selected source set");
    }
    for (expected, item) in sources.iter().zip(input_sources) {
        let path = item.str()?;
        git::validate_path(path)?;
        if path != expected {
            return refuse("resolved JVM sources differ from exact tracked compilation sources");
        }
    }
    let release = match document.get("release")? {
        Value::Null => None,
        Value::String(s) => {
            let value = s.parse::<u32>().map_err(|_| "invalid JVM --release")?;
            if !(8..=30).contains(&value) || s != &value.to_string() {
                return refuse("unsupported JVM --release value");
            }
            Some(s.clone())
        }
        _ => return refuse("JVM --release must be a version string or null"),
    };
    let declared = document.get("classpath")?.array_ref()?;
    if declared.is_empty() || declared.len() > MAX_JARS {
        return refuse("resolved JVM classpath must contain 1..=16 JARs");
    }
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    let mut total = 0u64;
    for item in declared {
        if item.entries()?.len() != 3 {
            return refuse("unsupported JVM classpath entry fields");
        }
        let value = item.get("path")?.str()?;
        if !seen.insert(value.to_owned()) {
            return refuse("duplicate external JVM artifact");
        }
        let size = item.get("size_bytes")?.integer()?;
        if size <= 0 {
            return refuse("invalid JVM artifact size");
        }
        total = total
            .checked_add(size as u64)
            .ok_or("JVM artifact byte budget overflow")?;
        if total > MAX_CLASSPATH_BYTES {
            return refuse("resolved JVM classpath exceeds 64 MiB");
        }
        paths.push(jar(value, item.get("sha256")?.str()?, size as u64)?);
    }
    Ok(ResolvedInputs {
        document: document.clone(),
        classpath_paths: paths,
        release,
    })
}

impl ResolvedInputs {
    /// Snapshot immutable JAR bytes into private compiler scratch space.
    /// The JDK never reads the mutable Maven/Gradle cache directly.
    pub fn stage_classpath(&self, scratch: &Path) -> Result<String> {
        let destination = scratch.join("jars");
        fs::create_dir(&destination).map_err(|e| format!("stage JVM classpath: {e}"))?;
        let expected = self.document.get("classpath")?.array_ref()?;
        let mut staged = Vec::new();
        for (i, (source, record)) in self.classpath_paths.iter().zip(expected).enumerate() {
            let target = destination.join(format!("{i}.jar"));
            fs::copy(source, &target).map_err(|e| format!("snapshot JVM artifact: {e}"))?;
            let data = fs::read(&target).map_err(|e| format!("read staged JVM artifact: {e}"))?;
            if data.len() as i64 != record.get("size_bytes")?.integer()?
                || digest::sha256(&data) != record.get("sha256")?.str()?
            {
                return refuse("staged JVM artifact differs from hash-checked input");
            }
            staged.push(target);
        }
        Ok(std::env::join_paths(staged)
            .map_err(|e| format!("staged JVM classpath cannot be represented: {e}"))?
            .to_string_lossy()
            .into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_empty_and_unknown_classpath_receipts() {
        assert!(!valid_sha256("Z".repeat(64).as_str()));
        assert!(load(Path::new("/missing-jvm-manifest")).is_err());
    }
}
