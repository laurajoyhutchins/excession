//! Language-neutral candidate and delta model over existing Git-bound producers.
//! Native producer claims are preserved, not re-parsed or upgraded into safety proofs.
use crate::json::{Value, obj};
use crate::{Result, git, refuse, source_capabilities};
use std::collections::BTreeSet;

pub(crate) const MAX_PLAN_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct Location {
    path: String,
    symbol: String,
}

impl Location {
    fn from_path(value: &Value, symbol: &str) -> Result<Self> {
        let path = value.get("path")?.str()?;
        git::validate_path(path)?;
        Ok(Self {
            path: path.to_owned(),
            symbol: symbol.to_owned(),
        })
    }

    fn json(&self) -> Value {
        obj(&[
            ("path", Value::string(&self.path)),
            ("symbol", Value::string(&self.symbol)),
        ])
    }
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct Finding {
    kind: String,
    locations: Vec<Location>,
}

impl Finding {
    fn from_candidate(candidate: &Value, producer: &str) -> Result<Self> {
        let kind = candidate.get("kind")?.str()?;
        let mut locations = match (producer, kind) {
            ("scip", "reciprocal_symbol_references") => {
                let sources = candidate.get("sources")?.array_ref()?;
                if sources.len() != 2 {
                    return refuse("reciprocal candidate requires two source files");
                }
                sources
                    .iter()
                    .map(|s| Location::from_path(s, ""))
                    .collect::<Result<Vec<_>>>()?
            }
            ("scip", "cross_language_symbol_boundary") => {
                let reference = candidate.get("reference")?;
                let symbol = reference.get("symbol")?.str()?;
                ["from", "to"]
                    .iter()
                    .map(|side| Location::from_path(reference.get(side)?, symbol))
                    .collect::<Result<Vec<_>>>()?
            }
            ("indexed", "dependency_cycle") => {
                let sources = candidate.get("modules")?.array_ref()?;
                if sources.len() < 2 {
                    return refuse("indexed dependency cycle must contain at least two modules");
                }
                sources
                    .iter()
                    .map(|s| Location::from_path(s, ""))
                    .collect::<Result<Vec<_>>>()?
            }
            ("indexed", "duplicate_body_fingerprint") => {
                let symbols = candidate.get("symbols")?.array_ref()?;
                if symbols.len() < 2 {
                    return refuse("indexed duplication requires at least two symbols");
                }
                symbols
                    .iter()
                    .map(|s| Location::from_path(s, s.get("name")?.str()?))
                    .collect::<Result<Vec<_>>>()?
            }
            _ => return refuse(format!("unsupported {producer} improvement kind: {kind}")),
        };
        locations.sort();
        locations.dedup();
        if locations.len() < 2 {
            return refuse("candidate must contain at least two distinct source locations");
        }
        Ok(Self {
            kind: kind.to_owned(),
            locations,
        })
    }

    fn json(&self) -> Value {
        obj(&[
            ("kind", Value::string(&self.kind)),
            (
                "locations",
                Value::array(self.locations.iter().map(Location::json)),
            ),
        ])
    }

    fn touches(&self, paths: &BTreeSet<String>) -> bool {
        self.locations.iter().any(|l| paths.contains(&l.path))
    }
}

fn producer(report: &Value) -> Result<&'static str> {
    match report.get("schema")?.str()? {
        "excession.scip-refactor.v1" => Ok("scip"),
        "excession.refactor-candidates.v1" => Ok("indexed"),
        other => refuse(format!("unsupported improvement producer report: {other}")),
    }
}

pub(crate) fn findings(report: &Value) -> Result<BTreeSet<Finding>> {
    let source = producer(report)?;
    let mut items = BTreeSet::new();
    for candidate in report.get("candidates")?.array_ref()? {
        let finding = Finding::from_candidate(candidate, source)?;
        if !items.insert(finding) {
            return refuse("duplicate normalized improvement finding");
        }
    }
    Ok(items)
}

pub(crate) fn report_ids(report: &Value, key: &str) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for id in report.get("triage")?.get(key)?.array_ref()? {
        if !result.insert(id.str()?.to_owned()) {
            return refuse("duplicate SCIP triage candidate ID");
        }
    }
    Ok(result)
}

