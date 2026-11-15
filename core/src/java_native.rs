//! JDK-native, self-contained Java source resolution. Compiler APIs own the
//! language semantics; Git owns every admitted source identity. No Maven,
//! Gradle, annotation processors, generated sources, or compiled user code.
use crate::json::{Value, obj};
use crate::{Result, adapter, digest, git, jvm_inputs, refuse};
use percent_encoding::percent_decode_str;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MAX_FILES: usize = 128;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_REFERENCES: usize = 4096;
static NEXT: AtomicU64 = AtomicU64::new(0);
const JAVA_ADAPTER: &str = include_str!("../adapters/ExcessionJavaNative.java");

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        for _ in 0..16 {
            let dir = std::env::temp_dir().join(format!(
                "excession-java-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return refuse(format!("create Java analysis scratch: {e}")),
            }
        }
        refuse("cannot allocate unique Java analysis scratch directory")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git_source(commit: &str, tree: &str, entry: &git::Entry) -> Value {
    obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(&entry.path)),
        ("blob", Value::string(&entry.oid)),
        ("mode", Value::string(&entry.mode)),
    ])
}

fn decode_path(encoded: &str) -> Result<String> {
    let normalized = encoded.replace('+', " ");
    let decoded = percent_decode_str(&normalized)
        .decode_utf8()
        .map_err(|_| "invalid UTF-8 in javac source reference")?;
    let path = decoded.into_owned();
    git::validate_path(&path)?;
    if path.chars().any(char::is_control) {
        return refuse("Java source filenames with control characters are unsupported");
    }
    Ok(path)
}

