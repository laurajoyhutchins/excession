//! Deterministic, deliberately narrow architectural hypotheses from source-bound
//! native observations. A reciprocal static import is neither a runtime cycle
//! nor a proof that the two modules share responsibility.
use crate::concepts;
use crate::concepts_normalize;
use crate::json::{Value, obj};
use crate::{Result, digest, refuse};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CANDIDATES: usize = 128;
const COMPILER: &str = "excession.concepts-source-candidates.v2";
const RULE: &str = "reciprocal-static-import.v1;reciprocal-scip-reference.v1";

fn support(observation: &Value) -> Result<Value> {
    Ok(obj(&[
        ("receipt_id", observation.get("receipt_id")?.clone()),
        (
            "observation_id",
            observation.get("source_observation_id")?.clone(),
        ),
        ("role", Value::string("SUPPORTS")),
        ("observation_kind", observation.get("kind")?.clone()),
    ]))
}

fn claim(
    predicate: &str,
    subjects: Vec<String>,
    concept_id: Option<&str>,
    support: &[Value],
) -> Result<Value> {
    let mut fields = obj(&[
        ("predicate", Value::string(predicate)),
        (
            "subjects",
            Value::array(subjects.into_iter().map(Value::string)),
        ),
        ("assessment", Value::string("PROPOSED")),
        ("behavioral_verification", Value::string("UNTESTED")),
        ("support", Value::array(support.to_vec())),
    ]);
    if let Some(id) = concept_id {
        fields
            .entries_mut()?
            .insert("concept_id".into(), Value::string(id));
    }
    concepts::seal(fields, "claim_id")
}

/// Compile only a hypothesis supported by two separately observed reciprocal
/// static import candidates. Source/target identities are never merged.
pub fn compile(batch: &Value) -> Result<Value> {
    concepts_normalize::validate_batch(batch)?;
    let mut subjects = BTreeMap::new();
    for value in batch.get("subjects")?.array_ref()? {
        subjects.insert(value.get("subject_id")?.str()?.to_owned(), value.clone());
    }
    let mut edges = BTreeMap::<(String, String, String), Vec<(String, Value)>>::new();
    let mut receipt_ids = BTreeSet::new();
    for input in batch.get("inputs")?.array_ref()? {
        receipt_ids.insert(input.get("receipt_id")?.str()?.to_owned());
    }
    for observation in batch.get("observations")?.array_ref()? {
        let kind = observation.get("kind")?.str()?;
        let status = observation.get("status")?.str()?;
        if !matches!(
            (kind, status),
            ("STATIC_IMPORT_CANDIDATE", "CANDIDATE") | ("SCIP_REFERENCE", "OBSERVED")
        ) {
            continue;
        }
        let source_id = observation.get("source_subject_id")?.str()?;
        let target_id = observation.get("target_subject_id")?.str()?;
        let from = subjects
            .get(source_id)
            .ok_or("missing normalized source")?
            .get("path")?
            .str()?
            .to_owned();
        let to = subjects
            .get(target_id)
            .ok_or("missing normalized target")?
            .get("path")?
            .str()?
            .to_owned();
        if from == to {
            continue;
        }
        edges
            .entry((kind.to_owned(), from, to))
            .or_default()
            .push((source_id.to_owned(), observation.clone()));
    }

    let mut concepts_out = Vec::new();
    let mut claims = Vec::new();
    let mut seen_pairs = BTreeSet::new();
    for ((kind, left, right), forward) in &edges {
        if left >= right || !edges.contains_key(&(kind.clone(), right.clone(), left.clone())) {
            continue;
        }
        if !seen_pairs.insert((left.clone(), right.clone())) {
            return refuse("duplicate reciprocal import pair");
        }
        if concepts_out.len() >= MAX_CANDIDATES {
            return refuse("semantic concept candidate limit exceeded");
        }
        let backward = &edges[&(kind.clone(), right.clone(), left.clone())];
        let (left_id, left_observation) = &forward[0];
        let (right_id, right_observation) = &backward[0];
        let support = [support(left_observation)?, support(right_observation)?];
        // A candidate identity is scoped to the current revision; only review can
        // establish a durable concept identity across later source revisions.
        let concept_id = digest::sha256(
            format!(
                "excession.concept-candidate.v2\0{}\0{}\0{}\0{}",
                batch.get("source_commit")?.str()?,
                kind,
                left,
                right
            )
            .as_bytes(),
        );
        let a = claim(
            "REALIZES_CONCEPT",
            vec![left_id.clone()],
            Some(&concept_id),
            &support,
        )?;
        let b = claim(
            "REALIZES_CONCEPT",
            vec![right_id.clone()],
            Some(&concept_id),
            &support,
        )?;
        let relation = claim(
            "POSSIBLY_SAME_RESPONSIBILITY",
            vec![left_id.clone(), right_id.clone()],
            None,
            &support,
        )?;
        let member_a = a.get("claim_id")?.clone();
        let member_b = b.get("claim_id")?.clone();
        claims.extend([a, b, relation]);
        let concept = concepts::seal(
            obj(&[
                ("concept_id", Value::string(&concept_id)),
                (
                    "name",
                    Value::string(format!("Possible source boundary: {left} / {right}")),
                ),
                (
                    "definition",
                    Value::string(
                        if kind == "SCIP_REFERENCE" {
                            "Proposed architectural responsibility from reciprocal SCIP symbol references; not a compiler import cycle, call graph, FFI bridge, or behavioral equivalence."
                        } else {
                            "Proposed architectural responsibility from reciprocal static import candidates; not a verified import cycle, interface implementation, or behavioral equivalence."
                        },
                    ),
                ),
                ("membership_claims", Value::array(vec![member_a, member_b])),
                ("review_status", Value::string("UNREVIEWED")),
                ("aliases", Value::array(vec![])),
            ]),
            "revision_id",
        )?;
        concepts_out.push(concept);
    }
    concepts_out.sort_by_key(Value::canonical);
    claims.sort_by_key(Value::canonical);

    let mut gaps = Vec::new();
    for gap in batch.get("gaps")?.array_ref()? {
        let reason = match gap.get("reason")?.str()? {
            "RESOURCE_LIMIT" => "RESOURCE_LIMIT",
            "MISSING_PRODUCER" => "MISSING_INDEXER",
            "UNRESOLVED_SYMBOL" => "UNRESOLVED_SYMBOL",
            "UNSUPPORTED_SOURCE" => "UNSUPPORTED_LANGUAGE",
            _ => "UNKNOWN_COVERAGE",
        };
        let mut mapped = obj(&[("reason", Value::string(reason))]);
        if let Ok(path) = gap.get("source_path") {
            mapped
                .entries_mut()?
                .insert("source_path".into(), path.clone());
        }
        gaps.push(mapped);
    }
    // Candidate-only static observations never establish closed-world coverage.
    if gaps.is_empty() {
        gaps.push(obj(&[("reason", Value::string("UNKNOWN_COVERAGE"))]));
    }
    let model = concepts::seal(
        obj(&[
            ("schema", Value::string("excession.concepts-model.v1")),
            ("source_commit", batch.get("source_commit")?.clone()),
            ("source_tree", batch.get("source_tree")?.clone()),
            ("subjects", Value::array(subjects.into_values())),
            ("claims", Value::array(claims)),
            ("concepts", Value::array(concepts_out)),
            (
                "coverage",
                obj(&[
                    ("status", Value::string("PARTIAL")),
                    (
                        "producer_receipts",
                        Value::array(receipt_ids.into_iter().map(Value::string)),
                    ),
                    ("gaps", Value::array(gaps)),
                ]),
            ),
        ]),
        "model_id",
    )?;
    concepts::validate_model(&model)?;
    Ok(model)
}