pub(crate) fn plan_from_report(report: &Value) -> Result<Value> {
    let provider = producer(report)?;
    let review = if provider == "scip" {
        report_ids(report, "review_candidate_ids")?
    } else {
        BTreeSet::new()
    };
    let structural = if provider == "scip" {
        report_ids(report, "likely_structural_module_candidate_ids")?
    } else {
        BTreeSet::new()
    };
    if !review.is_disjoint(&structural) {
        return refuse("SCIP triage partitions overlap");
    }
    let mut found_ids = BTreeSet::new();
    let mut found_locations = BTreeSet::new();
    let mut candidates = Vec::new();
    for item in report.get("candidates")?.array_ref()? {
        let id = item.get("candidate_id")?.str()?;
        if !found_ids.insert(id.to_owned()) {
            return refuse("duplicate producer candidate identity");
        }
        if provider == "scip" && !review.contains(id) && !structural.contains(id) {
            return refuse("SCIP candidate missing from producer triage");
        }
        let finding = Finding::from_candidate(item, provider)?;
        if !found_locations.insert(finding.clone()) {
            return refuse("duplicate producer candidate locations");
        }
        let rust_action = provider == "scip"
            && item.get("kind")?.str()? == "reciprocal_symbol_references"
            && report
                .get("indexer")
                .ok()
                .and_then(|value| value.get("name").ok())
                .and_then(|value| value.str().ok())
                == Some("rust-analyzer")
            && item
                .get("sources")?
                .array_ref()?
                .iter()
                .all(|source| source.get("language").and_then(Value::str).ok() == Some("rust"));
        let operation = if rust_action {
            "RUST_AST_EXTRACTION_IF_SUPPORTED"
        } else {
            "NO_NATIVE_EDIT_PRODUCER"
        };
        candidates.push(obj(&[
            ("candidate_id", Value::string(id)),
            ("kind", item.get("kind")?.clone()),
            ("finding", finding.json()),
            (
                "priority",
                Value::string(if provider == "scip" && structural.contains(id) {
                    "LIKELY_MODULE_DECLARATION"
                } else {
                    "REVIEW_PRODUCER_EVIDENCE"
                }),
            ),
            ("edit_capability", Value::string(operation)),
            ("producer_evidence", item.clone()),
            ("automatic_deletion_authorized", Value::Bool(false)),
        ]));
    }
    if provider == "scip" && found_ids.len() != review.len() + structural.len() {
        return refuse("SCIP triage references absent candidates");
    }
    candidates.sort_by_key(|item| {
        let priority = item.get("priority").and_then(Value::str).unwrap_or("");
        (priority != "REVIEW_PRODUCER_EVIDENCE", item.canonical())
    });
    let report = obj(&[
        ("schema", Value::string("excession.improvement-plan.v1")),
        (
            "status",
            Value::string("OBSERVATIONS_REQUIRE_TRANSFORMATION"),
        ),
        ("producer", Value::string(provider)),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("producer_report_id", report.get("report_id")?.clone()),
        ("coverage", report.get("coverage")?.clone()),
        ("candidate_count", Value::number(candidates.len())),
        ("candidates", Value::array(candidates)),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ])
    .stamp_id("plan_id")?;
    if report.canonical().len() > MAX_PLAN_BYTES {
        return refuse("improvement plan exceeds 4 MiB; narrow producer scope");
    }
    Ok(report)
}

fn coverage_number(report: &Value, key: &str) -> Result<i64> {
    let count = report.get("coverage")?.get(key)?.integer()?;
    if count < 0 {
        return refuse("negative improvement producer coverage metric");
    }
    Ok(count)
}

fn coverage_regressed(before: &Value, after: &Value, provider: &str) -> Result<bool> {
    let (sources, unresolved) = match provider {
        "scip" => ("indexed_documents", "unresolved_references"),
        "indexed" => ("indexed_modules", "indexed_hazards"),
        _ => return refuse("unsupported improvement evidence producer"),
    };
    Ok(
        coverage_number(after, sources)? < coverage_number(before, sources)?
            || coverage_number(after, unresolved)? > coverage_number(before, unresolved)?,
    )
}

fn changed_paths(inventory: &Value) -> Result<BTreeSet<String>> {
    let mut changed = BTreeSet::new();
    for item in inventory.get("changes")?.array_ref()? {
        for field in ["before", "after"] {
            let source = item.get(field)?;
            if source != &Value::Null {
                let path = source.get("path")?.str()?;
                git::validate_path(path)?;
                changed.insert(path.to_owned());
            }
        }
    }
    Ok(changed)
}

