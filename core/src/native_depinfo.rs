//! Bind Cargo's compiler-produced dep-info source set to a pinned Git revision.
//!
//! This reads an existing, fresh-build output, not source code or a compiler.
//! Dep-info records compilation inputs, not direct import edges, runtime effects,
//! or a complete dependency proof. The report is a strictly advisory v1 native
//! declaration accepted by the existing language-boundary comparator.
use crate::digest;
use crate::git;
use crate::json::{Value, obj};
use crate::native_boundary;
use crate::{Result, refuse};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DEPFILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOOLCHAIN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_INPUTS: usize = 512;

fn parse_primary_rule(data: &[u8]) -> Result<Vec<&str>> {
    if data.is_empty() || data.len() > MAX_DEPFILE_BYTES {
        return refuse("Cargo dep-info must be 1 to 4 MiB");
    }
    let text = std::str::from_utf8(data).map_err(|_| "Cargo dep-info is not UTF-8")?;
    let primary = text.lines().next().ok_or("empty Cargo dep-info")?;
    let (target, deps) = primary
        .split_once(": ")
        .ok_or("Cargo dep-info has no first dependency rule")?;
    // A path with make-style escaping needs a full Makefile parser. Refuse it
    // rather than silently treating escaped whitespace as separate files.
    if target.is_empty()
        || !target.starts_with('/')
        || target.contains('\\')
        || deps.contains('\\')
        || deps.contains(':')
        || deps.contains('#')
    {
        return refuse("Cargo dep-info uses unsupported target or path escaping");
    }
    let tokens = deps.split_ascii_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() || tokens.len() > MAX_INPUTS {
        return refuse("Cargo dep-info source input count outside 1..512");
    }
    Ok(tokens)
}

fn source_path(repo: &Path, package: &Path, token: &str) -> Result<String> {
    if token.is_empty() || token.contains('\0') || token.contains('\\') {
        return refuse("Cargo dep-info has unsafe input pathname");
    }
    let path = Path::new(token);
    let candidates = if path.is_absolute() {
        vec![path.to_path_buf()]
    } else {
        vec![package.join(path), repo.join(path)]
    };
    let mut chosen = None;
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let resolved = candidate
            .canonicalize()
            .map_err(|e| format!("cannot resolve compiler input: {e}"))?;
        if !resolved.starts_with(repo) {
            return refuse("compiler input escapes pinned Git repository");
        }
        if let Some(previous) = &chosen {
            if previous != &resolved {
                return refuse("relative compiler input is ambiguous across checkout roots");
            }
        } else {
            chosen = Some(resolved);
        }
    }
    let resolved: PathBuf = chosen.ok_or("compiler input is missing from checkout")?;
    let rel = resolved
        .strip_prefix(repo)
        .map_err(|_| "compiler input is not under pinned repository")?;
    let name = rel
        .to_str()
        .ok_or("non-UTF-8 compiler source path unsupported")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    git::validate_path(&name)?;
    Ok(name)
}

fn check_blob(repo: &Path, git_objects: &BTreeMap<String, git::Entry>, path: &str) -> Result<()> {
    let entry = git_objects
        .get(path)
        .ok_or_else(|| format!("compiler input not tracked by Git: {path}"))?;
    if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
        return refuse(format!("compiler input is not a regular Git file: {path}"));
    }
    let data = fs::read(repo.join(path)).map_err(|e| format!("read compiler input {path}: {e}"))?;
    if digest::git_blob(&data) != entry.oid {
        return refuse(format!(
            "compiled file does not match pinned Git blob: {path}"
        ));
    }
    Ok(())
}

