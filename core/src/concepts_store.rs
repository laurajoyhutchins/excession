//! Immutable, independently auditable projections in the existing Excession SQLite store.
use crate::concepts;
use crate::json::{self, Value};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeSet;

const SNAPSHOT_SCHEMA: &str = include_str!("../../contracts/concepts/snapshot.v1.schema.json");
const SQL: &str = include_str!("concepts_schema.sql");

pub fn init(db: &Db) -> Result<()> {
    db.exec(SQL)
}

fn validate_manifest(model: &Value, manifest: &Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SNAPSHOT_SCHEMA).map_err(|e| format!("snapshot schema: {e}"))?;
    let document: serde_json::Value =
        serde_json::from_str(&manifest.canonical()).map_err(|e| format!("snapshot JSON: {e}"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("compile snapshot schema: {e}"))?;
    if let Err(e) = validator.validate(&document) {
        return refuse(format!("invalid snapshot manifest: {e}"));
    }
    if concepts::content_id(manifest, "snapshot_id")? != manifest.get("snapshot_id")?.str()? {
        return refuse("noncanonical immutable snapshot ID");
    }
    for key in ["model_id", "source_commit", "source_tree"] {
        if manifest.get(key)? != model.get(key)? {
            return refuse(format!("model and snapshot disagree on {key}"));
        }
    }
    let counts = manifest.get("record_counts")?;
    for (name, array) in [
        ("subjects", "subjects"),
        ("concepts", "concepts"),
        ("claims", "claims"),
    ] {
        if counts.get(name)?.integer()? != model.get(array)?.array_ref()?.len() as i64 {
            return refuse(format!("snapshot count mismatch: {name}"));
        }
    }
    let support_count: usize = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|x| x.get("support").and_then(Value::array_ref).map(|s| s.len()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum();
    if counts.get("support_links")?.integer()? != support_count as i64 {
        return refuse("snapshot support-link count mismatch");
    }
    if manifest.get("coverage_status")? != model.get("coverage")?.get("status")? {
        return refuse("model and snapshot disagree on coverage status");
    }
    let mut inputs = BTreeSet::new();
    for id in manifest.get("input_batch_ids")?.array_ref()? {
        inputs.insert(id.str()?);
    }
    if inputs.is_empty() {
        return refuse("snapshot must cite input batches");
    }
    Ok(())
}

fn existing(db: &Db, id: &str) -> Result<bool> {
    Ok(db.scalar(
        "SELECT count(*) FROM concept_snapshot WHERE snapshot_id=?",
        &[Param::Text(id)],
    )? != "0")
}

/// Publish only after semantic model validation and canonical manifest checks.
/// Replays source bindings separately; no stored digest constitutes attestation.
pub fn publish(db: &Db, model: &Value, manifest: &Value) -> Result<()> {
    concepts::validate_model(model)?;
    validate_manifest(model, manifest)?;
    init(db)?;
    let id = manifest.get("snapshot_id")?.str()?;
    if existing(db, id)? {
        return audit(db, id).and_then(|(saved, receipt)| {
            if saved == *model && receipt == *manifest {
                Ok(())
            } else {
                refuse("snapshot identity collides with different stored content")
            }
        });
    }
    db.transaction(|tx| {
        tx.execute(
            "INSERT INTO concept_snapshot VALUES (?,?,?,?,?,?)",
            &[
                Param::Text(id),
                Param::Text(model.get("model_id")?.str()?),
                Param::Text(model.get("source_commit")?.str()?),
                Param::Text(model.get("source_tree")?.str()?),
                Param::Text(&model.canonical()),
                Param::Text(&manifest.canonical()),
            ],
        )?;
        for subject in model.get("subjects")?.array_ref()? {
            tx.execute(
                "INSERT INTO concept_subject VALUES (?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(subject.get("subject_id")?.str()?),
                    Param::Text(&subject.canonical()),
                ],
            )?;
        }
        for claim in model.get("claims")?.array_ref()? {
            let claim_id = claim.get("claim_id")?.str()?;
            tx.execute(
                "INSERT INTO concept_claim VALUES (?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(claim_id),
                    Param::Text(&claim.canonical()),
                ],
            )?;
            for support in claim.get("support")?.array_ref()? {
                tx.execute(
                    "INSERT INTO concept_support VALUES (?,?,?,?,?,?)",
                    &[
                        Param::Text(id),
                        Param::Text(claim_id),
                        Param::Text(support.get("receipt_id")?.str()?),
                        Param::Text(
                            support
                                .get("observation_id")
                                .ok()
                                .map(Value::str)
                                .transpose()?
                                .unwrap_or(""),
                        ),
                        Param::Text(support.get("role")?.str()?),
                        Param::Text(support.get("observation_kind")?.str()?),
                    ],
                )?;
            }
        }
        for concept in model.get("concepts")?.array_ref()? {
            let concept_id = concept.get("concept_id")?.str()?;
            tx.execute(
                "INSERT INTO concept_definition VALUES (?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(concept_id),
                    Param::Text(concept.get("revision_id")?.str()?),
                    Param::Text(&concept.canonical()),
                ],
            )?;
            for membership in concept.get("membership_claims")?.array_ref()? {
                tx.execute(
                    "INSERT INTO concept_membership VALUES (?,?,?)",
                    &[
                        Param::Text(id),
                        Param::Text(concept_id),
                        Param::Text(membership.str()?),
                    ],
                )?;
            }
        }
        for receipt in model
            .get("coverage")?
            .get("producer_receipts")?
            .array_ref()?
        {
            tx.execute(
                "INSERT INTO concept_coverage_receipt VALUES (?,?)",
                &[Param::Text(id), Param::Text(receipt.str()?)],
            )?;
        }
        Ok(())
    })?;
    audit(db, id).map(|_| ())
}

