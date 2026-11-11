//! Append-only, explicitly trusted-local interpretation reviews.
//! Review events are not behavioral proofs, identity attestations, or source edits.
use crate::concepts;
use crate::concepts_store;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::path::Path;

const SCHEMA: &str = include_str!("../../contracts/concepts/review-event.v1.schema.json");
const SQL: &str = include_str!("concepts_review.sql");

fn validate_event(event: &Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|e| format!("review schema: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&event.canonical()).map_err(|e| format!("review JSON: {e}"))?;
    let validator =
        jsonschema::draft202012::new(&schema).map_err(|e| format!("compile review schema: {e}"))?;
    if let Err(e) = validator.validate(&value) {
        return refuse(format!("review event violates schema: {e}"));
    }
    if concepts::content_id(event, "event_id")? != event.get("event_id")?.str()? {
        return refuse("review event ID differs from canonical contents");
    }
    Ok(())
}

fn authenticate_scope(db: &Db, event: &Value) -> Result<()> {
    let scope = event.get("scope")?;
    let snapshot = scope.get("snapshot_id")?.str()?;
    let (model, _) = concepts_store::audit(db, snapshot)?;
    if scope.get("source_commit")? != model.get("source_commit")?
        || scope.get("model_id")? != model.get("model_id")?
    {
        return refuse("review event source or model scope does not match published snapshot");
    }
    let id = event.get("concept_id")?.str()?;
    let version = event.get("concept_revision_id")?.str()?;
    let mut found = false;
    let mut concept_ids = BTreeSet::new();
    for concept in model.get("concepts")?.array_ref()? {
        let name = concept.get("concept_id")?.str()?;
        concept_ids.insert(name.to_owned());
        if name == id && concept.get("revision_id")?.str()? == version {
            found = true;
        }
    }
    if !found {
        return refuse("review target does not name the exact published concept revision");
    }
    let related = event
        .get("related_concept_ids")
        .ok()
        .and_then(|x| x.array_ref().ok());
    if matches!(event.get("action")?.str()?, "MERGE" | "SPLIT")
        && related.is_none_or(|x| x.is_empty())
    {
        return refuse("merge/split review requires named related concepts");
    }
    if let Some(related) = related {
        for other in related {
            let other = other.str()?;
            if other == id || !concept_ids.contains(other) {
                return refuse("review references an absent or identical related concept");
            }
        }
    }
    let receipts = model
        .get("coverage")?
        .get("producer_receipts")?
        .array_ref()?
        .iter()
        .map(Value::str)
        .collect::<Result<BTreeSet<_>>>()?;
    for evidence in event.get("evidence_receipts")?.array_ref()? {
        if !receipts.contains(evidence.str()?) {
            return refuse("review claims evidence absent from original snapshot");
        }
    }
    Ok(())
}

/// An explicit trusted-local operation, not an authority escalation.
/// The reviewer field is a declared identity, not independently authenticated.
pub fn append(db: &Db, event: &Value, trusted_local: bool) -> Result<()> {
    if !trusted_local {
        return refuse("concept review requires explicit trusted-local approval");
    }
    validate_event(event)?;
    concepts_store::init(db)?;
    db.exec(SQL)?;
    authenticate_scope(db, event)?;
    let id = event.get("event_id")?.str()?;
    let existing = db.query(
        "SELECT canonical_json FROM concept_review_event WHERE event_id=?",
        &[Param::Text(id)],
    )?;
    if !existing.is_empty() {
        if existing.len() == 1 && existing[0][0].as_deref() == Some(event.canonical().as_str()) {
            return Ok(());
        }
        return refuse("conflicting review event shares the same event identity");
    }
    db.transaction(|tx| {
        tx.execute(
            "INSERT INTO concept_review_event VALUES (?,?,?,?,?)",
            &[
                Param::Text(id),
                Param::Text(event.get("scope")?.get("snapshot_id")?.str()?),
                Param::Text(event.get("concept_id")?.str()?),
                Param::Text(event.get("concept_revision_id")?.str()?),
                Param::Text(&event.canonical()),
            ],
        )
    })
}

pub fn list(db: &Db, snapshot: &str, concept: &str) -> Result<Value> {
    concepts_store::audit(db, snapshot)?;
    db.exec(SQL)?;
    let mut result = Vec::new();
    for row in db.query(
        "SELECT event_id,canonical_json FROM concept_review_event WHERE snapshot_id=? AND concept_id=? ORDER BY event_id",
        &[Param::Text(snapshot), Param::Text(concept)]
    )? {
        let id = row[0].as_ref().ok_or("null review ID")?;
        let body = row[1].as_ref().ok_or("null review contents")?;
        let event = json::parse(body.as_bytes())?;
        validate_event(&event)?;
        authenticate_scope(db, &event)?;
        if event.get("event_id")?.str()? != id {
            return refuse("review log event differs from its primary key");
        }
        result.push(event);
    }
    Ok(obj(&[
        ("snapshot_id", Value::string(snapshot)),
        ("concept_id", Value::string(concept)),
        ("events", Value::array(result)),
        ("behavioral_verification_inferred", Value::Bool(false)),
    ]))
}

pub fn append_file(db: &Db, file: &Path, trusted_local: bool) -> Result<Value> {
    if std::fs::metadata(file).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return refuse("review event exceeds byte limit");
    }
    let data = std::fs::read(file).map_err(|e| e.to_string())?;
    let event = json::parse(&data)?;
    append(db, &event, trusted_local)?;
    Ok(obj(&[
        ("event_id", event.get("event_id")?.clone()),
        ("status", Value::string("REVIEW_EVENT_RETAINED")),
        ("behavioral_verification_inferred", Value::Bool(false)),
    ]))
}