/// Require one checked-out, clean, tracked Java source set. The JDK's
/// JavacTask.parse+analyze resolve references without generating classes or
/// executing the repository's build scripts. Missing build dependencies
/// are refused rather than guessed.
pub fn scan(
    repo: &Path,
    revision: &str,
    source: &str,
    prefix: &str,
    resolved_manifest: Option<&Path>,
) -> Result<Value> {
    git::validate_path(source)?;
    if !source.ends_with(".java") {
        return refuse("native Java source must end in .java");
    }
    if prefix != "." {
        git::validate_path(prefix)?;
    }
    let root = repo
        .canonicalize()
        .map_err(|e| format!("Java checkout: {e}"))?;
    git::verify_repo(&root)?;
    git::clean(&root)?;
    let commit = git::rev(&root, revision)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("native Java analysis requires the selected clean HEAD");
    }
    let tree = git::rev(&root, &format!("{commit}^{{tree}}"))?;
    let mut entries = git::entries(&root, &commit)?
        .into_iter()
        .filter(|e| {
            e.kind == "blob"
                && matches!(e.mode.as_str(), "100644" | "100755")
                && e.path.ends_with(".java")
                && (prefix == "." || e.path == prefix || e.path.starts_with(&format!("{prefix}/")))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    if entries.is_empty() || entries.len() > MAX_FILES {
        return refuse("Java source set requires 1..=128 tracked files; narrow --source-prefix");
    }
    let by_path = entries
        .iter()
        .map(|e| (e.path.as_str(), e))
        .collect::<BTreeMap<_, _>>();
    let subject = by_path
        .get(source)
        .ok_or("selected Java source is absent from the tracked source set")?;
    let mut bytes = 0usize;
    let mut sources = Vec::new();
    for entry in &entries {
        if entry.path.chars().any(char::is_control) {
            return refuse("Java source paths with control characters are unsupported");
        }
        let absolute = root.join(&entry.path);
        let physical = absolute
            .canonicalize()
            .map_err(|e| format!("Java checkout source missing: {e}"))?;
        if !physical.starts_with(&root) {
            return refuse("Java source resolves outside checkout");
        }
        let content = fs::read(&physical).map_err(|e| format!("read Java checkout source: {e}"))?;
        bytes = bytes
            .checked_add(content.len())
            .ok_or("Java source byte count overflow")?;
        if bytes > MAX_SOURCE_BYTES || digest::git_blob(&content) != entry.oid {
            return refuse("Java source exceeds 4 MiB or differs from pinned Git blob");
        }
        sources.push(physical);
    }
    let source_paths = entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    let resolved = match resolved_manifest {
        Some(path) => Some(jvm_inputs::verify(
            &jvm_inputs::load(path)?,
            &commit,
            &tree,
            prefix,
            &source_paths,
        )?),
        None => None,
    };
    let scratch = Scratch::new()?;
    let helper = scratch.0.join("ExcessionJavaNative.java");
    fs::write(&helper, JAVA_ADAPTER).map_err(|e| format!("write JDK adapter: {e}"))?;
    let empty = scratch.0.join("empty-classpath");
    fs::create_dir(&empty).map_err(|e| format!("create empty Java classpath: {e}"))?;
    let classpath = match &resolved {
        Some(inputs) => inputs.stage_classpath(&scratch.0)?,
        None => empty.to_string_lossy().into_owned(),
    };
    let mut command = Command::new("java");
    command
        .arg(&helper)
        .arg(&root)
        .arg(&empty)
        .arg(&classpath)
        .arg(
            resolved
                .as_ref()
                .and_then(|inputs| inputs.release.as_deref())
                .unwrap_or("-"),
        )
        .args(&sources)
        .current_dir(&scratch.0)
        .env_remove("CLASSPATH")
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS");
    let raw = adapter::run_tool(command, "JDK compiler analysis", Duration::from_secs(75))?;
    let output = std::str::from_utf8(&raw).map_err(|_| "JDK compiler emitted non-UTF8")?;
    let mut lines = output.lines();
    let first = lines.next().ok_or("JDK analysis returned no header")?;
    let version = first
        .strip_prefix("JAVA-NATIVE-V1\t")
        .ok_or("unrecognized JDK compiler output schema")?;
    if version.is_empty() || version.len() > 100 || version.chars().any(char::is_control) {
        return refuse("invalid Java compiler version");
    }
    let mut references = BTreeSet::<(String, String)>::new();
    for line in lines {
        let (from_raw, to_raw) = line
            .split_once('\t')
            .ok_or("invalid JDK reference record")?;
        if to_raw.contains('\t') {
            return refuse("JDK reference record has too many fields");
        }
        let from = decode_path(from_raw)?;
        let to = decode_path(to_raw)?;
        if from == to || !by_path.contains_key(from.as_str()) || !by_path.contains_key(to.as_str())
        {
            return refuse("JDK reference names undeclared or identical Git sources");
        }
        if !references.insert((from, to)) {
            return refuse("duplicate JDK source-reference observation");
        }
        if references.len() > MAX_REFERENCES {
            return refuse("JDK reference output exceeds 4096 observations");
        }
    }
    // Detect even a native producer modifying its inputs while being observed.
    if let Some(inputs) = &resolved {
        jvm_inputs::verify(&inputs.document, &commit, &tree, prefix, &source_paths)?;
    }
    git::clean(&root)?;
    for (entry, path) in entries.iter().zip(sources.iter()) {
        let content = fs::read(path).map_err(|e| format!("recheck Java source: {e}"))?;
        if digest::git_blob(&content) != entry.oid {
            return refuse("Java sources changed during compiler analysis");
        }
    }
    let inputs = entries
        .iter()
        .map(|entry| {
            obj(&[
                ("source", git_source(&commit, &tree, entry)),
                ("role", Value::string("source")),
            ])
        })
        .collect::<Vec<_>>();
    let refs = references
        .into_iter()
        .map(|(from, to)| {
            obj(&[
                ("from", git_source(&commit, &tree, by_path[from.as_str()])),
                ("to", git_source(&commit, &tree, by_path[to.as_str()])),
                ("relation", Value::string("JAVAC_RESOLVED_SOURCE_REFERENCE")),
            ])
        })
        .collect::<Vec<_>>();
    let configuration = if resolved.is_some() {
        "EXTERNAL_RESOLVED_CLASSPATH_NO_PROCESSORS"
    } else {
        "EMPTY_CLASSPATH_AND_SOURCEPATH_NO_PROCESSORS"
    };
    let nested = obj(&[
        ("schema", Value::string("excession.java-native.v1")),
        ("status", Value::string("JAVAC_ANALYZED_PROC_DISABLED")),
        ("source_commit", Value::string(&commit)),
        ("source_tree", Value::string(&tree)),
        ("subject", Value::string(source)),
        ("source_prefix", Value::string(prefix)),
        ("java_version", Value::string(version)),
        (
            "sources",
            Value::array(entries.iter().map(|entry| Value::string(&entry.path))),
        ),
        ("references", Value::array(refs)),
        ("configuration", Value::string(configuration)),
        (
            "resolved_inputs",
            resolved
                .as_ref()
                .map_or(Value::Null, |inputs| inputs.document.clone()),
        ),
    ])
    .stamp_id("report_id")?;
    obj(&[
        ("schema", Value::string("excession.native-analysis.v1")),
        ("status", Value::string("PARTIAL")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        (
            "source",
            git_source(
                &git::rev(&root, "HEAD")?,
                &git::rev(&root, "HEAD^{tree}")?,
                subject,
            ),
        ),
        ("language", Value::string("java")),
        ("producer", Value::string(if resolved.is_some() {
            "javac-configured"
        } else {
            "javac-symbols"
        })),
        (
            "scope",
            Value::string(if resolved.is_some() {
                "JAVAC_CONFIGURED_SOURCE_REFERENCES_PARTIAL"
            } else {
                "JAVAC_RESOLVED_SOURCE_REFERENCES_PARTIAL"
            }),
        ),
        ("execution", Value::string("TRUSTED_LOCAL_NOT_ATTESTED")),
        ("input_sources", Value::number(entries.len())),
        ("input_configurations", Value::number(0)),
        ("inputs", Value::array(inputs)),
        ("observation", nested),
        (
            "limitations",
            Value::array([
                Value::string(
                    "Only the selected tracked self-contained Java source set is analyzed",
                ),
                Value::string(
                    "No annotation processors, project build execution or automatic dependency resolution",
                ),
                Value::string(
                    "External Maven/Gradle provider claim is not independently attested",
                ),
                Value::string(
                    "Resolved source references do not establish call or runtime behavior",
                ),
                Value::string("Trusted-local compiler execution is not independent attestation"),
            ]),
        ),
    ])
    .stamp_id("report_id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoded_paths_are_strict() {
        assert_eq!(
            decode_path("src%2FHello+World.java").unwrap(),
            "src/Hello World.java"
        );
        assert!(decode_path("../A.java").is_err());
        assert!(decode_path("src%2Fbad%09name.java").is_err());
    }
}
