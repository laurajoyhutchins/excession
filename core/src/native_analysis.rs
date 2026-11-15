//! Uniform Git-bound evidence surface for *existing* native producers.
//! Go reports compiler-selected source/configuration membership. Clang reports
//! one configured translation unit's AST, calls, and observed headers.
//! These are distinct predicates; neither proves a complete build graph.
use crate::json::{Value, obj};
use crate::{Result, compiler_clang, git, native_boundary, native_discovery, refuse, rust_native};
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

#[derive(Default, Clone, Copy)]
pub struct NativeOptions<'a> {
    pub compile_db: Option<&'a Path>,
    pub compiler: Option<&'a str>,
    pub source_prefix: Option<&'a str>,
    pub classpath_manifest: Option<&'a Path>,
    pub rust_analyzer: Option<&'a str>,
    pub scip: Option<&'a str>,
}

/// Build the same evidence envelope around supported native producer types.
/// Only the selected source and declared Go inputs have authenticated Git blobs.
pub fn scan(
    repo: &Path,
    revision: &str,
    source: &str,
    language: &str,
    options: NativeOptions<'_>,
    trusted_local: &str,
) -> Result<Value> {
    let NativeOptions {
        compile_db,
        compiler,
        source_prefix,
        classpath_manifest,
        rust_analyzer,
        scip,
    } = options;
    if trusted_local != "true" {
        return refuse("native analysis requires explicit --trusted-local true");
    }
    if !matches!(language, "go" | "c" | "cpp" | "java" | "rust") {
        return refuse(
            "native producer unavailable: supported languages are go, c, cpp, java, rust; use analyze languages for syntax-only coverage",
        );
    }
    git::validate_path(source)?;
    let expected_ext = match language {
        "go" => source.ends_with(".go"),
        "rust" => source.ends_with(".rs"),
        "c" => source.ends_with(".c"),
        "java" => source.ends_with(".java"),
        "cpp" => [".cc", ".cpp", ".cxx", ".C"]
            .iter()
            .any(|ext| source.ends_with(ext)),
        _ => false,
    };
    if !expected_ext {
        return refuse("source extension does not match selected native language");
    }
    if language == "rust" && (compile_db.is_some() || compiler.is_some() || classpath_manifest.is_some()) {
        return refuse("Rust SCIP does not use --compile-db, --compiler or --classpath-manifest");
    }
    if language != "rust" && (rust_analyzer.is_some() || scip.is_some()) {
        return refuse("--rust-analyzer and --scip are only supported by Rust native analysis");
    }
    if language == "java" && (compile_db.is_some() || compiler.is_some()) {
        return refuse("Java uses the installed JDK, not --compile-db or --compiler");
    }
    if language != "java" && classpath_manifest.is_some() {
        return refuse("--classpath-manifest is only supported by Java native analysis");
    }
    if language != "java" && source_prefix.is_some() {
        return refuse("--source-prefix currently applies only to Java native analysis");
    }
    if language == "go" && (compile_db.is_some() || compiler.is_some()) {
        return refuse("Go does not use --compile-db or --compiler");
    }
    if matches!(language, "c" | "cpp") && compile_db.is_none() {
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
    if language == "rust" {
        if let Some(prefix) = source_prefix {
            git::validate_path(prefix)?;
            if !source.starts_with(&format!("{prefix}/")) {
                return refuse("selected Rust source is outside --source-prefix");
            }
        }
        let native = rust_native::scan(
            &root,
            &commit,
            source_prefix,
            rust_analyzer.unwrap_or("rust-analyzer"),
            scip.unwrap_or("scip"),
            "true",
        )?;
        if native.get("schema")?.str()? != "excession.scip-observations.v1"
            || !native.content_id_matches("report_id")?
            || native.get("source_commit")?.str()? != commit
            || native.get("source_tree")?.str()? != tree
            || native.get("indexer")?.get("name")?.str()? != "rust-analyzer"
        {
            return refuse("Rust SCIP observations do not match selected Git revision and producer");
        }
        return obj(&[
            ("schema", Value::string("excession.native-analysis.v1")),
            ("status", Value::string("PARTIAL")),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("source", subject.clone()),
            ("language", Value::string("rust")),
            ("producer", Value::string("rust-analyzer-scip")),
            ("scope", Value::string("SCIP_WORKSPACE_REFERENCES_NOT_BUILD_VERIFIED")),
            ("execution", Value::string("TRUSTED_LOCAL_NOT_ATTESTED")),
            ("input_sources", Value::number(1)),
            ("input_configurations", Value::number(0)),
            ("inputs", Value::array([obj(&[
                ("source", subject),
                ("role", Value::string("source")),
            ])])),
            ("observation", native),
            ("limitations", Value::array([
                Value::string("SCIP is a workspace index, not a rustc build or typecheck receipt"),
                Value::string("The requested Git source is not proof that the indexer covered that file"),
                Value::string("Cross-file symbol references are not necessarily imports or calls"),
                Value::string("Absent references are not proof of unreachable or unused code"),
                Value::string("Trusted-local indexer execution is not independent attestation"),
            ])),
        ])
        .stamp_id("report_id");
    }
    if language == "java" {
        return crate::java_native::scan(
            &root,
            &commit,
            source,
            source_prefix.unwrap_or("."),
            classpath_manifest,
        );
    }
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
            scan(
                repo,
                "HEAD",
                "a.go",
                "go",
                NativeOptions::default(),
                "false"
            )
            .unwrap_err()
            .contains("--trusted-local")
        );
        assert!(
            scan(
                repo,
                "HEAD",
                "a.java",
                "java",
                NativeOptions {
                    compile_db: Some(Path::new("compile.json")),
                    ..NativeOptions::default()
                },
                "true"
            )
            .unwrap_err()
            .contains("Java uses the installed JDK")
        );
        assert!(
            scan(
                repo,
                "HEAD",
                "a.go",
                "go",
                NativeOptions {
                    compiler: Some("clang"),
                    ..NativeOptions::default()
                },
                "true"
            )
            .unwrap_err()
            .contains("does not use")
        );
        assert!(
            scan(
                repo,
                "HEAD",
                "a.cpp",
                "cpp",
                NativeOptions::default(),
                "true"
            )
            .unwrap_err()
            .contains("--compile-db")
        );
        assert!(
            scan(repo, "HEAD", "a.c", "cpp", NativeOptions::default(), "true")
                .unwrap_err()
                .contains("source extension")
        );
    }
}
