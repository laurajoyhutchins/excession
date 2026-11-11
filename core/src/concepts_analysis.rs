//! Additive concept context for existing refactoring hypotheses.
//! Never replace source facts, collapse predicates, or certify edits.
use crate::concepts;
use crate::concepts_store;
use crate::json::{Value, obj};
use crate::refactor;
use crate::sqlite::Db;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = include_str!("../../contracts/concepts/analysis-link.v1.schema.json");
const MAX_LINKS: usize = 128;

fn validate_shape(report: &Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|e| format!("concept analysis-link schema: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&report.canonical())
        .map_err(|e| format!("concept analysis-link JSON: {e}"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("compile concept analysis-link schema: {e}"))?;
    if let Err(e) = validator.validate(&value) {
        return refuse(format!("concept analysis-link contract violation: {e}"));
    }
    Ok(())
}

fn candidate_sources(candidate: &Value) -> Result<BTreeSet<(String, String)>> {
    let mut sources = BTreeSet::new();
    for key in ["modules", "symbols"] {
        if let Ok(elements) = candidate.get(key) {
            for item in elements.array_ref()? {
                sources.insert((
                    item.get("path")?.str()?.to_owned(),
                    item.get("blob")?.str()?.to_owned(),
                ));
            }
        }
    }
    Ok(sources)
}

/// Annotate a separately regenerated source-bound refactor report by matching
/// *both* exact tracked path/blob identities of an unverified concept proposal.
pub fn annotate_refactor(db: &Db, name: &str, revision: &str, snapshot_id: &str) -> Result<Value> {
    let (model, manifest) = concepts_store::audit(db, snapshot_id)?;
    let baseline = refactor::suggest(db, name, revision)?;
    if baseline.get("source_commit")? != model.get("source_commit")?
        || baseline.get("source_tree")? != model.get("source_tree")?
    {
        return refuse(
            "concept snapshot and refactoring report refer to different source revisions",
        );
    }
    let subjects = model
        .get("subjects")?
        .array_ref()?
        .iter()
        .map(|subject| {
            Ok((
                subject.get("subject_id")?.str()?.to_owned(),
                (
                    subject.get("path")?.str()?.to_owned(),
                    subject.get("blob")?.str()?.to_owned(),
                ),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let claims = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|claim| Ok((claim.get("claim_id")?.str()?.to_owned(), claim)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut annotations = Vec::new();
    for candidate in baseline.get("candidates")?.array_ref()? {
        let evidence = candidate_sources(candidate)?;
        if evidence.len() < 2 {
            continue;
        }
        let mut related_claims = BTreeSet::new();
        for concept in model.get("concepts")?.array_ref()? {
            let mut matched = BTreeSet::new();
            let mut membership_claims = BTreeSet::new();
            for membership in concept.get("membership_claims")?.array_ref()? {
                let id = membership.str()?;
                let claim = claims.get(id).ok_or("concept membership claim missing")?;
                if claim.get("predicate")?.str()? != "REALIZES_CONCEPT" {
                    return refuse("concept membership is not a realizes-concept claim");
                }
                for subject in claim.get("subjects")?.array_ref()? {
                    if let Some(identity) = subjects.get(subject.str()?)
                        && evidence.contains(identity)
                    {
                        matched.insert(identity.clone());
                        membership_claims.insert(id.to_owned());
                    }
                }
            }
            if matched.len() >= 2 {
                related_claims.extend(membership_claims);
            }
        }
        if related_claims.is_empty() {
            continue;
        }
        if annotations.len() >= MAX_LINKS {
            return refuse("concept annotation limit exceeded; narrow source roots");
        }
        annotations.push(obj(&[
            (
                "analysis_candidate_id",
                candidate.get("candidate_id")?.clone(),
            ),
            (
                "concept_claim_ids",
                Value::array(related_claims.into_iter().map(Value::string)),
            ),
            ("role", Value::string("CONCEPT_ANNOTATION")),
            ("assessment", Value::string("ANNOTATION_ONLY")),
        ]));
    }
    annotations.sort_by_key(Value::canonical);
    let mut result = obj(&[
        (
            "schema",
            Value::string("excession.concepts-analysis-link.v1"),
        ),
        ("source_commit", model.get("source_commit")?.clone()),
        ("snapshot_id", manifest.get("snapshot_id")?.clone()),
        ("analysis_report_id", baseline.get("report_id")?.clone()),
        ("analysis_kind", Value::string("REFACTOR_CANDIDATES")),
        ("links", Value::array(annotations)),
        ("coverage_status", Value::string("PARTIAL")),
    ]);
    result = concepts::seal(result, "link_report_id")?;
    validate_shape(&result)?;
    Ok(result)
}

/// An audit regenerates the original refactor source evidence and fully audits
/// the concept snapshot before comparing the complete annotation report.
pub fn audit_refactor(
    db: &Db,
    name: &str,
    revision: &str,
    snapshot_id: &str,
    saved: &Value,
) -> Result<Value> {
    if annotate_refactor(db, name, revision, snapshot_id)? != *saved {
        return refuse("concept analysis enrichment differs from regenerated evidence");
    }
    Ok(obj(&[
        ("link_report_id", saved.get("link_report_id")?.clone()),
        (
            "status",
            Value::string("ANNOTATION_REPLAYED_NO_VERIFICATION_PROMOTION"),
        ),
    ]))
}
