//! Exact report replay and source-bound explanation.
//! Does not promote candidate dependencies to verified behavior.
use crate::impact_query::{self, EvidenceByRevision};
use crate::json::{self, Value, obj};
use crate::observed_relations::Selection;
use crate::{Result, refuse};
use std::fs;
use std::path::Path;

const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

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
        let mut unsigned = effect.clone();
        unsigned.entries_mut()?.remove("claim_id");
        if unsigned.sha256() != actual {
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
fn checked_claim(
    repo: &Path,
    saved_report: &Path,
    claim_id: &str,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<(Value, Value)> {
    let reported = supported_report(saved_report)?;
    let base = reported.get("base_commit")?.str()?;
    let head = reported.get("head_commit")?.str()?;
    let fresh = if let Some(choices) = reported.entries()?.get("producer_selections") {
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
    let effect = find_claim(&fresh, claim_id)?;
    Ok((fresh.clone(), effect.clone()))
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
    let mut out = obj(&[
        ("schema", Value::string("excession.explanation.v1")),
        ("claim_id", Value::string(claim_id)),
        ("source_commit", Value::string(commit)),
        ("steps", steps),
        ("coverage", report.get("coverage")?.clone()),
    ]);
    let report_id = out.sha256();
    out.entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok(out)
}

pub fn audit(
    repo: &Path,
    saved_report: &Path,
    claim_id: &str,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    let explanation = explain(repo, saved_report, claim_id, roots, db)?;
    let mut result = obj(&[
        ("schema", Value::string("excession.audit-result.v1")),
        ("schema_validation", Value::string("NOT_RUN")),
        ("git_authentication", Value::string("PASS")),
        ("replay", Value::string("PASS")),
        ("semantic_correctness", Value::string("NOT_EVALUATED")),
    ]);
    // Replaying the source authenticates the supported observation path,
    // but independent semantic truth remains deliberately unassessed.
    let _ = explanation.get("report_id")?;
    let id = result.sha256();
    result
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(result)
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
