//! Configuration-specific C observations produced by the real Clang parser.
//! The compile command and local build artifacts are trusted-local inputs,
//! distinct from the authenticated Git source and static syntax relations.
use crate::json::{Value, obj};
use crate::{Result, adapter, digest, git, refuse};
use std::fs;
use std::path::Path;

const MAX_SOURCE: usize = 1024 * 1024;

/// Reparse one source file with an existing compilation database command.
///
/// The compilation database is normally generated from Linux's .cmd files by
/// scripts/clang-tools/gen_compile_commands.py, after building a configuration.
/// This reparse does not establish that the kernel built or ran successfully.
pub fn scan(
    repo: &Path,
    revision: &str,
    path: &str,
    compile_db: &Path,
    compiler: &str,
    config: Option<&Path>,
    trusted_local: &str,
) -> Result<Value> {
    if trusted_local != "true" {
        return refuse("Clang requires --trusted-local true; compilation flags are local code");
    }
    git::validate_path(path)?;
    if !path.ends_with(".c") {
        return refuse("Clang source must be a C translation unit");
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
            "false",
        );
        assert!(result.unwrap_err().contains("--trusted-local true"));
    }

    #[test]
    fn refuse_non_c_source_prior_to_io() {
        let result = scan(
            Path::new("/missing"),
            "HEAD",
            "src/missing.cc",
            Path::new("compile_commands.json"),
            "clang",
            None,
            "true",
        );
        assert!(result.unwrap_err().contains("C translation unit"));
    }
}