fn match_rows(
    db: &Db,
    sql: &str,
    id: &str,
    mut expected: Vec<Vec<Option<String>>>,
    label: &str,
) -> Result<()> {
    let mut observed = db.query(sql, &[Param::Text(id)])?;
    observed.sort();
    expected.sort();
    if observed != expected {
        return refuse(format!(
            "semantic snapshot {label} projections differ from canonical model"
        ));
    }
    Ok(())
}

fn record(fields: &[String]) -> Vec<Option<String>> {
    fields.iter().map(|x| Some(x.clone())).collect()
}

/// Independently compare every queryable row against the stored canonical
/// model and verify both content hashes. Extra and missing rows fail closed.
pub fn audit(db: &Db, id: &str) -> Result<(Value, Value)> {
    init(db)?;
    let rows = db.query(
        "SELECT model_json, manifest_json FROM concept_snapshot WHERE snapshot_id=?",
        &[Param::Text(id)],
    )?;
    if rows.len() != 1 {
        return refuse("missing semantic snapshot or duplicate snapshot identity");
    }
    let body = rows[0][0].as_ref().ok_or("null semantic model")?;
    let receipt = rows[0][1].as_ref().ok_or("null semantic manifest")?;
    let model = json::parse(body.as_bytes())?;
    let manifest = json::parse(receipt.as_bytes())?;
    if manifest.get("snapshot_id")?.str()? != id {
        return refuse("stored snapshot identifier differs from requested identity");
    }
    concepts::validate_model(&model)?;
    validate_manifest(&model, &manifest)?;
    let subject_rows = model
        .get("subjects")?
        .array_ref()?
        .iter()
        .map(|x| {
            Ok(record(&[
                x.get("subject_id")?.str()?.to_owned(),
                x.canonical(),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    match_rows(
        db,
        "SELECT subject_id, canonical_json FROM concept_subject WHERE snapshot_id=?",
        id,
        subject_rows,
        "subject",
    )?;
    let claim_rows = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|x| {
            Ok(record(&[
                x.get("claim_id")?.str()?.to_owned(),
                x.canonical(),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    match_rows(
        db,
        "SELECT claim_id, canonical_json FROM concept_claim WHERE snapshot_id=?",
        id,
        claim_rows,
        "claim",
    )?;
    let mut support_rows = Vec::new();
    for claim in model.get("claims")?.array_ref()? {
        for x in claim.get("support")?.array_ref()? {
            support_rows.push(record(&[
                claim.get("claim_id")?.str()?.to_owned(),
                x.get("receipt_id")?.str()?.to_owned(),
                x.get("observation_id")
                    .ok()
                    .map(Value::str)
                    .transpose()?
                    .unwrap_or("")
                    .to_owned(),
                x.get("role")?.str()?.to_owned(),
                x.get("observation_kind")?.str()?.to_owned(),
            ]));
        }
    }
    match_rows(
        db,
        "SELECT claim_id,receipt_id,observation_id,role,kind FROM concept_support WHERE snapshot_id=?",
        id,
        support_rows,
        "support",
    )?;
    let concept_rows = model
        .get("concepts")?
        .array_ref()?
        .iter()
        .map(|x| {
            Ok(record(&[
                x.get("concept_id")?.str()?.to_owned(),
                x.get("revision_id")?.str()?.to_owned(),
                x.canonical(),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    match_rows(
        db,
        "SELECT concept_id,revision_id,canonical_json FROM concept_definition WHERE snapshot_id=?",
        id,
        concept_rows,
        "definition",
    )?;
    let mut member_rows = Vec::new();
    for concept in model.get("concepts")?.array_ref()? {
        for claim in concept.get("membership_claims")?.array_ref()? {
            member_rows.push(record(&[
                concept.get("concept_id")?.str()?.to_owned(),
                claim.str()?.to_owned(),
            ]));
        }
    }
    match_rows(
        db,
        "SELECT concept_id,claim_id FROM concept_membership WHERE snapshot_id=?",
        id,
        member_rows,
        "membership",
    )?;
    let receipts = model
        .get("coverage")?
        .get("producer_receipts")?
        .array_ref()?
        .iter()
        .map(|x| Ok(record(&[x.str()?.to_owned()])))
        .collect::<Result<Vec<_>>>()?;
    match_rows(
        db,
        "SELECT receipt_id FROM concept_coverage_receipt WHERE snapshot_id=?",
        id,
        receipts,
        "coverage",
    )?;
    db.foreign_keys_valid()?;
    Ok((model, manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::obj;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn sample() -> (Value, Value) {
        let model = concepts::seal(
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
        .unwrap();
        let manifest = concepts::seal(
            obj(&[
                ("schema", Value::string("excession.concepts-snapshot.v1")),
                ("model_id", model.get("model_id").unwrap().clone()),
                ("source_commit", model.get("source_commit").unwrap().clone()),
                ("source_tree", model.get("source_tree").unwrap().clone()),
                (
                    "input_batch_ids",
                    Value::array(vec![Value::string("c".repeat(64))]),
                ),
                ("compiler_version", Value::string("test")),
                ("rule_set_id", Value::string("d".repeat(64))),
                (
                    "record_counts",
                    obj(&[
                        ("subjects", Value::number(0)),
                        ("concepts", Value::number(0)),
                        ("claims", Value::number(0)),
                        ("support_links", Value::number(0)),
                    ]),
                ),
                ("coverage_status", Value::string("PARTIAL")),
            ]),
            "snapshot_id",
        )
        .unwrap();
        (model, manifest)
    }

    #[test]
    fn import_reopen_reimport_detects_injected_projection() {
        let path = std::env::temp_dir().join(format!(
            "excession-concepts-store-{}-{}.db",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let (model, manifest) = sample();
        let id = manifest
            .get("snapshot_id")
            .unwrap()
            .str()
            .unwrap()
            .to_owned();
        {
            let db = Db::open(&path).unwrap();
            publish(&db, &model, &manifest).unwrap();
            publish(&db, &model, &manifest).unwrap();
        }
        {
            let db = Db::open(&path).unwrap();
            assert_eq!(audit(&db, &id).unwrap(), (model, manifest));
            db.execute(
                "INSERT INTO concept_subject VALUES (?,?,?)",
                &[
                    Param::Text(&id),
                    Param::Text(&"e".repeat(64)),
                    Param::Text("{}"),
                ],
            )
            .unwrap();
            assert!(audit(&db, &id).is_err());
        }
        let _ = std::fs::remove_file(&path);
    }
}
