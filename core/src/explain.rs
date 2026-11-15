//! Exact report replay and source-bound explanation.
//! Does not promote candidate dependencies to verified behavior.
use crate::analyze;
use crate::discovery;
use crate::impact_query::{self, EvidenceByRevision};
use crate::json::{self, Value, obj};
use crate::observed_relations::{self, Plan, Selection};
use crate::sqlite::Db;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BATCH_SOURCES: usize = 128;
const MAX_WITNESS_STEPS: usize = 10_000;

fn supported_report(path: &Path) -> Result<Value> {
    let len = fs::metadata(path)
        .map_err(|e| format!("impact report: {e}"))?
        .len();
    if len > MAX_REPORT_BYTES {
        return refuse("impact report exceeds 64 MiB input limit");
    }
    let bytes = fs::read(path).map_err(|e| format!("impact report: {e}"))?;
    let report = json::parse(&bytes)?;
    if report.get("schema")?.str()? != "excession.impact.v1" {
        return refuse("expected an excession.impact.v1 report");
    }
    Ok(report)
}

fn find_claim<'a>(report: &'a Value, id: &str) -> Result<&'a Value> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return refuse("explanation claim ID requires a lowercase SHA-256 digest");
    }
    let mut found = None;
    for effect in report.get("effects")?.array_ref()? {
        let actual = effect.get("claim_id")?.str()?;
        if !effect.content_id_matches("claim_id")? {
            return refuse("impact claim identifier disagrees with its content");
        }
        if actual == id && found.replace(effect).is_some() {
            return refuse("ambiguous impact claim identifier");
        }
    }
    found.ok_or_else(|| format!("unknown impact claim: {id}"))
}

/// Recompute the complete candidate impact report before selecting a claim.
/// Matching JSON alone would only establish self-consistency; reindexing
/// authenticates source against Git objects and repeats the native analyzers.
fn checked_report(
    repo: &Path,
    saved_report: &Path,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let reported = supported_report(saved_report)?;
    let base = reported.get("base_commit")?.str()?;
    let head = reported.get("head_commit")?.str()?;
    let fresh = if let Some(chosen) = reported.entries()?.get("producer_selection_plan") {
        let discovered = discovery::inspect(repo, head)?;
        let db_path = analyze::checked_external_destination(
            &discovered.root,
            db.map(Path::new).unwrap_or(&discovered.index_path),
            "SQLite",
        )?;
        let before = chosen.get("base")?;
        let after = chosen.get("head")?;
        let (base_plan, head_plan) = if db_path.is_file() {
            let store = Db::open(&db_path)?;
            (
                observed_relations::pinned_plan(&store, &discovered.root, base, before)?,
                observed_relations::pinned_plan(&store, &discovered.root, head, after)?,
            )
        } else {
            let empty = Plan::default();
            if before != &empty.document() || after != &empty.document() {
                return refuse("saved impact needs an existing native evidence store");
            }
            (Plan::default(), Plan::default())
        };
        impact_query::run_with_plans(repo, base, head, roots, db, &base_plan, &head_plan)?
    } else if let Some(choices) = reported.entries()?.get("producer_selections") {
        let selected = |key: &str| -> Result<Option<&str>> {
            match choices.get(key)? {
                Value::Null => Ok(None),
                item => Ok(Some(item.str()?)),
            }
        };
        impact_query::run_with_evidence(
            repo,
            base,
            head,
            roots,
            db,
            EvidenceByRevision {
                base: Selection {
                    scip_snapshot: selected("base_scip_snapshot")?,
                    graph_id: selected("base_graph_id")?,
                    source_prefix: selected("base_source_prefix")?,
                },
                head: Selection {
                    scip_snapshot: selected("head_scip_snapshot")?,
                    graph_id: selected("head_graph_id")?,
                    source_prefix: selected("head_source_prefix")?,
                },
            },
        )?
    } else {
        impact_query::run(repo, base, head, roots, db)?
    };
    if reported.canonical() != fresh.canonical() {
        return refuse(
            "impact report does not replay against pinned Git source and configured analyzers",
        );
    }
    Ok(fresh)
}