fn values(findings: impl IntoIterator<Item = Finding>) -> Value {
    Value::array(findings.into_iter().map(|f| f.json()))
}

/// Compare source-anchored findings across independently replayed producer
/// reports. Path/symbol changes may affect identity; disappearance is never
/// interpreted as proof of dead code or of semantic equivalence.
pub(crate) fn evaluate_reports(
    before: &Value,
    after: &Value,
    inventory: &Value,
    original_plan: &Value,
) -> Result<Value> {
    let provider = producer(before)?;
    if producer(after)? != provider {
        return refuse("cannot compare different improvement observation producers");
    }
    let expected = plan_from_report(before)?;
    if &expected != original_plan || !original_plan.content_id_matches("plan_id")? {
        return refuse("improvement plan is not the replayed base observation");
    }
    if before.get("source_commit")? != inventory.get("base_commit")?
        || before.get("source_tree")? != inventory.get("base_tree")?
        || after.get("source_commit")? != inventory.get("head_commit")?
        || after.get("source_tree")? != inventory.get("head_tree")?
    {
        return refuse("improvement observations do not match exact Git change inventory");
    }
    let old = findings(before)?;
    let new = findings(after)?;
    let changed = changed_paths(inventory)?;
    let removed: BTreeSet<_> = old.difference(&new).cloned().collect();
    let introduced: BTreeSet<_> = new.difference(&old).cloned().collect();
    let unexplained: BTreeSet<_> = removed
        .iter()
        .filter(|f| !f.touches(&changed))
        .cloned()
        .collect();
    let mut new_sources = BTreeSet::new();
    for change in inventory.get("changes")?.array_ref()? {
        if change.get("kind")?.str()? != "ADDED" {
            continue;
        }
        let path = change.get("after")?.get("path")?.str()?;
        git::validate_path(path)?;
        if source_capabilities::detected_language(path).is_some() {
            new_sources.insert(path.to_owned());
        }
    }
    let loss = coverage_regressed(before, after, provider)?;
    let status = if loss || !unexplained.is_empty() {
        "INCONCLUSIVE_EVIDENCE_CHANGE"
    } else if !removed.is_empty() && !new_sources.is_empty() {
        "STRUCTURAL_RELOCATION_OBSERVED_UNVERIFIED"
    } else if !removed.is_empty() && introduced.is_empty() {
        "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED"
    } else if !removed.is_empty() {
        "MIXED_STRUCTURAL_CHANGE"
    } else {
        "NO_STRUCTURAL_REDUCTION_OBSERVED"
    };
    let mut result = obj(&[
        (
            "schema",
            Value::string("excession.improvement-evaluation.v1"),
        ),
        ("producer", Value::string(provider)),
        ("status", Value::string(status)),
        ("base_commit", inventory.get("base_commit")?.clone()),
        ("candidate_commit", inventory.get("head_commit")?.clone()),
        ("change_inventory_id", inventory.get("report_id")?.clone()),
        ("plan_id", original_plan.get("plan_id")?.clone()),
        ("base_producer_report_id", before.get("report_id")?.clone()),
        (
            "candidate_producer_report_id",
            after.get("report_id")?.clone(),
        ),
        ("observed_findings_before", Value::number(old.len())),
        ("observed_findings_after", Value::number(new.len())),
        ("removed_findings", values(removed.clone())),
        ("introduced_findings", values(introduced)),
        ("unexplained_disappearances", values(unexplained)),
        ("coverage_regressed", Value::Bool(loss)),
        ("source_changes", Value::number(changed.len())),
        (
            "introduced_source_paths",
            Value::array(new_sources.into_iter().map(Value::string)),
        ),
        (
            "obligations",
            Value::array([
                Value::string("INDEPENDENT_NATIVE_BUILD_AND_TEST_REQUIRED"),
                Value::string("PUBLIC_API_AND_SEMANTIC_COMPATIBILITY_UNVERIFIED"),
                Value::string("SOURCE_RELATIONSHIP_COVERAGE_UNVERIFIED"),
                Value::string("BUILD_PROVENANCE_AND_REA_EVIDENCE_NOT_ATTESTED"),
            ]),
        ),
        ("ready_to_merge", Value::Bool(false)),
        ("automatic_deletion_authorized", Value::Bool(false)),
    ]);
    // Retain the older SCIP-only counts while moving all decision logic to
    // normalized findings shared with the indexed Python/JavaScript producer.
    if provider == "scip" {
        let reciprocal = |f: &&Finding| f.kind == "reciprocal_symbol_references";
        result.entries_mut()?.insert(
            "reciprocal_pairs_before".into(),
            Value::number(old.iter().filter(reciprocal).count()),
        );
        result.entries_mut()?.insert(
            "reciprocal_pairs_after".into(),
            Value::number(new.iter().filter(reciprocal).count()),
        );
    }
    result.stamp_id("report_id")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(kind: &str, source: &str) -> Value {
        let (candidate, coverage) = if kind == "dependency_cycle" {
            (
                obj(&[
                    ("kind", Value::string(kind)),
                    ("candidate_id", Value::string("a")),
                    (
                        "modules",
                        Value::array([
                            obj(&[("path", Value::string("lib/a.py"))]),
                            obj(&[("path", Value::string("lib/b.py"))]),
                        ]),
                    ),
                ]),
                obj(&[
                    ("indexed_modules", Value::number(2)),
                    ("indexed_hazards", Value::number(0)),
                ]),
            )
        } else {
            (
                obj(&[
                    ("kind", Value::string(kind)),
                    ("candidate_id", Value::string("b")),
                    (
                        "symbols",
                        Value::array([
                            obj(&[
                                ("path", Value::string("web/a.js")),
                                ("name", Value::string("foo")),
                            ]),
                            obj(&[
                                ("path", Value::string("web/b.js")),
                                ("name", Value::string("foo")),
                            ]),
                        ]),
                    ),
                ]),
                obj(&[
                    ("indexed_modules", Value::number(2)),
                    ("indexed_hazards", Value::number(0)),
                ]),
            )
        };
        obj(&[
            ("schema", Value::string("excession.refactor-candidates.v1")),
            ("source_commit", Value::string(source)),
            (
                "source_tree",
                Value::string(if source == "base" { "bt" } else { "ht" }),
            ),
            ("report_id", Value::string("report")),
            ("coverage", coverage),
            ("candidates", Value::array([candidate])),
        ])
    }

    fn inventory() -> Value {
        obj(&[
            ("base_commit", Value::string("base")),
            ("base_tree", Value::string("bt")),
            ("head_commit", Value::string("head")),
            ("head_tree", Value::string("ht")),
            ("report_id", Value::string("changes")),
            (
                "changes",
                Value::array([obj(&[
                    ("kind", Value::string("MODIFIED")),
                    ("before", obj(&[("path", Value::string("lib/a.py"))])),
                    ("after", obj(&[("path", Value::string("lib/a.py"))])),
                ])]),
            ),
        ])
    }

    #[test]
    fn non_rust_scip_candidates_never_advertise_rust_edits() {
        let candidate = obj(&[
            ("candidate_id", Value::string("scip-candidate")),
            ("kind", Value::string("reciprocal_symbol_references")),
            (
                "sources",
                Value::array([
                    obj(&[
                        ("path", Value::string("src/a.go")),
                        ("language", Value::string("go")),
                    ]),
                    obj(&[
                        ("path", Value::string("src/b.go")),
                        ("language", Value::string("go")),
                    ]),
                ]),
            ),
        ]);
        let scip = obj(&[
            ("schema", Value::string("excession.scip-refactor.v1")),
            ("source_commit", Value::string("head")),
            ("source_tree", Value::string("tree")),
            ("report_id", Value::string("report")),
            ("indexer", obj(&[("name", Value::string("rust-analyzer"))])),
            ("coverage", obj(&[])),
            ("candidates", Value::array([candidate])),
            (
                "triage",
                obj(&[
                    (
                        "review_candidate_ids",
                        Value::array([Value::string("scip-candidate")]),
                    ),
                    ("likely_structural_module_candidate_ids", Value::array([])),
                ]),
            ),
        ]);
        let plan = plan_from_report(&scip).unwrap();
        let chosen = &plan.get("candidates").unwrap().array_ref().unwrap()[0];
        assert_eq!(
            chosen.get("edit_capability").unwrap().str().unwrap(),
            "NO_NATIVE_EDIT_PRODUCER"
        );
    }

    #[test]
    fn indexed_python_cycle_is_planable_without_rust_actions() {
        let report = fixture("dependency_cycle", "base");
        let plan = plan_from_report(&report).unwrap();
        assert_eq!(plan.get("producer").unwrap().str().unwrap(), "indexed");
        let item = &plan.get("candidates").unwrap().array_ref().unwrap()[0];
        assert_eq!(
            item.get("edit_capability").unwrap().str().unwrap(),
            "NO_NATIVE_EDIT_PRODUCER"
        );
        assert!(
            !plan
                .get("automatic_deletion_authorized")
                .unwrap()
                .bool()
                .unwrap()
        );
    }

    #[test]
    fn indexed_javascript_duplication_is_not_equivalence() {
        let report = fixture("duplicate_body_fingerprint", "base");
        let plan = plan_from_report(&report).unwrap();
        assert_eq!(plan.get("candidate_count").unwrap().integer().unwrap(), 1);
        let item = &plan.get("candidates").unwrap().array_ref().unwrap()[0];
        assert_eq!(
            item.get("kind").unwrap().str().unwrap(),
            "duplicate_body_fingerprint"
        );
        assert!(!plan.get("ready_to_merge").unwrap().bool().unwrap());
    }

    #[test]
    fn indexed_cycle_disappearance_is_still_unverified() {
        let base = fixture("dependency_cycle", "base");
        let mut head = fixture("dependency_cycle", "head");
        head.entries_mut()
            .unwrap()
            .insert("candidates".into(), Value::array([]));
        let plan = plan_from_report(&base).unwrap();
        let evaluated = evaluate_reports(&base, &head, &inventory(), &plan).unwrap();
        assert_eq!(
            evaluated.get("status").unwrap().str().unwrap(),
            "STRUCTURAL_REDUCTION_OBSERVED_UNVERIFIED"
        );
        assert_eq!(
            evaluated
                .get("observed_findings_after")
                .unwrap()
                .integer()
                .unwrap(),
            0
        );
        assert!(!evaluated.get("ready_to_merge").unwrap().bool().unwrap());
    }

    #[test]
    fn rejects_producer_mismatch_unrelated_disappearance_and_unsupported_shapes() {
        let before = fixture("dependency_cycle", "base");
        let mut after = fixture("dependency_cycle", "head");
        after
            .entries_mut()
            .unwrap()
            .insert("candidates".into(), Value::array([]));
        let plan = plan_from_report(&before).unwrap();
        let mut unrelated = inventory();
        unrelated
            .entries_mut()
            .unwrap()
            .insert("changes".into(), Value::array([]));
        assert_eq!(
            evaluate_reports(&before, &after, &unrelated, &plan)
                .unwrap()
                .get("status")
                .unwrap()
                .str()
                .unwrap(),
            "INCONCLUSIVE_EVIDENCE_CHANGE"
        );
        let mut forged = before.clone();
        forged
            .entries_mut()
            .unwrap()
            .insert("schema".into(), Value::string("invented.producer"));
        assert!(plan_from_report(&forged).is_err());
        assert!(evaluate_reports(&before, &forged, &inventory(), &plan).is_err());
    }

    #[test]
    fn generic_relocation_does_not_require_rust() {
        let base = fixture("dependency_cycle", "base");
        let mut head = fixture("dependency_cycle", "head");
        head.entries_mut()
            .unwrap()
            .insert("candidates".into(), Value::array([]));
        let plan = plan_from_report(&base).unwrap();
        let mut evidence = inventory();
        let mut rows = evidence
            .get("changes")
            .unwrap()
            .array_ref()
            .unwrap()
            .to_vec();
        rows.push(obj(&[
            ("kind", Value::string("ADDED")),
            ("before", Value::Null),
            ("after", obj(&[("path", Value::string("lib/extracted.py"))])),
        ]));
        evidence
            .entries_mut()
            .unwrap()
            .insert("changes".into(), Value::array(rows));
        let evaluated = evaluate_reports(&base, &head, &evidence, &plan).unwrap();
        assert_eq!(
            evaluated.get("status").unwrap().str().unwrap(),
            "STRUCTURAL_RELOCATION_OBSERVED_UNVERIFIED"
        );
        assert_eq!(
            evaluated
                .get("introduced_source_paths")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            1
        );
    }
}
