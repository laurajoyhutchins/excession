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
        ("status", Value::string("OBSERVATIONS_REQUIRE_TRANSFORMATION")),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("native_report_id", report.get("report_id")?.clone()),
        ("coverage", report.get("coverage")?.clone()),
        ("candidate_count", Value::number(proposals.len())),
        ("candidates", Value::array(proposals)),
        ("automatic_deletion_authorized", Value::Bool(false)),
        ("ready_to_merge", Value::Bool(false)),
    ]).stamp_id("plan_id")
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
    Value::array(pairs.into_iter().map(|(left, right)| {
        Value::array([Value::string(left), Value::string(right)])
    }))
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
        ("schema", Value::string("excession.improvement-evaluation.v1")),
        ("status", Value::string(status)),
        ("base_commit", inventory.get("base_commit")?.clone()),
        ("candidate_commit", inventory.get("head_commit")?.clone()),
        ("change_inventory_id", inventory.get("report_id")?.clone()),
        ("plan_id", original_plan.get("plan_id")?.clone()),
        ("base_native_report_id", before.get("report_id")?.clone()),
        ("candidate_native_report_id", after.get("report_id")?.clone()),
        ("reciprocal_pairs_before", Value::number(old.len())),
        ("reciprocal_pairs_after", Value::number(new.len())),
        ("removed_observed_pairs", pair_values(removed)),
        ("introduced_observed_pairs", pair_values(added)),
        ("unexplained_disappearances", pair_values(unexplained)),
        ("coverage_regressed", Value::Bool(coverage_regressed)),
        ("source_changes", Value::number(modified.len())),
        ("obligations", Value::array([
            Value::string("INDEPENDENT_BUILD_AND_TESTS_REQUIRED"),
            Value::string("PUBLIC_API_AND_SEMANTIC_COMPATIBILITY_UNVERIFIED"),
            Value::string("SCIP_COVERAGE_AND_INDEXER_CORRECTNESS_UNVERIFIED"),
            Value::string("BUILD_PROVENANCE_AND_REA_EVIDENCE_NOT_ATTESTED"),
        ])),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ]).stamp_id("report_id")
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
        request.base_repo, &base_commit, request.base_index, request.source_prefix,
    )?;
    let after = scip_refactor::suggest_with_prefix(
        request.candidate_repo, &candidate_commit, request.candidate_index, request.source_prefix,
    )?;
    let inventory = change_inventory::inventory(request.candidate_repo, &base_commit, &candidate_commit)?;
    evaluate_reports(&before, &after, &inventory, &saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(commit: &str, tree: &str, pair: bool, documents: i64, unresolved: i64) -> Value {
        let candidate = obj(&[
            ("candidate_id", Value::string("candidate-a")),
            ("kind", Value::string("reciprocal_symbol_references")),
            ("sources", Value::array([
                obj(&[("path", Value::string("src/a.rs"))]),
                obj(&[("path", Value::string("src/b.rs"))]),
            ])),
        ]);
        obj(&[
            ("schema", Value::string("excession.scip-refactor.v1")),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("report_id", Value::string("native-id")),
            ("coverage", obj(&[
                ("indexed_documents", Value::number(documents)),
                ("unresolved_references", Value::number(unresolved)),
            ])),
            ("candidates", Value::array(if pair { vec![candidate] } else { vec![] })),
            ("triage", obj(&[
                ("review_candidate_ids", Value::array(if pair { vec![Value::string("candidate-a")] } else { vec![] })),
                ("likely_structural_module_candidate_ids", Value::array([])),
            ])),
        ])
    }

    fn inventory(changed: bool) -> Value {
        obj(&[
            ("base_commit", Value::string("base")),
            ("base_tree", Value::string("bt")),
            ("head_commit", Value::string("head")),
            ("head_tree", Value::string("ht")),
            ("report_id", Value::string("git-id")),
            ("changes", Value::array(if changed {
                vec![obj(&[
                    ("before", obj(&[("path", Value::string("src/a.rs"))])),
                    ("after", obj(&[("path", Value::string("src/a.rs"))])),
                ])]
            } else { vec![] })),
        ])
    }

    #[test]
    fn structural_delta_is_observation_not_merge_authorization() {
        let before = report("base", "bt", true, 3, 0);
        let after = report("head", "ht", false, 3, 0);
        let plan = plan_from_report(&before).unwrap();
        let result = evaluate_reports(&before, &after, &inventory(true), &plan).unwrap();
        assert_eq!(result.get("status").unwrap().str().unwrap(), "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED");
        assert!(!result.get("ready_to_merge").unwrap().bool().unwrap());
        assert!(!result.get("automatic_deletion_authorized").unwrap().bool().unwrap());
        assert_eq!(result.get("reciprocal_pairs_before").unwrap().integer().unwrap(), 1);
    }

    #[test]
    fn coverage_loss_or_unrelated_disappearance_is_inconclusive() {
        let before = report("base", "bt", true, 3, 0);
        let plan = plan_from_report(&before).unwrap();
        for (documents, unresolved, changed) in [(2, 0, true), (3, 1, true), (3, 0, false)] {
            let after = report("head", "ht", false, documents, unresolved);
            let result = evaluate_reports(&before, &after, &inventory(changed), &plan).unwrap();
            assert_eq!(result.get("status").unwrap().str().unwrap(), "INCONCLUSIVE_EVIDENCE_CHANGE");
        }
    }

    #[test]
    fn mismatched_plan_and_revisions_refuse() {
        let before = report("base", "bt", true, 3, 0);
        let after = report("head", "ht", false, 3, 0);
        let mut plan = plan_from_report(&before).unwrap();
        plan.entries_mut().unwrap().insert("candidate_count".into(), Value::number(100));
        assert!(evaluate_reports(&before, &after, &inventory(true), &plan).is_err());
        let good_plan = plan_from_report(&before).unwrap();
        let wrong_head = report("other", "ht", false, 3, 0);
        assert!(evaluate_reports(&before, &wrong_head, &inventory(true), &good_plan).is_err());
    }

    #[test]
    fn triage_mismatch_and_duplicate_native_pairs_refuse() {
        let mut broken = report("base", "bt", true, 3, 0);
        broken.get("triage").unwrap();
        broken.entries_mut().unwrap().insert("triage".into(), obj(&[
            ("review_candidate_ids", Value::array([])),
            ("likely_structural_module_candidate_ids", Value::array([])),
        ]));
        assert!(plan_from_report(&broken).is_err());
        let original = report("base", "bt", true, 3, 0);
        let mut duplicate = original.clone();
        let candidate = duplicate.get("candidates").unwrap().array_ref().unwrap()[0].clone();
        duplicate.entries_mut().unwrap().insert(
            "candidates".into(), Value::array([candidate.clone(), candidate]),
        );
        assert!(pairs(&duplicate).is_err());
    }
}
