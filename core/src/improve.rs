//! Bounded improvement planning and measured structural deltas.
//! These are Git-bound observations, never proofs of dead code or safe edits.
use crate::json::{self, Value, obj};
use crate::{Result, change_inventory, git, refuse, scip_refactor};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const MAX_PLAN_BYTES: usize = 4 * 1024 * 1024;
type Pair = (String, String);

fn ids(report: &Value, field: &str) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for id in report.get("triage")?.get(field)?.array_ref()? {
        if !result.insert(id.str()?.to_owned()) {
            return refuse("duplicate native triage candidate ID");
        }
    }
    Ok(result)
}

fn pairs(report: &Value) -> Result<BTreeSet<Pair>> {
    if report.get("schema")?.str()? != "excession.scip-refactor.v1" {
        return refuse("improvement requires the native SCIP refactor report");
    }
    let mut result = BTreeSet::new();
    for candidate in report.get("candidates")?.array_ref()? {
        if candidate.get("kind")?.str()? != "reciprocal_symbol_references" {
            continue;
        }
        let sources = candidate.get("sources")?.array_ref()?;
        if sources.len() != 2 {
            return refuse("reciprocal candidate must have two sources");
        }
        let left = sources[0].get("path")?.str()?;
        let right = sources[1].get("path")?.str()?;
        if left == right {
            return refuse("reciprocal candidate contains the same source twice");
        }
        let pair = if left < right {
            (left.to_owned(), right.to_owned())
        } else {
            (right.to_owned(), left.to_owned())
        };
        if !result.insert(pair) {
            return refuse("duplicate reciprocal source pair in SCIP report");
        }
    }
    Ok(result)
}

fn coverage_count(report: &Value, field: &str) -> Result<i64> {
    let value = report.get("coverage")?.get(field)?.integer()?;
    if value < 0 {
        return refuse("negative SCIP coverage count");
    }
    Ok(value)
}

/// Rank real native observations without inventing semantic confidence.
/// Candidate IDs and source evidence are preserved verbatim.
fn plan_from_report(report: &Value) -> Result<Value> {
    let review = ids(report, "review_candidate_ids")?;
    let structural = ids(report, "likely_structural_module_candidate_ids")?;
    if !review.is_disjoint(&structural) {
        return refuse("native triage partitions overlap");
    }
    let mut found = BTreeSet::new();
    let mut proposals = Vec::new();
    for candidate in report.get("candidates")?.array_ref()? {
        let id = candidate.get("candidate_id")?.str()?;
        if !found.insert(id.to_owned()) {
            return refuse("duplicate native candidate identity");
        }
        if !review.contains(id) && !structural.contains(id) {
            return refuse("native candidate missing from triage");
        }
        let priority = if review.contains(id) {
            "REVIEW_NATIVE_REFERENCES"
        } else {
            "LIKELY_MODULE_DECLARATION"
        };
        proposals.push(obj(&[
            ("candidate_id", Value::string(id)),
            ("kind", candidate.get("kind")?.clone()),
            ("priority", Value::string(priority)),
            ("native_evidence", candidate.clone()),
            ("automatic_deletion_authorized", Value::Bool(false)),
            ("next_action", Value::string("Select a supported native refactor action and verify its result in an isolated candidate; never delete on reference absence alone.")),
        ]));
    }
    if found.len() != review.len() + structural.len() {
        return refuse("native triage contains an absent candidate ID");
    }
    proposals.sort_by_key(|item| {
        let priority = item.get("priority").and_then(Value::str).unwrap_or("");
        (priority != "REVIEW_NATIVE_REFERENCES", item.canonical())
    });
    obj(&[
        ("schema", Value::string("excession.improvement-plan.v1")),
        (
            "status",
            Value::string("OBSERVATIONS_REQUIRE_TRANSFORMATION"),
        ),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("native_report_id", report.get("report_id")?.clone()),
        ("coverage", report.get("coverage")?.clone()),
        ("candidate_count", Value::number(proposals.len())),
        ("candidates", Value::array(proposals)),
        ("automatic_deletion_authorized", Value::Bool(false)),
        ("ready_to_merge", Value::Bool(false)),
    ])
    .stamp_id("plan_id")
}

