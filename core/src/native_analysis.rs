//! Uniform Git-bound evidence surface for *existing* native producers.
//! Go reports compiler-selected source/configuration membership. Clang reports
//! one configured translation unit's AST, calls, and observed headers.
//! These are distinct predicates; neither proves a complete build graph.
use crate::json::{Value, obj};
use crate::{Result, compiler_clang, git, native_boundary, native_discovery, refuse};
use std::path::Path;

fn identity(repo: &Path, commit: &str, tree: &str, path: &str) -> Result<Value> {
    let entry = git::entry_at(repo, commit, path)?;
    if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
        return refuse(format!("native input is not a regular Git source: {path}"));
    }
    Ok(obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(path)),
        ("blob", Value::string(entry.oid)),
        ("mode", Value::string(entry.mode)),
    ]))
}

fn input_receipts(
    repo: &Path,
    commit: &str,
    tree: &str,
    declaration: &Value,
) -> Result<(Value, usize, usize)> {
    native_boundary::declaration_preflight(declaration)?;
    let mut receipts = Vec::new();
    let mut source_count = 0usize;
    let mut config_count = 0usize;
    for item in declaration.get("inputs")?.array_ref()? {
        let path = item.get("path")?.str()?;
        let role = item.get("role")?.str()?;
        let source = identity(repo, commit, tree, path)?;
        receipts.push(obj(&[("source", source), ("role", Value::string(role))]));
        match role {
            "source" => source_count += 1,
            "configuration" => config_count += 1,
            _ => return refuse("unrecognized native input role"),
        }
    }
    Ok((Value::array(receipts), source_count, config_count))
}