fn checked_claim(
    repo: &Path,
    saved_report: &Path,
    claim_id: &str,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<(Value, Value)> {
    let report = checked_report(repo, saved_report, roots, db)?;
    let effect = find_claim(&report, claim_id)?.clone();
    Ok((report, effect))
}

/// One authenticated source witness per (revision, dependent source). A
/// witness is a decreasing-distance certificate into a changed Git source;
/// it does not assert that the change affects runtime behavior.
fn source_witnesses(report: &Value) -> Result<BTreeMap<(String, String), Value>> {
    let mut by_source = BTreeMap::new();
    for witness in report.get("candidate_source_impacts")?.array_ref()? {
        if witness.get("assessment")?.str()? != "CANDIDATE_PATH" {
            return refuse("distinct-source witness is not a candidate");
        }
        let entity = witness.get("entity")?;
        let hop = witness.get("next_hop")?;
        let commit = entity.get("source_commit")?.str()?;
        let path = entity.get("path")?.str()?;
        let target = hop.get("target")?;
        if entity != hop.get("source")?
            || hop.get("source")?.get("source_commit")?.str()? != commit
            || target.get("source_commit")?.str()? != commit
            || target.get("path")?.str()? == path
            || hop.get("evidence")?.array_ref()?.is_empty()
            || witness.get("distance_to_change")?.integer()? <= 0
        {
            return refuse("invalid source witness identity, rank, or evidence");
        }
        if by_source
            .insert((commit.to_owned(), path.to_owned()), witness.clone())
            .is_some()
        {
            return refuse("ambiguous duplicate distinct-source witness");
        }
    }
    Ok(by_source)
}

fn explain_witness(
    report: &Value,
    by_source: &BTreeMap<(String, String), Value>,
    witness: &Value,
) -> Result<Value> {
    let entity = witness.get("entity")?;
    let commit = entity.get("source_commit")?.str()?;
    let mut path = entity.get("path")?.str()?.to_owned();
    let mut rank = witness.get("distance_to_change")?.integer()?;
    if rank <= 0 {
        return refuse("source witness rank must be positive");
    }
    let mut seen = BTreeSet::new();
    let mut steps = Vec::new();
    while rank > 0 {
        if steps.len() >= MAX_WITNESS_STEPS || !seen.insert(path.clone()) {
            return refuse("source witness cycle or depth limit");
        }
        let current = by_source
            .get(&(commit.to_owned(), path.clone()))
            .ok_or("missing intermediate source witness")?;
        if current.get("distance_to_change")?.integer()? != rank {
            return refuse("source witness rank does not decrease by one");
        }
        let hop = current.get("next_hop")?;
        if hop.get("source")? != current.get("entity")?
            || hop.get("target")?.get("source_commit")?.str()? != commit
            || hop.get("evidence")?.array_ref()?.is_empty()
        {
            return refuse("source witness has inconsistent Git evidence");
        }
        path = hop.get("target")?.get("path")?.str()?.to_owned();
        steps.push(hop.clone());
        rank -= 1;
    }
    if seen.contains(&path) {
        return refuse("source witness terminates in a cycle");
    }
    let claim_id = witness.clone().stamp_id("claim_id")?;
    obj(&[
        ("schema", Value::string("excession.explanation.v1")),
        ("selection_kind", Value::string("DISTINCT_SOURCE_WITNESS")),
        ("claim_id", claim_id.get("claim_id")?.clone()),
        ("source_commit", Value::string(commit)),
        ("steps", Value::array(steps)),
        ("coverage", report.get("coverage")?.clone()),
    ])
    .stamp_id("report_id")
}

/// Replay the *complete* saved impact report from Git and pinned analyzers once,
/// then reconstruct the requested source path without enumerating alternatives.
/// A path present in both revisions needs an explicit source commit.
pub fn explain_source(
    repo: &Path,
    saved_report: &Path,
    source_path: &str,
    source_commit: Option<&str>,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    crate::git::validate_path(source_path)?;
    let report = checked_report(repo, saved_report, roots, db)?;
    let by_source = source_witnesses(&report)?;
    let matches = by_source
        .iter()
        .filter(|((commit, path), _)| {
            path == source_path && source_commit.is_none_or(|selected| commit == selected)
        })
        .map(|(_, witness)| witness)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        if matches.is_empty() && !report.get("candidate_source_enumeration_complete")?.bool()? {
            return refuse("source absent from bounded witness inventory; enumeration was truncated");
        }
        return refuse(if matches.is_empty() {
            "no distinct-source impact witness for selected Git source"
        } else {
            "source exists in multiple revisions; supply --source-commit"
        });
    }
    explain_witness(&report, &by_source, matches[0])
}