pub fn plan(repo: &Path, revision: &str, index: &Path, prefix: Option<&str>) -> Result<Value> {
    let report = scip_refactor::suggest_with_prefix(repo, revision, index, prefix)?;
    plan_from_report(&report)
}

fn changed_paths(inventory: &Value) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for item in inventory.get("changes")?.array_ref()? {
        for field in ["before", "after"] {
            let source = item.get(field)?;
            if source != &Value::Null {
                paths.insert(source.get("path")?.str()?.to_owned());
            }
        }
    }
    Ok(paths)
}

fn pair_values(pairs: impl IntoIterator<Item = Pair>) -> Value {
    Value::array(
        pairs
            .into_iter()
            .map(|(left, right)| Value::array([Value::string(left), Value::string(right)])),
    )
}

/// Compare stable path pairs, not content-derived IDs that necessarily change
/// when a source blob changes. Reindexing can make relationships disappear, so
/// we report a structural observation with remaining verification obligations.
fn evaluate_reports(
    before: &Value,
    after: &Value,
    inventory: &Value,
    original_plan: &Value,
) -> Result<Value> {
    let expected = plan_from_report(before)?;
    if &expected != original_plan || !original_plan.content_id_matches("plan_id")? {
        return refuse("improvement plan does not match replayed base SCIP evidence");
    }
    if before.get("source_commit")? != inventory.get("base_commit")?
        || before.get("source_tree")? != inventory.get("base_tree")?
        || after.get("source_commit")? != inventory.get("head_commit")?
        || after.get("source_tree")? != inventory.get("head_tree")?
    {
        return refuse("native source revisions do not match Git change inventory");
    }
    let old = pairs(before)?;
    let new = pairs(after)?;
    let modified = changed_paths(inventory)?;
    let removed: BTreeSet<_> = old.difference(&new).cloned().collect();
    let added: BTreeSet<_> = new.difference(&old).cloned().collect();
    let unexplained = removed
        .iter()
        .filter(|(a, b)| !modified.contains(a) && !modified.contains(b))
        .cloned()
        .collect::<BTreeSet<_>>();
    let before_documents = coverage_count(before, "indexed_documents")?;
    let after_documents = coverage_count(after, "indexed_documents")?;
    let before_unresolved = coverage_count(before, "unresolved_references")?;
    let after_unresolved = coverage_count(after, "unresolved_references")?;
    let coverage_regressed =
        after_documents < before_documents || after_unresolved > before_unresolved;
    let status = if coverage_regressed || !unexplained.is_empty() {
        "INCONCLUSIVE_EVIDENCE_CHANGE"
    } else if !removed.is_empty() && added.is_empty() {
        "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED"
    } else if !removed.is_empty() {
        "MIXED_STRUCTURAL_CHANGE"
    } else {
        "NO_STRUCTURAL_REDUCTION_OBSERVED"
    };
    obj(&[
        (
            "schema",
            Value::string("excession.improvement-evaluation.v1"),
        ),
        ("status", Value::string(status)),
        ("base_commit", inventory.get("base_commit")?.clone()),
        ("candidate_commit", inventory.get("head_commit")?.clone()),
        ("change_inventory_id", inventory.get("report_id")?.clone()),
        ("plan_id", original_plan.get("plan_id")?.clone()),
        ("base_native_report_id", before.get("report_id")?.clone()),
        (
            "candidate_native_report_id",
            after.get("report_id")?.clone(),
        ),
        ("reciprocal_pairs_before", Value::number(old.len())),
        ("reciprocal_pairs_after", Value::number(new.len())),
        ("removed_observed_pairs", pair_values(removed)),
        ("introduced_observed_pairs", pair_values(added)),
        ("unexplained_disappearances", pair_values(unexplained)),
        ("coverage_regressed", Value::Bool(coverage_regressed)),
        ("source_changes", Value::number(modified.len())),
        (
            "obligations",
            Value::array([
                Value::string("INDEPENDENT_BUILD_AND_TESTS_REQUIRED"),
                Value::string("PUBLIC_API_AND_SEMANTIC_COMPATIBILITY_UNVERIFIED"),
                Value::string("SCIP_COVERAGE_AND_INDEXER_CORRECTNESS_UNVERIFIED"),
                Value::string("BUILD_PROVENANCE_AND_REA_EVIDENCE_NOT_ATTESTED"),
            ]),
        ),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ])
    .stamp_id("report_id")
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

pub fn evaluate(request: &Evaluation<'_>) -> Result<Value> {
    let base_commit = git::rev(request.base_repo, request.base)?;
    let candidate_commit = git::rev(request.candidate_repo, request.candidate)?;
    if base_commit == candidate_commit
        || git::rev(request.candidate_repo, &base_commit)? != base_commit
        || git::rev(request.base_repo, &candidate_commit)? != candidate_commit
    {
        return refuse("evaluation requires two different commits available in both Git checkouts");
    }
    let bytes = fs::read(request.plan).map_err(|e| format!("read improvement plan: {e}"))?;
    if bytes.len() > MAX_PLAN_BYTES {
        return refuse("improvement plan exceeds 4 MiB");
    }
    let saved = json::parse(&bytes)?;
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
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
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
    let candidate = report.get("candidates")?.array_ref()?.iter()
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
        let Some(name) = symbol.rsplit('/').next().and_then(|s| s.strip_suffix("().")) else {
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
    let report = scip_refactor::suggest_with_prefix(
        request.repo,
        request.revision,
        request.index,
        request.source_prefix,
    )?;
    scip_refactor::audit_with_prefix(
        request.repo,
        request.revision,
        request.index,
        request.report,
        request.source_prefix,
    )?;
    let plan_bytes = fs::read(request.plan).map_err(|e| format!("read improvement plan: {e}"))?;
    if plan_bytes.len() > MAX_PLAN_BYTES {
        return refuse("improvement plan exceeds 4 MiB");
    }
    let original_plan = json::parse(&plan_bytes)?;
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
            ("status", Value::string(if possible.is_empty() {
                "NO_SUPPORTED_NATIVE_TRANSFORMATION"
            } else {
                "AMBIGUOUS_NATIVE_TRANSFORMATIONS"
            })),
            ("candidate_id", Value::string(request.candidate_id)),
            ("plan_id", original_plan.get("plan_id")?.clone()),
            ("supported_preview_count", Value::number(possible.len())),
            ("refusals", Value::array(rejected)),
            ("ready_to_merge", Value::Bool(false)),
        ]).stamp_id("trial_id");
    }
    let (source, destination, symbol, preview) = possible.remove(0);
    let outcome = if let Some(output) = request.output {
        let applied = crate::refactor_codegen::extract_shared_module(
            &crate::refactor_codegen::Request {
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
            },
        )?;
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
    ]).stamp_id("trial_id")
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
                ("symbol", Value::string("scip rust-analyzer pkg/selected().")),
                ("to", obj(&[
                    ("path", Value::string("core/src/a.rs")),
                    ("language", Value::string("rust")),
                ])),
            ]),
            obj(&[
                ("symbol", Value::string("scip rust-analyzer pkg/ignored().")),
                ("to", obj(&[
                    ("path", Value::string("core/src/lib.rs")),
                    ("language", Value::string("rust")),
                ])),
            ]),
            obj(&[
                ("symbol", Value::string("scip rust-analyzer pkg/helper().")),
                ("to", obj(&[
                    ("path", Value::string("core/src/a.rs")),
                    ("language", Value::string("python")),
                ])),
            ]),
        ]);
        let mut candidate = native.get("candidates").unwrap().array_ref().unwrap()[0].clone();
        candidate.entries_mut().unwrap().insert("references".into(), references);
        native.entries_mut().unwrap().insert("candidates".into(), Value::array([candidate]));
        let targets = extraction_targets(&native, "candidate-a", None).unwrap();
        assert_eq!(targets, vec![(
            "core/src/a.rs".to_owned(),
            "core/src/a_selected_extracted.rs".to_owned(),
            "selected".to_owned(),
        )]);
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
        assert!(pairs(&duplicate).is_err());
    }
}
