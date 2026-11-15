//! Improvement orchestration over native, source-bound evidence producers.
//! The normalized candidate and evaluation model is language-neutral.
use crate::json::{self, Value};
use crate::improve_generic::{evaluate_reports, plan_from_report, MAX_PLAN_BYTES};
use crate::sqlite::Db;
use crate::{Result, change_inventory, git, refactor, refuse, scip_refactor};
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

#[path = "improve_rust.rs"]
mod rust_trial;
pub use rust_trial::{Trial, trial};

#[cfg(test)]
use crate::json::obj;
#[cfg(test)]
use rust_trial::extraction_targets;

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