/// Build the same evidence envelope around two different native producer types.
/// Only the selected source and declared Go inputs have authenticated Git blobs.
pub fn scan(
    repo: &Path,
    revision: &str,
    source: &str,
    language: &str,
    compile_db: Option<&Path>,
    compiler: Option<&str>,
    trusted_local: &str,
) -> Result<Value> {
    if trusted_local != "true" {
        return refuse("native analysis requires explicit --trusted-local true");
    }
    if !matches!(language, "go" | "c" | "cpp") {
        return refuse(
            "native producer unavailable: supported languages are go, c, cpp; use analyze languages for syntax-only coverage",
        );
    }
    git::validate_path(source)?;
    let expected_ext = match language {
        "go" => source.ends_with(".go"),
        "c" => source.ends_with(".c"),
        "cpp" => [".cc", ".cpp", ".cxx", ".C"]
            .iter()
            .any(|ext| source.ends_with(ext)),
        _ => false,
    };
    if !expected_ext {
        return refuse("source extension does not match selected native language");
    }
    if language == "go" && (compile_db.is_some() || compiler.is_some()) {
        return refuse("Go does not use --compile-db or --compiler");
    }
    if language != "go" && compile_db.is_none() {
        return refuse("Clang requires --compile-db for a configured translation unit");
    }
    let root = repo
        .canonicalize()
        .map_err(|e| format!("resolve Git checkout: {e}"))?;
    git::verify_repo(&root)?;
    // Local compilers must not silently depend on uncommitted source files.
    git::clean(&root)?;
    let commit = git::rev(&root, revision)?;
    if git::text(&root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("native analysis requires a commit revision");
    }
    let tree = git::rev(&root, &format!("{commit}^{{tree}}"))?;
    let subject = identity(&root, &commit, &tree, source)?;
    let (producer, scope, inputs, source_count, config_count, observation) = if language == "go" {
        let result = native_discovery::discover(&root, &commit, source, "go", None, true)?;
        let declaration = result.get("declaration")?;
        if result.get("stats")?.get("commit")?.str()? != commit
            || declaration.get("subject")?.str()? != source
            || declaration.get("language")?.str()? != "go"
        {
            return refuse("Go native declaration is not pinned to selected source");
        }
        let (inputs, source_count, config_count) =
            input_receipts(&root, &commit, &tree, declaration)?;
        (
            "go-list",
            "COMPILER_SELECTED_INPUTS_ONLY",
            inputs,
            source_count,
            config_count,
            declaration.clone(),
        )
    } else {
        // Clang uses the actual checkout; historical Git objects cannot
        // authenticate the compiler's external database or header contents.
        if git::rev(&root, "HEAD")? != commit {
            return refuse("Clang native analysis requires selected clean HEAD");
        }
        let result = compiler_clang::scan(
            &root,
            &commit,
            source,
            compile_db.ok_or("Clang compilation database missing")?,
            compiler.unwrap_or(if language == "c" { "clang" } else { "clang++" }),
            compiler_clang::ClangOptions::default(),
            "true",
        )?;
        if result.get("source_commit")?.str()? != commit
            || result.get("source_tree")?.str()? != tree
            || result.get("observation")?.get("source_blob")?.str()?
                != subject.get("blob")?.str()?
            || result.get("observation")?.get("language")?.str()?
                != if language == "c" { "c" } else { "c++" }
        {
            return refuse("Clang native observation differs from selected Git source");
        }
        (
            "clang-ast",
            "CONFIGURED_TU_NOT_BUILD_VERIFIED",
            Value::array([obj(&[
                ("source", subject.clone()),
                ("role", Value::string("source")),
            ])]),
            1,
            0,
            result,
        )
    };
    let limitations = if language == "go" {
        Value::array([
            Value::string("Go input membership is not a symbol reference or call graph"),
            Value::string("CGO and go.work are unsupported; dependency resolution is offline"),
            Value::string("Native local tool execution is not remote attestation"),
        ])
    } else {
        Value::array([
            Value::string("Only one Clang translation unit and compilation configuration"),
            Value::string(
                "External compilation database and headers are not fully Git-authenticated",
            ),
            Value::string("Clang reparsing is not a verified build or call graph closure"),
        ])
    };
    obj(&[
        ("schema", Value::string("excession.native-analysis.v1")),
        ("status", Value::string("PARTIAL")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("source", subject),
        ("language", Value::string(language)),
        ("producer", Value::string(producer)),
        ("scope", Value::string(scope)),
        ("execution", Value::string("TRUSTED_LOCAL_NOT_ATTESTED")),
        ("input_sources", Value::number(source_count)),
        ("input_configurations", Value::number(config_count)),
        ("inputs", inputs),
        ("observation", observation),
        ("limitations", limitations),
    ])
    .stamp_id("report_id")
}

pub fn human(report: &Value) -> Result<String> {
    Ok(format!(
        "{} native analysis ({})\nGit commit: {}\nSource: {}\nGit-bound source inputs: {}\nGit-bound configuration inputs: {}\nCoverage: PARTIAL ({})\nTrusted-local execution is not build verification. Use --json for evidence.",
        report.get("language")?.str()?,
        report.get("producer")?.str()?,
        report.get("source_commit")?.str()?,
        report.get("source")?.get("path")?.str()?,
        report.get("input_sources")?.integer()?,
        report.get("input_configurations")?.integer()?,
        report.get("scope")?.str()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_untrusted_and_unsupported_before_invoking_toolchains() {
        let repo = Path::new("/does-not-exist");
        assert!(
            scan(repo, "HEAD", "a.go", "go", None, None, "false")
                .unwrap_err()
                .contains("--trusted-local")
        );
        assert!(
            scan(repo, "HEAD", "a.java", "java", None, None, "true")
                .unwrap_err()
                .contains("native producer unavailable")
        );
        assert!(
            scan(repo, "HEAD", "a.go", "go", None, Some("clang"), "true")
                .unwrap_err()
                .contains("does not use")
        );
        assert!(
            scan(repo, "HEAD", "a.cpp", "cpp", None, None, "true")
                .unwrap_err()
                .contains("--compile-db")
        );
        assert!(
            scan(repo, "HEAD", "a.c", "cpp", None, None, "true")
                .unwrap_err()
                .contains("source extension")
        );
    }
}
