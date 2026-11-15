//! Replay externally resolved Maven/Gradle JVM inputs without invoking a build.
//! The provider label is producer-supplied, NEVER an attested Maven/Gradle run.
//! All byte identities and source selections are independently rechecked.
use crate::json::{self, Value};
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
    pub classpath: String,
    pub release: Option<String>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
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
    let real = literal.canonicalize().map_err(|e| format!("JVM artifact path: {e}"))?;
    if real != literal || !real.is_file() {
        return refuse("JVM artifact path must be canonical and not a symlink");
    }
    let data = fs::read(&real).map_err(|e| format!("JVM artifact read: {e}"))?;
    if data.len() as u64 != size || digest::sha256(&data) != expected {
        return refuse("JVM artifact SHA-256 differs from resolved-input receipt");
    }
    Ok(real)
}

pub fn load(path: &Path) -> Result<Value> {
    let size = fs::metadata(path)
        .map_err(|e| format!("resolved JVM manifest metadata: {e}"))?.len();
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
        total = total.checked_add(size as u64).ok_or("JVM artifact byte budget overflow")?;
        if total > MAX_CLASSPATH_BYTES {
            return refuse("resolved JVM classpath exceeds 64 MiB");
        }
        paths.push(jar(value, item.get("sha256")?.str()?, size as u64)?);
    }
    let classpath = std::env::join_paths(&paths)
        .map_err(|e| format!("JVM classpath path cannot be represented: {e}"))?
        .to_string_lossy().into_owned();
    Ok(ResolvedInputs { document: document.clone(), classpath, release })
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
