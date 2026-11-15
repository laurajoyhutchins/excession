//! Unified source-bound *syntax* analysis for any grammar supported by the
//! installed Tree-sitter language pack. It does not manufacture compiler-level
//! dependencies for languages without a conservative resolver.
use crate::json::{Value, obj};
use crate::{Result, syntax, syntax_resolution};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default)]
struct LanguageStats {
    sources: usize,
    parsed: usize,
    parsed_with_errors: usize,
    unavailable: usize,
    candidate_edges: usize,
    unresolved_imports: usize,
}

fn conservative_resolver(language: &str) -> bool {
    matches!(
        language,
        "python" | "javascript" | "typescript" | "tsx" | "go"
    )
}

fn assemble(syntax_report: &Value, dependencies: &Value) -> Result<Value> {
    let mut by_language = BTreeMap::<String, LanguageStats>::new();
    let mut total_parsed = 0usize;
    for entry in syntax_report.get("sources")?.array_ref()? {
        let language = entry.get("language")?.str()?;
        let language = if language.is_empty() {
            "unknown"
        } else {
            language
        };
        let stats = by_language.entry(language.to_owned()).or_default();
        stats.sources += 1;
        match entry.get("status")?.str()? {
            "PARSED" => {
                stats.parsed += 1;
                total_parsed += 1;
            }
            "PARSED_WITH_ERRORS" => {
                stats.parsed_with_errors += 1;
                total_parsed += 1;
            }
            _ => stats.unavailable += 1,
        }
    }
    for candidate in dependencies.get("candidate_edges")?.array_ref()? {
        let language = candidate.get("language")?.str()?;
        by_language
            .entry(language.to_owned())
            .or_default()
            .candidate_edges += 1;
    }
    for unresolved in dependencies.get("unresolved_imports")?.array_ref()? {
        let language = unresolved.get("language")?.str()?;
        by_language
            .entry(language.to_owned())
            .or_default()
            .unresolved_imports += 1;
    }
    let languages = by_language
        .into_iter()
        .map(|(language, stats)| {
            let scope = if conservative_resolver(&language) {
                "CONSERVATIVE_IMPORT_CANDIDATES"
            } else {
                "SYNTAX_ONLY_NO_DEPENDENCY_RESOLVER"
            };
            obj(&[
                ("language", Value::string(language)),
                ("scanned_sources", Value::number(stats.sources)),
                ("parsed_sources", Value::number(stats.parsed)),
                (
                    "parsed_with_errors",
                    Value::number(stats.parsed_with_errors),
                ),
                ("unavailable_sources", Value::number(stats.unavailable)),
                ("candidate_edges", Value::number(stats.candidate_edges)),
                (
                    "unresolved_imports",
                    Value::number(stats.unresolved_imports),
                ),
                ("resolution_scope", Value::string(scope)),
            ])
        })
        .collect::<Vec<_>>();
    let payload = obj(&[
        ("schema", Value::string("excession.language-analysis.v1")),
        ("status", Value::string("PARTIAL")),
        ("source_commit", syntax_report.get("source_commit")?.clone()),
        ("source_tree", syntax_report.get("source_tree")?.clone()),
        ("roots", syntax_report.get("roots")?.clone()),
        ("parser", syntax_report.get("parser")?.clone()),
        (
            "parser_version",
            syntax_report.get("parser_version")?.clone(),
        ),
        ("syntax_report_id", syntax_report.get("report_id")?.clone()),
        ("resolution_id", dependencies.get("resolution_id")?.clone()),
        (
            "scanned_sources",
            syntax_report.get("source_count")?.clone(),
        ),
        ("parsed_sources", Value::number(total_parsed)),
        ("languages", Value::array(languages)),
        (
            "candidate_edges",
            dependencies.get("candidate_edges")?.clone(),
        ),
        (
            "unresolved_imports",
            dependencies.get("unresolved_imports")?.clone(),
        ),
        (
            "skipped_sources",
            dependencies.get("skipped_sources")?.clone(),
        ),
        ("metadata", dependencies.get("metadata")?.clone()),
    ]);
    payload.stamp_id("report_id")
}