pub fn manifest(batch: &Value, model: &Value) -> Result<Value> {
    concepts_normalize::validate_batch(batch)?;
    concepts::validate_model(model)?;
    if batch.get("source_commit")? != model.get("source_commit")?
        || batch.get("source_tree")? != model.get("source_tree")?
    {
        return refuse("compiler model differs from input Git revision");
    }
    let support_links = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|c| c.get("support").and_then(Value::array_ref).map(|s| s.len()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>();
    concepts::seal(
        obj(&[
            ("schema", Value::string("excession.concepts-snapshot.v1")),
            ("model_id", model.get("model_id")?.clone()),
            ("source_commit", model.get("source_commit")?.clone()),
            ("source_tree", model.get("source_tree")?.clone()),
            (
                "input_batch_ids",
                Value::array(vec![batch.get("batch_id")?.clone()]),
            ),
            ("compiler_version", Value::string(COMPILER)),
            (
                "rule_set_id",
                Value::string(digest::sha256(RULE.as_bytes())),
            ),
            (
                "record_counts",
                obj(&[
                    (
                        "subjects",
                        Value::number(model.get("subjects")?.array_ref()?.len()),
                    ),
                    (
                        "concepts",
                        Value::number(model.get("concepts")?.array_ref()?.len()),
                    ),
                    (
                        "claims",
                        Value::number(model.get("claims")?.array_ref()?.len()),
                    ),
                    ("support_links", Value::number(support_links)),
                ]),
            ),
            ("coverage_status", Value::string("PARTIAL")),
        ]),
        "snapshot_id",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_candidate_from_empty_evidence() {
        let batch = concepts::seal(
            obj(&[
                (
                    "schema",
                    Value::string("excession.concepts-observation-batch.v1"),
                ),
                ("source_commit", Value::string("a".repeat(40))),
                ("source_tree", Value::string("b".repeat(40))),
                ("completeness", Value::string("PARTIAL_CANDIDATES_ONLY")),
                ("subjects", Value::array(vec![])),
                (
                    "inputs",
                    Value::array(vec![obj(&[
                        ("receipt_id", Value::string("c".repeat(64))),
                        ("producer", Value::string("test")),
                        ("version", Value::string("1")),
                        ("kind", Value::string("STATIC_SYNTAX")),
                        ("scope", Value::array(vec![])),
                    ])]),
                ),
                ("observations", Value::array(vec![])),
                ("gaps", Value::array(vec![])),
            ]),
            "batch_id",
        )
        .unwrap();
        let model = compile(&batch).unwrap();
        assert!(model.get("claims").unwrap().array_ref().unwrap().is_empty());
        assert!(
            model
                .get("concepts")
                .unwrap()
                .array_ref()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            model
                .get("coverage")
                .unwrap()
                .get("status")
                .unwrap()
                .str()
                .unwrap(),
            "PARTIAL"
        );
        let receipt = manifest(&batch, &model).unwrap();
        assert_eq!(
            receipt
                .get("record_counts")
                .unwrap()
                .get("claims")
                .unwrap()
                .integer()
                .unwrap(),
            0
        );
    }
}
