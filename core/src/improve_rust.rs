//! Rust-specific edit producer for the language-neutral improvement coordinator.
//! This is an opt-in native action. SCIP observations alone never certify edits.
use super::read_plan;
use crate::improve_generic::{plan_from_report, report_ids as ids};
use crate::json::{self, Value, obj};
use crate::{Result, refuse, scip_refactor};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// A constrained native extraction trial. A preview is source-bound and
/// read-only; application can only create a new detached worktree.
pub struct Trial<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub index: &'a Path,
    pub report: &'a Path,
    pub plan: &'a Path,
    pub candidate_id: &'a str,
    pub symbol: Option<&'a str>,
    pub ast_grep: &'a Path,
    pub source_prefix: Option<&'a str>,
    pub output: Option<&'a Path>,
    pub trusted_local: &'a str,
}

fn simple_rust_identifier(name: &str) -> bool {
    name.len() <= 64
        && name
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Use SCIP-reported function references solely to propose candidates. The
/// existing ast-grep recipe performs the actual syntax and byte-range checks.
pub(super) fn extraction_targets(
    report: &Value,
    candidate_id: &str,
    selected_symbol: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    if report.get("indexer")?.get("name")?.str()? != "rust-analyzer" {
        return refuse("automatic extraction requires rust-analyzer SCIP observations");
    }
    if !ids(report, "review_candidate_ids")?.contains(candidate_id) {
        return refuse("automatic extraction requires an advisory review candidate");
    }
    let candidate = report
        .get("candidates")?
        .array_ref()?
        .iter()
        .find(|item| item.get("candidate_id").and_then(Value::str).ok() == Some(candidate_id))
        .ok_or("unknown native refactor candidate")?;
    if candidate.get("kind")?.str()? != "reciprocal_symbol_references" {
        return refuse("only reciprocal symbol-reference candidates support this trial");
    }
    if let Some(name) = selected_symbol
        && !simple_rust_identifier(name)
    {
        return refuse("requested extraction symbol is not a plain Rust name");
    }
    let mut found = BTreeSet::new();
    for reference in candidate.get("references")?.array_ref()? {
        let symbol = reference.get("symbol")?.str()?;
        let Some(name) = symbol
            .rsplit('/')
            .next()
            .and_then(|s| s.strip_suffix("()."))
        else {
            continue;
        };
        if !simple_rust_identifier(name) || selected_symbol.is_some_and(|s| s != name) {
            continue;
        }
        let to = reference.get("to")?;
        if !to.get("language")?.str()?.eq_ignore_ascii_case("rust") {
            continue;
        }
        let path = to.get("path")?.str()?;
        let Some((directory, filename)) = path.rsplit_once('/') else {
            continue;
        };
        if directory != "src" && !directory.ends_with("/src") {
            continue;
        }
        let Some(stem) = filename.strip_suffix(".rs") else {
            continue;
        };
        if !simple_rust_identifier(stem) || matches!(stem, "lib" | "main" | "mod") {
            continue;
        }
        let destination_name = format!("{stem}_{name}_extracted");
        if destination_name.len() > 100 {
            continue;
        }
        found.insert((
            path.to_owned(),
            format!("{directory}/{destination_name}.rs"),
            name.to_owned(),
        ));
    }
    if found.len() > 8 {
        return refuse("automatic extraction has more than eight possible function targets");
    }
    Ok(found.into_iter().collect())
}

pub fn trial(request: &Trial<'_>) -> Result<Value> {
    if request.trusted_local != "true" {
        return refuse("improvement trial executes ast-grep; require --trusted-local true");
    }
    let audited = scip_refactor::audit_with_prefix(
        request.repo,
        request.revision,
        request.index,
        request.report,
        request.source_prefix,
    )?;
    let report_bytes =
        fs::read(request.report).map_err(|e| format!("read native refactor report: {e}"))?;
    if report_bytes.len() > 16 * 1024 * 1024 {
        return refuse("native refactor report exceeds 16 MiB");
    }
    let report = json::parse(&report_bytes)?;
    if report.get("report_id")? != audited.get("report_id")?
        || !report.content_id_matches("report_id")?
    {
        return refuse("native refactor report changed since its source-bound audit");
    }
    let original_plan = read_plan(request.plan)?;
    if plan_from_report(&report)? != original_plan
        || !original_plan.content_id_matches("plan_id")?
    {
        return refuse("improvement trial plan is stale or altered");
    }
    let targets = extraction_targets(&report, request.candidate_id, request.symbol)?;
    let mut possible = Vec::new();
    let mut rejected = Vec::new();
    for (source, destination, symbol) in targets {
        let native_request = crate::refactor_codegen::Request {
            repo: request.repo,
            revision: request.revision,
            index: request.index,
            report: request.report,
            candidate: request.candidate_id,
            source: &source,
            destination: &destination,
            symbols: &symbol,
            ast_grep: request.ast_grep,
            source_prefix: request.source_prefix,
            output: None,
        };
        match crate::refactor_codegen::extract_shared_module(&native_request) {
            Ok(preview) => possible.push((source, destination, symbol, preview)),
            Err(reason) => rejected.push(obj(&[
                ("source", Value::string(source)),
                ("symbol", Value::string(symbol)),
                ("reason", Value::string(reason)),
            ])),
        }
    }
    if possible.len() != 1 {
        return obj(&[
            ("schema", Value::string("excession.improvement-trial.v1")),
            (
                "status",
                Value::string(if possible.is_empty() {
                    "NO_SUPPORTED_NATIVE_TRANSFORMATION"
                } else {
                    "AMBIGUOUS_NATIVE_TRANSFORMATIONS"
                }),
            ),
            ("candidate_id", Value::string(request.candidate_id)),
            ("plan_id", original_plan.get("plan_id")?.clone()),
            ("supported_preview_count", Value::number(possible.len())),
            ("refusals", Value::array(rejected)),
            ("ready_to_merge", Value::Bool(false)),
        ])
        .stamp_id("trial_id");
    }
    let (source, destination, symbol, preview) = possible.remove(0);
    let outcome = if let Some(output) = request.output {
        let applied =
            crate::refactor_codegen::extract_shared_module(&crate::refactor_codegen::Request {
                repo: request.repo,
                revision: request.revision,
                index: request.index,
                report: request.report,
                candidate: request.candidate_id,
                source: &source,
                destination: &destination,
                symbols: &symbol,
                ast_grep: request.ast_grep,
                source_prefix: request.source_prefix,
                output: Some(output),
            })?;
        if applied.get("plan_id")? != preview.get("plan_id")? {
            return refuse("native extraction preview and application plan IDs differ");
        }
        applied
    } else {
        preview
    };
    obj(&[
        ("schema", Value::string("excession.improvement-trial.v1")),
        ("status", outcome.get("status")?.clone()),
        ("candidate_id", Value::string(request.candidate_id)),
        ("plan_id", original_plan.get("plan_id")?.clone()),
        ("source", Value::string(source)),
        ("destination", Value::string(destination)),
        ("symbol", Value::string(symbol)),
        ("native_result", outcome),
        ("refusals", Value::array(rejected)),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ])
    .stamp_id("trial_id")
}

