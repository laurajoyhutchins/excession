//! Improvement orchestration over native, source-bound evidence producers.
//! The normalized candidate and evaluation model is language-neutral.
use crate::json::{self, Value, obj};
use crate::improve_generic::{evaluate_reports, plan_from_report, report_ids as ids, MAX_PLAN_BYTES};
use crate::sqlite::Db;
use crate::{Result, change_inventory, git, refactor, refuse, scip_refactor};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub fn plan(repo: &Path, revision: &str, index: &Path, prefix: Option<&str>) -> Result<Value> {
    let report = scip_refactor::suggest_with_prefix(repo, revision, index, prefix)?;
    plan_from_report(&report)
}

pub struct Evaluation<'a> {
    pub base_repo: &'a Path,
    pub candidate_repo: &'a Path,
    pub base: &'a str,
    pub candidate: &'a str,
    pub base_index: &'a Path,
    pub candidate_index: &'a Path,
    pub plan: &'a Path,
    pub source_prefix: Option<&'a str>,
}

/// Plan from Excession's existing indexed Python/JavaScript refactor producer.
/// The producer authenticates the revision through the Git-backed SQLite index.
pub fn plan_indexed(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let report = refactor::suggest(db, name, revision)?;
    plan_from_report(&report)
}

fn read_plan(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("read improvement plan: {e}"))?;
    if bytes.len() > MAX_PLAN_BYTES {
        return refuse("improvement plan exceeds 4 MiB");
    }
    json::parse(&bytes)
}

pub fn evaluate(request: &Evaluation<'_>) -> Result<Value> {
    let base_commit = git::rev(request.base_repo, request.base)?;
    let candidate_commit = git::rev(request.candidate_repo, request.candidate)?;
    if base_commit == candidate_commit
        || git::rev(request.candidate_repo, &base_commit)? != base_commit
        || git::rev(request.base_repo, &candidate_commit)? != candidate_commit
    {
        return refuse("evaluation requires two different commits available in both Git checkouts");
    }
    let saved = read_plan(request.plan)?;
    let before = scip_refactor::suggest_with_prefix(
        request.base_repo,
        &base_commit,
        request.base_index,
        request.source_prefix,
    )?;
    let after = scip_refactor::suggest_with_prefix(
        request.candidate_repo,
        &candidate_commit,
        request.candidate_index,
        request.source_prefix,
    )?;
    let inventory =
        change_inventory::inventory(request.candidate_repo, &base_commit, &candidate_commit)?;
    evaluate_reports(&before, &after, &inventory, &saved)
}