pub fn declaration(
    checkout: &Path,
    package_root: &str,
    depfile: &Path,
    subject: &str,
    rustc: &Path,
    version: &str,
    revision: &str,
) -> Result<Value> {
    git::validate_path(package_root)?;
    git::validate_path(subject)?;
    if version.is_empty() || version.len() > 180 {
        return refuse("rustc version is missing or oversized");
    }
    let checkout = checkout.canonicalize().map_err(|e| e.to_string())?;
    git::clean(&checkout)?;
    let head = git::rev(&checkout, "HEAD")?;
    let commit = git::rev(&checkout, revision)?;
    if commit != head {
        return refuse("Cargo dep-info must describe the checked-out Git commit");
    }
    let package = checkout.join(package_root);
    if !package.is_dir() {
        return refuse("Cargo package root not found");
    }
    let depfile_path = depfile.canonicalize().map_err(|e| e.to_string())?;
    if depfile_path.starts_with(&checkout) {
        return refuse("compiler dep-info artifact must be outside the checkout");
    }
    let raw = fs::read(&depfile_path).map_err(|e| e.to_string())?;
    let tokens = parse_primary_rule(&raw)?;
    let git_objects = git::entries(&checkout, &commit)?
        .into_iter()
        .map(|e| (e.path.clone(), e))
        .collect::<BTreeMap<_, _>>();
    let mut inputs = BTreeMap::<String, &str>::new();
    for token in tokens {
        let path = source_path(&checkout, &package, token)?;
        check_blob(&checkout, &git_objects, &path)?;
        let role = if path.ends_with(".rs") {
            "source"
        } else {
            // The v1 declaration has only source/configuration roles.
            // Other compiler-read assets are retained as configuration inputs,
            // without claiming that they configure Rust semantics.
            "configuration"
        };
        if inputs.insert(path.clone(), role).is_some() {
            return refuse("duplicate source in compiler dep-info");
        }
    }
    if inputs.get(subject) != Some(&"source") {
        return refuse("Cargo dep-info omitted required Rust crate entrypoint");
    }
    for config in [
        format!("{package_root}/Cargo.toml"),
        format!("{package_root}/Cargo.lock"),
    ] {
        if git_objects.contains_key(&config) {
            check_blob(&checkout, &git_objects, &config)?;
            inputs.entry(config).or_insert("configuration");
        }
    }
    if inputs.len() > MAX_INPUTS {
        return refuse("native input declaration exceeds 512 tracked paths");
    }

    let binary_path = rustc.canonicalize().map_err(|e| e.to_string())?;
    let binary_metadata = fs::metadata(&binary_path).map_err(|e| e.to_string())?;
    if !binary_metadata.is_file()
        || binary_metadata.len() == 0
        || binary_metadata.len() > MAX_TOOLCHAIN_BYTES
    {
        return refuse("rustc executable is missing or exceeds hash limit");
    }
    let compiler_sha =
        digest::sha256(&fs::read(&binary_path).map_err(|e| format!("rustc hash read: {e}"))?);
    let source_tree = git::rev(&checkout, &format!("{commit}^{{tree}}"))?;
    let payload = obj(&[
        ("schema", Value::string("excession.input-declaration.v1")),
        ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
        ("language", Value::string("rust")),
        ("subject", Value::string(subject)),
        (
            "inputs",
            Value::array(inputs.iter().map(|(path, role)| {
                obj(&[
                    ("path", Value::string(path)),
                    ("role", Value::string(*role)),
                ])
            })),
        ),
        (
            "assumptions",
            obj(&[
                (
                    "dependency-discovery",
                    Value::string("excession-toolchain-native-v1"),
                ),
                ("toolchain-version", Value::string(version)),
                ("toolchain-sha256", Value::string(compiler_sha)),
                (
                    "platform",
                    Value::string(format!(
                        "{}-{}",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    )),
                ),
                (
                    "rust-source-evidence",
                    Value::string("UNATTESTED_CARGO_DEPFILE_SOURCE_SET"),
                ),
                ("cargo-depfile-sha256", Value::string(digest::sha256(&raw))),
                ("source-commit", Value::string(commit)),
                ("source-tree", Value::string(source_tree)),
            ]),
        ),
    ]);
    native_boundary::declaration_preflight(&payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_depfile_is_bounded_and_requires_explicit_paths() {
        let paths = parse_primary_rule(b"/tmp/libexcession.rlib: src/lib.rs src/a.rs\nsrc/a.rs:\n")
            .expect("dep-info is a simple rustc rule");
        assert_eq!(paths, vec!["src/lib.rs", "src/a.rs"]);
        assert!(parse_primary_rule(b"").is_err());
        assert!(parse_primary_rule(b"foo: src/lib.rs").is_err());
        assert!(parse_primary_rule(b"/tmp/foo: src/a\\ b.rs").is_err());
        assert!(parse_primary_rule(b"/tmp/foo: src/a.rs\\\n src/b.rs").is_err());
        assert!(parse_primary_rule(b"/tmp/foo: src/a.rs # comment").is_err());
    }
}
