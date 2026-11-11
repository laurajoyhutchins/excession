//! Repository-independent, configuration-scoped C/C++ Clang observations.
//! Compiler and local dependencies are trusted inputs, not Git guarantees.
//! The selected source's Git blob is independently authenticated by Rust.
use crate::json::{Value, obj};
use crate::{Result, adapter, digest, git, refuse};
use std::fs;
use std::path::Path;

const MAX_SOURCE: usize = 1024 * 1024;

/// Reparse one tracked C/C++ translation unit with an existing compilation
/// database. Works for CMake, Meson, Bear, Bazel exports and Linux Kbuild.
/// This is a trusted-local compiler observation, not build verification.
pub fn scan(
    repo: &Path,
    revision: &str,
    path: &str,
    compile_db: &Path,
    compiler: &str,
    config: Option<&Path>,
    object_prefix: Option<&str>,
    trusted_local: &str,
) -> Result<Value> {
    if trusted_local != "true" {
        return refuse("Clang requires --trusted-local true; compilation flags are local code");
    }
    git::validate_path(path)?;
    if ![".c", ".cc", ".cpp", ".cxx", ".C"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
    {
        return refuse("Clang source must be a C/C++ translation unit");
    }
    git::verify_repo(repo)?;
    let commit = git::rev(repo, revision)?;
    if git::text(repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("Clang scan revision must be a commit");
    }
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let blob = git::blob(repo, &commit, path)?;
    if git::text(repo, &["cat-file", "-t", &blob])? != "blob" {
        return refuse("selected source is not a Git blob");
    }
    let root = repo
        .canonicalize()
        .map_err(|e| format!("canonicalize Git repository: {e}"))?;
    let physical = root
        .join(path)
        .canonicalize()
        .map_err(|e| format!("selected C file is absent in checkout: {e}"))?;
    if !physical.starts_with(&root) {
        return refuse("C source resolves outside Git repository");
    }
    let source = fs::read(&physical).map_err(|e| format!("read C source: {e}"))?;
    if source.len() > MAX_SOURCE || digest::git_blob(&source) != blob {
        return refuse("checked-out C bytes do not match the pinned Git blob");
    }
    if !compile_db.is_file() {
        return refuse("existing compilation database required");
    }
    let config_json = match config {
        Some(file) => Value::string(
            file.canonicalize()
                .map_err(|e| format!("configuration file not readable: {e}"))?
                .to_string_lossy(),
        ),
        None => Value::Null,
    };
    let request = obj(&[
        ("repo", Value::string(root.to_string_lossy())),
        ("source_path", Value::string(path)),
        ("source_blob", Value::string(&blob)),
        (
            "compile_db",
            Value::string(
                compile_db
                    .canonicalize()
                    .map_err(|e| format!("compilation database not readable: {e}"))?
                    .to_string_lossy(),
            ),
        ),
        ("compiler", Value::string(compiler)),
        ("config", config_json),
        (
            "object_prefix",
            object_prefix.map_or(Value::Null, Value::string),
        ),
        ("trusted_local", Value::Bool(true)),
    ]);
    let observation = adapter::clang_ast(&request)?;
    if observation.get("status")?.str()? != "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
        || observation.get("source_path")?.str()? != path
        || observation.get("source_blob")?.str()? != blob
    {
        return refuse("Clang observation does not match pinned source identity");
    }
    if observation.get("symbols")?.array_ref()?.len() > 5000
        || observation.get("calls")?.array_ref()?.len() > 5000
        || observation.get("dependencies")?.array_ref()?.len() > 2048
    {
        return refuse("Clang observation exceeded output bounds");
    }
    let body = obj(&[
        ("schema", Value::string("excession.clang-tu.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("observation", observation),
        ("execution", Value::string("EXPLICIT_TRUSTED_LOCAL_CLANG")),
    ]);
    let mut result = body.entries()?.clone();
    result.insert("report_id".into(), Value::string(body.sha256()));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuse_implicit_execution() {
        let result = scan(
            Path::new("/missing"),
            "HEAD",
            "src/main.c",
            Path::new("compile_commands.json"),
            "clang",
            None,
            None,
            "false",
        );
        assert!(result.unwrap_err().contains("--trusted-local true"));
    }

    #[test]
    fn refuse_non_translation_unit_prior_to_io() {
        let result = scan(
            Path::new("/missing"),
            "HEAD",
            "src/missing.rs",
            Path::new("compile_commands.json"),
            "clang",
            None,
            None,
            "true",
        );
        assert!(result.unwrap_err().contains("C/C++ translation unit"));
    }
}