/// Evaluate indexed Python/JavaScript evidence on two pinned commits.
/// The same normalized comparison is used by the SCIP producer. No edits.
pub fn evaluate_indexed(
    repo: &Path,
    db: &Db,
    name: &str,
    base: &str,
    candidate: &str,
    plan: &Path,
) -> Result<Value> {
    let inventory = change_inventory::inventory(repo, base, candidate)?;
    let base_commit = inventory.get("base_commit")?.str()?;
    let candidate_commit = inventory.get("head_commit")?.str()?;
    if base_commit == candidate_commit {
        return refuse("improvement evaluation requires two distinct commits");
    }
    let before = refactor::suggest(db, name, base_commit)?;
    let after = refactor::suggest(db, name, candidate_commit)?;
    let saved = read_plan(plan)?;
    evaluate_reports(&before, &after, &inventory, &saved)
}

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
fn extraction_targets(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn report(commit: &str, tree: &str, pair: bool, documents: i64, unresolved: i64) -> Value {
        let candidate = obj(&[
            ("candidate_id", Value::string("candidate-a")),
            ("kind", Value::string("reciprocal_symbol_references")),
            (
                "sources",
                Value::array([
                    obj(&[("path", Value::string("src/a.rs"))]),
                    obj(&[("path", Value::string("src/b.rs"))]),
                ]),
            ),
        ]);
        obj(&[
            ("schema", Value::string("excession.scip-refactor.v1")),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("report_id", Value::string("native-id")),
            (
                "coverage",
                obj(&[
                    ("indexed_documents", Value::number(documents)),
                    ("unresolved_references", Value::number(unresolved)),
                ]),
            ),
            (
                "candidates",
                Value::array(if pair { vec![candidate] } else { vec![] }),
            ),
            (
                "triage",
                obj(&[
                    (
                        "review_candidate_ids",
                        Value::array(if pair {
                            vec![Value::string("candidate-a")]
                        } else {
                            vec![]
                        }),
                    ),
                    ("likely_structural_module_candidate_ids", Value::array([])),
                ]),
            ),
        ])
    }

    fn inventory(changed: bool) -> Value {
        obj(&[
            ("base_commit", Value::string("base")),
            ("base_tree", Value::string("bt")),
            ("head_commit", Value::string("head")),
            ("head_tree", Value::string("ht")),
            ("report_id", Value::string("git-id")),
            (
                "changes",
                Value::array(if changed {
                    vec![obj(&[
                        ("kind", Value::string("MODIFIED")),
                        ("before", obj(&[("path", Value::string("src/a.rs"))])),
                        ("after", obj(&[("path", Value::string("src/a.rs"))])),
                    ])]
                } else {
                    vec![]
                }),
            ),
        ])
    }

    #[test]
    fn native_trial_targets_are_bounded_and_source_specific() {
        let mut native = report("base", "bt", true, 3, 0);
        native.entries_mut().unwrap().insert(
            "indexer".into(),
            obj(&[("name", Value::string("rust-analyzer"))]),
        );
        let references = Value::array([
            obj(&[
                (
                    "symbol",
                    Value::string("scip rust-analyzer pkg/selected()."),
                ),
                (
                    "to",
                    obj(&[
                        ("path", Value::string("core/src/a.rs")),
                        ("language", Value::string("rust")),
                    ]),
                ),
            ]),
            obj(&[
                ("symbol", Value::string("scip rust-analyzer pkg/ignored().")),
                (
                    "to",
                    obj(&[
                        ("path", Value::string("core/src/lib.rs")),
                        ("language", Value::string("rust")),
                    ]),
                ),
            ]),
            obj(&[
                ("symbol", Value::string("scip rust-analyzer pkg/helper().")),
                (
                    "to",
                    obj(&[
                        ("path", Value::string("core/src/a.rs")),
                        ("language", Value::string("python")),
                    ]),
                ),
            ]),
        ]);
        let mut candidate = native.get("candidates").unwrap().array_ref().unwrap()[0].clone();
        candidate
            .entries_mut()
            .unwrap()
            .insert("references".into(), references);
        native
            .entries_mut()
            .unwrap()
            .insert("candidates".into(), Value::array([candidate]));
        let targets = extraction_targets(&native, "candidate-a", None).unwrap();
        assert_eq!(
            targets,
            vec![(
                "core/src/a.rs".to_owned(),
                "core/src/a_selected_extracted.rs".to_owned(),
                "selected".to_owned(),
            )]
        );
        assert_eq!(
            extraction_targets(&native, "candidate-a", Some("selected")).unwrap(),
            targets
        );
        assert!(extraction_targets(&native, "candidate-a", Some("../escape")).is_err());
        assert!(extraction_targets(&native, "forged-id", None).is_err());
    }

    #[test]
    fn structural_delta_is_observation_not_merge_authorization() {
        let before = report("base", "bt", true, 3, 0);
        let after = report("head", "ht", false, 3, 0);
        let plan = plan_from_report(&before).unwrap();
        let result = evaluate_reports(&before, &after, &inventory(true), &plan).unwrap();
        assert_eq!(
            result.get("status").unwrap().str().unwrap(),
            "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED"
        );
        assert!(!result.get("ready_to_merge").unwrap().bool().unwrap());
        assert!(
            !result
                .get("automatic_deletion_authorized")
                .unwrap()
                .bool()
                .unwrap()
        );
        assert_eq!(
            result
                .get("reciprocal_pairs_before")
                .unwrap()
                .integer()
                .unwrap(),
            1
        );
    }

    #[test]
    fn moving_a_function_to_a_new_module_is_not_debt_removal() {
        let before = report("base", "bt", true, 3, 0);
        let after = report("head", "ht", false, 4, 0);
        let plan = plan_from_report(&before).unwrap();
        let mut changes = inventory(true);
        let mut items = changes
            .get("changes")
            .unwrap()
            .array_ref()
            .unwrap()
            .to_vec();
        items.push(obj(&[
            ("kind", Value::string("ADDED")),
            ("before", Value::Null),
            (
                "after",
                obj(&[("path", Value::string("src/a_selected_extracted.rs"))]),
            ),
        ]));
        changes
            .entries_mut()
            .unwrap()
            .insert("changes".into(), Value::array(items));
        let result = evaluate_reports(&before, &after, &changes, &plan).unwrap();
        assert_eq!(
            result.get("status").unwrap().str().unwrap(),
            "STRUCTURAL_RELOCATION_OBSERVED_UNVERIFIED"
        );
        assert_eq!(
            result
                .get("introduced_source_paths")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            1
        );
        assert!(!result.get("ready_to_merge").unwrap().bool().unwrap());
    }

    #[test]
    fn coverage_loss_or_unrelated_disappearance_is_inconclusive() {
        let before = report("base", "bt", true, 3, 0);
        let plan = plan_from_report(&before).unwrap();
        for (documents, unresolved, changed) in [(2, 0, true), (3, 1, true), (3, 0, false)] {
            let after = report("head", "ht", false, documents, unresolved);
            let result = evaluate_reports(&before, &after, &inventory(changed), &plan).unwrap();
            assert_eq!(
                result.get("status").unwrap().str().unwrap(),
                "INCONCLUSIVE_EVIDENCE_CHANGE"
            );
        }
    }

    #[test]
    fn mismatched_plan_and_revisions_refuse() {
        let before = report("base", "bt", true, 3, 0);
        let after = report("head", "ht", false, 3, 0);
        let mut plan = plan_from_report(&before).unwrap();
        plan.entries_mut()
            .unwrap()
            .insert("candidate_count".into(), Value::number(100));
        assert!(evaluate_reports(&before, &after, &inventory(true), &plan).is_err());
        let good_plan = plan_from_report(&before).unwrap();
        let wrong_head = report("other", "ht", false, 3, 0);
        assert!(evaluate_reports(&before, &wrong_head, &inventory(true), &good_plan).is_err());
    }

    #[test]
    fn triage_mismatch_and_duplicate_native_pairs_refuse() {
        let mut broken = report("base", "bt", true, 3, 0);
        broken.get("triage").unwrap();
        broken.entries_mut().unwrap().insert(
            "triage".into(),
            obj(&[
                ("review_candidate_ids", Value::array([])),
                ("likely_structural_module_candidate_ids", Value::array([])),
            ]),
        );
        assert!(plan_from_report(&broken).is_err());
        let original = report("base", "bt", true, 3, 0);
        let mut duplicate = original.clone();
        let candidate = duplicate.get("candidates").unwrap().array_ref().unwrap()[0].clone();
        duplicate.entries_mut().unwrap().insert(
            "candidates".into(),
            Value::array([candidate.clone(), candidate]),
        );
        assert!(crate::improve_generic::findings(&duplicate).is_err());
    }
}