/// Parse pinned Git contents using installed grammars, then apply only the
/// language-specific candidate resolvers that actually exist. No source is run.
pub fn run(repo: &Path, revision: &str, roots: &[&str]) -> Result<Value> {
    let scanned = syntax::scan(repo, revision, roots)?;
    let resolved = syntax_resolution::candidates_from_scanned(repo, &scanned, |_| Ok(()))?;
    assemble(&scanned, &resolved)
}

pub fn human(report: &Value) -> Result<String> {
    let mut lines = vec![
        format!("Language analysis: {}", report.get("source_commit")?.str()?),
        format!(
            "Parsed {}/{} tracked source files; {} candidate edges, {} unresolved imports.",
            report.get("parsed_sources")?.integer()?,
            report.get("scanned_sources")?.integer()?,
            report.get("candidate_edges")?.array_ref()?.len(),
            report.get("unresolved_imports")?.array_ref()?.len(),
        ),
    ];
    for item in report.get("languages")?.array_ref()? {
        lines.push(format!(
            "  {}: {}/{} parsed, {} unavailable, {} candidates, {} unresolved ({})",
            item.get("language")?.str()?,
            item.get("parsed_sources")?.integer()? + item.get("parsed_with_errors")?.integer()?,
            item.get("scanned_sources")?.integer()?,
            item.get("unavailable_sources")?.integer()?,
            item.get("candidate_edges")?.integer()?,
            item.get("unresolved_imports")?.integer()?,
            item.get("resolution_scope")?.str()?,
        ));
    }
    lines.push("Coverage: PARTIAL. Syntax and candidate imports do not establish build or runtime behavior.".to_owned());
    lines.push("Use --json for source-bound candidates and per-file gaps.".to_owned());
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_receives_explicit_coverage_without_inventing_resolution() {
        let source = |path: &str, language: &str, status: &str| {
            obj(&[
                ("source_path", Value::string(path)),
                ("language", Value::string(language)),
                ("status", Value::string(status)),
            ])
        };
        let scanned = obj(&[
            ("source_commit", Value::string("a".repeat(40))),
            ("source_tree", Value::string("b".repeat(40))),
            ("roots", Value::array([Value::string(".")])),
            ("parser", Value::string("tree-sitter-language-pack")),
            ("parser_version", Value::string("test")),
            ("report_id", Value::string("source-id")),
            ("source_count", Value::number(5)),
            (
                "sources",
                Value::array([
                    source("src/a.py", "python", "PARSED"),
                    source("web/b.tsx", "tsx", "PARSED"),
                    source("server/main.go", "go", "GRAMMAR_NOT_INSTALLED"),
                    source("src/lib.rs", "rust", "PARSED"),
                    source("native/a.cpp", "cpp", "PARSED_WITH_ERRORS"),
                ]),
            ),
        ]);
        let edge = obj(&[("language", Value::string("tsx"))]);
        let unresolved = obj(&[("language", Value::string("python"))]);
        let resolved = obj(&[
            ("resolution_id", Value::string("resolved-id")),
            ("candidate_edges", Value::array([edge])),
            ("unresolved_imports", Value::array([unresolved])),
            ("skipped_sources", Value::array([])),
            ("metadata", Value::array([])),
        ]);
        let report = assemble(&scanned, &resolved).unwrap();
        let langs = report.get("languages").unwrap().array_ref().unwrap();
        assert_eq!(langs.len(), 5);
        let rust = langs
            .iter()
            .find(|x| x.get("language").unwrap().str().unwrap() == "rust")
            .unwrap();
        assert_eq!(
            rust.get("resolution_scope").unwrap().str().unwrap(),
            "SYNTAX_ONLY_NO_DEPENDENCY_RESOLVER"
        );
        assert_eq!(rust.get("candidate_edges").unwrap().integer().unwrap(), 0);
        let go = langs
            .iter()
            .find(|x| x.get("language").unwrap().str().unwrap() == "go")
            .unwrap();
        assert_eq!(go.get("unavailable_sources").unwrap().integer().unwrap(), 1);
        assert_eq!(report.get("parsed_sources").unwrap().integer().unwrap(), 4);
    }
}