/// Bulk explanations share one exact replay, replacing repeated fresh
/// analyzer invocations. Refuse oversized result sets rather than silently
/// truncating, and preserve partial-inventory status in the JSON output.
pub fn explain_all_sources(
    repo: &Path,
    saved_report: &Path,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let report = checked_report(repo, saved_report, roots, db)?;
    let witnesses = source_witnesses(&report)?;
    if witnesses.len() > MAX_BATCH_SOURCES {
        return refuse("too many source witnesses for one batch; use --source");
    }
    let mut explanations = Vec::with_capacity(witnesses.len());
    for witness in witnesses.values() {
        explanations.push(explain_witness(&report, &witnesses, witness)?);
    }
    obj(&[
        ("schema", Value::string("excession.source-explanations.v1")),
        ("replayed_impact_report_id", report.get("report_id")?.clone()),
        ("count", Value::number(explanations.len())),
        ("source_inventory_complete", report.get("candidate_source_enumeration_complete")?.clone()),
        ("semantic_correctness", Value::string("NOT_EVALUATED")),
        ("coverage", report.get("coverage")?.clone()),
        ("explanations", Value::array(explanations)),
    ])
    .stamp_id("report_id")
}

pub fn audit_source(
    repo: &Path,
    saved_report: &Path,
    source_path: &str,
    source_commit: Option<&str>,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let explanation = explain_source(repo, saved_report, source_path, source_commit, roots, db)?;
    let _ = explanation.get("report_id")?;
    audit_result()
}

fn audit_result() -> Result<Value> {
    obj(&[
        ("schema", Value::string("excession.audit-result.v1")),
        ("schema_validation", Value::string("NOT_RUN")),
        ("git_authentication", Value::string("PASS")),
        ("replay", Value::string("PASS")),
        ("semantic_correctness", Value::string("NOT_EVALUATED")),
    ])
    .stamp_id("report_id")
}

pub fn explain(
    repo: &Path,
    saved_report: &Path,
    claim_id: &str,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let (report, effect) = checked_claim(repo, saved_report, claim_id, roots, db)?;
    let steps = effect.get("hops")?.clone();
    let hops = steps.array_ref()?;
    if hops.is_empty() {
        return refuse("impact claim has no evidence path");
    }
    let commit = hops[0].get("source")?.get("source_commit")?.str()?;
    for hop in hops {
        let a = hop.get("source")?.get("source_commit")?.str()?;
        let b = hop.get("target")?.get("source_commit")?.str()?;
        if a != commit || b != commit {
            return refuse("evidence path crosses revisions without an explicit transition");
        }
        if hop.get("evidence")?.array_ref()?.is_empty() {
            return refuse("claim explanation has no supporting observation");
        }
    }
    let out = obj(&[
        ("schema", Value::string("excession.explanation.v1")),
        ("claim_id", Value::string(claim_id)),
        ("source_commit", Value::string(commit)),
        ("steps", steps),
        ("coverage", report.get("coverage")?.clone()),
    ]);
    out.stamp_id("report_id")
}

pub fn audit(
    repo: &Path,
    saved_report: &Path,
    claim_id: &str,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let explanation = explain(repo, saved_report, claim_id, roots, db)?;
    // Source replay authenticates the candidate path, not runtime behavior.
    let _ = explanation.get("report_id")?;
    audit_result()
}

pub fn human(explanation: &Value) -> Result<String> {
    let mut lines = vec![format!(
        "Excession candidate explanation {} (commit {})",
        explanation.get("claim_id")?.str()?,
        explanation.get("source_commit")?.str()?,
    )];
    for (i, hop) in explanation.get("steps")?.array_ref()?.iter().enumerate() {
        lines.push(format!(
            "{}. {} -> {} ({})",
            i + 1,
            hop.get("source")?.get("path")?.str()?,
            hop.get("target")?.get("path")?.str()?,
            hop.get("relation")?.str()?,
        ));
        for evidence in hop.get("evidence")?.array_ref()? {
            lines.push(format!(
                "   receipt {}: {} / {}",
                evidence.get("receipt_id")?.str()?,
                evidence.get("producer")?.str()?,
                evidence.get("assessment")?.str()?,
            ));
        }
    }
    lines.push("Coverage: PARTIAL. Source replay does not prove semantic correctness.".to_owned());
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_claim_hash_tampering_and_ambiguous_ids() {
        let effect = obj(&[
            ("entity", Value::string("source")),
            ("assessment", Value::string("CANDIDATE_PATH")),
        ]);
        let mut correct = effect.clone();
        correct
            .entries_mut()
            .unwrap()
            .insert("claim_id".into(), Value::string(effect.sha256()));
        let report = obj(&[("effects", Value::array([correct.clone()]))]);
        assert!(find_claim(&report, &effect.sha256()).is_ok());
        correct
            .entries_mut()
            .unwrap()
            .insert("claim_id".into(), Value::string("a".repeat(64)));
        let poisoned = obj(&[("effects", Value::array([correct]))]);
        assert!(find_claim(&poisoned, &effect.sha256()).is_err());
    }
}
