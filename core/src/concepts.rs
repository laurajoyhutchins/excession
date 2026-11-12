//! Versioned semantic concept model validation.
//! A valid schema is not an authenticated Git observation or behavioral proof.
use crate::json::Value;
use crate::{Result, refuse};
use std::collections::BTreeSet;

const SCHEMA: &str = include_str!("../../contracts/concepts/model.v1.schema.json");

/// Validate the bounded public wire shape; this does not authenticate the source.
pub fn validate_shape(model: &Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|e| format!("concept schema: {e}"))?;
    let instance: serde_json::Value =
        serde_json::from_str(&model.canonical()).map_err(|e| format!("concept document: {e}"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("compile concepts schema: {e}"))?;
    if let Err(e) = validator.validate(&instance) {
        return refuse(format!("invalid concepts v1 model: {e}"));
    }
    Ok(())
}

/// The canonical digest excludes only the designated content identifier.
/// Domain separation is achieved by distinct schema fields and object shapes.
pub fn content_id(value: &Value, id_field: &str) -> Result<String> {
    let mut body = value.clone();
    body.entries_mut()?.remove(id_field);
    Ok(body.sha256())
}

pub fn seal(mut value: Value, id_field: &str) -> Result<Value> {
    let digest = content_id(&value, id_field)?;
    value
        .entries_mut()?
        .insert(id_field.into(), Value::string(digest));
    Ok(value)
}

fn check_id(value: &Value, field: &str) -> Result<()> {
    let expected = content_id(value, field)?;
    if value.get(field)?.str()? != expected {
        return refuse(format!("noncanonical concepts {field}"));
    }
    Ok(())
}

/// Validate canonical internal references, evidence and non-promotion rules.
/// Git source replay is a separate, required boundary at ingestion/audit.
pub fn validate_model(model: &Value) -> Result<()> {
    validate_shape(model)?;
    check_id(model, "model_id")?;
    let commit = model.get("source_commit")?.str()?;
    let mut subjects = BTreeSet::new();
    for subject in model.get("subjects")?.array_ref()? {
        check_id(subject, "subject_id")?;
        if subject.get("source_commit")?.str()? != commit {
            return refuse("subject belongs to a different source revision");
        }
        let id = subject.get("subject_id")?.str()?;
        if !subjects.insert(id.to_owned()) {
            return refuse("duplicate subject identity");
        }
    }
    let mut receipts = BTreeSet::new();
    for receipt in model
        .get("coverage")?
        .get("producer_receipts")?
        .array_ref()?
    {
        if !receipts.insert(receipt.str()?.to_owned()) {
            return refuse("duplicate coverage receipt");
        }
    }
    let mut claim_ids = BTreeSet::new();
    for claim in model.get("claims")?.array_ref()? {
        check_id(claim, "claim_id")?;
        let id = claim.get("claim_id")?.str()?;
        if !claim_ids.insert(id.to_owned()) {
            return refuse("duplicate semantic claim");
        }
        for subject in claim.get("subjects")?.array_ref()? {
            if !subjects.contains(subject.str()?) {
                return refuse("claim references missing subject");
            }
        }
        let mut native_proof = false;
        for support in claim.get("support")?.array_ref()? {
            let receipt = support.get("receipt_id")?.str()?;
            if !receipts.contains(receipt) {
                return refuse("claim support is absent from source coverage manifest");
            }
            if matches!(
                support.get("observation_kind")?.str()?,
                "STATIC_IMPORT_CANDIDATE" | "SCIP_REFERENCE"
            ) && support.get("observation_id").is_err()
            {
                return refuse("static import support must identify its source observation");
            }
            if support.get("role")?.str()? == "SUPPORTS"
                && support.get("observation_kind")?.str()? == "NATIVE_VERIFICATION"
            {
                native_proof = true;
            }
        }
        let predicate = claim.get("predicate")?.str()?;
        let verified = claim.get("behavioral_verification")?.str()? == "VERIFIED_UNDER_CONTRACT";
        if verified && !native_proof {
            return refuse("behavioral verification requires independent native evidence");
        }
        if predicate == "BEHAVIORALLY_EQUIVALENT_UNDER" && !verified {
            return refuse("behavioral equivalence cannot be a proposed or untested claim");
        }
    }
    let mut concepts = BTreeSet::new();
    let mut revisions = BTreeSet::new();
    for concept in model.get("concepts")?.array_ref()? {
        // concept_id is a durable assigned identity, not content-derived.
        let id = concept.get("concept_id")?.str()?;
        if !concepts.insert(id.to_owned()) {
            return refuse("duplicate concept identity");
        }
        if !revisions.insert(concept.get("revision_id")?.str()?.to_owned()) {
            return refuse("duplicate concept revision");
        }
        for membership in concept.get("membership_claims")?.array_ref()? {
            if !claim_ids.contains(membership.str()?) {
                return refuse("concept references missing membership claim");
            }
        }
    }
    for claim in model.get("claims")?.array_ref()? {
        if claim.get("predicate")?.str()? == "REALIZES_CONCEPT" {
            let concept_id = claim.get("concept_id")?.str()?;
            if !concepts.contains(concept_id) {
                return refuse("claim references missing concept");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::obj;

    fn minimal_model() -> Value {
        seal(
            obj(&[
                ("schema", Value::string("excession.concepts-model.v1")),
                ("source_commit", Value::string("a".repeat(40))),
                ("source_tree", Value::string("b".repeat(40))),
                ("subjects", Value::array(vec![])),
                ("claims", Value::array(vec![])),
                ("concepts", Value::array(vec![])),
                (
                    "coverage",
                    obj(&[
                        ("status", Value::string("PARTIAL")),
                        ("producer_receipts", Value::array(vec![])),
                        (
                            "gaps",
                            Value::array(vec![obj(&[(
                                "reason",
                                Value::string("UNKNOWN_COVERAGE"),
                            )])]),
                        ),
                    ]),
                ),
            ]),
            "model_id",
        )
        .expect("seal")
    }

    #[test]
    fn canonical_minimal_model_is_accepted() {
        validate_model(&minimal_model()).unwrap();
    }

    #[test]
    fn changing_any_published_field_invalidates_model_id() {
        let mut model = minimal_model();
        model
            .entries_mut()
            .unwrap()
            .insert("source_tree".into(), Value::string("c".repeat(40)));
        assert!(validate_model(&model).is_err());
    }

    #[test]
    fn cannot_invent_an_unrecognized_predicate() {
        let mut model = minimal_model();
        let invalid_claim = obj(&[
            ("claim_id", Value::string("d".repeat(64))),
            ("predicate", Value::string("RELATED_TO")),
            ("subjects", Value::array(vec![])),
            ("assessment", Value::string("PROPOSED")),
            ("behavioral_verification", Value::string("UNTESTED")),
            ("support", Value::array(vec![])),
        ]);
        model
            .entries_mut()
            .unwrap()
            .insert("claims".into(), Value::array(vec![invalid_claim]));
        let model = seal(model, "model_id").unwrap();
        assert!(validate_model(&model).is_err());
    }
}
