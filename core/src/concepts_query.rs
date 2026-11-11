//! Bounded read-only concept queries, never a second SQL query authority.
use crate::concepts;
use crate::concepts_store;
use crate::json::{Value, obj};
use crate::sqlite::Db;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const QUERY_SCHEMA: &str = include_str!("../../contracts/concepts/query.v1.schema.json");
const ANSWER_SCHEMA: &str = include_str!("../../contracts/concepts/answer.v1.schema.json");
const MAX_QUERY_BYTES: u64 = 256 * 1024;

fn wire(value: &Value, text: &str) -> Result<()> {
    let schema: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let input: serde_json::Value =
        serde_json::from_str(&value.canonical()).map_err(|e| e.to_string())?;
    let validator =
        jsonschema::draft202012::new(&schema).map_err(|e| format!("concept query schema: {e}"))?;
    if let Err(e) = validator.validate(&input) {
        return refuse(format!("concept query wire contract: {e}"));
    }
    Ok(())
}

pub fn run(db: &Db, query: &Value) -> Result<Value> {
    wire(query, QUERY_SCHEMA)?;
    if concepts::content_id(query, "query_id")? != query.get("query_id")?.str()? {
        return refuse("noncanonical concept query ID");
    }
    let id = query.get("snapshot_id")?.str()?;
    let (model, _) = concepts_store::audit(db, id)?;
    let operation = query.get("operation")?.str()?;
    let selector = query.get("selector")?;
    let max_results = usize::try_from(query.get("limits")?.get("max_results")?.integer()?)
        .map_err(|_| "invalid query limit")?;
    let max_depth = query.get("limits")?.get("max_depth")?.integer()?;
    let mut selected = Vec::<Value>::new();
    let mut concept_ids = BTreeSet::<String>::new();
    let mut subject_ids = BTreeSet::<String>::new();
    let mut explanation = String::new();
    let mut found = false;
    let concepts_by_id = model
        .get("concepts")?
        .array_ref()?
        .iter()
        .map(|c| Ok((c.get("concept_id")?.str()?.to_owned(), c)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let claims_by_id = model
        .get("claims")?
        .array_ref()?
        .iter()
        .map(|c| Ok((c.get("claim_id")?.str()?.to_owned(), c)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    match operation {
        "FIND_CONCEPTS" => {
            let term = selector.get("term")?.str()?.to_lowercase();
            for (id, concept) in &concepts_by_id {
                if concept.get("name")?.str()?.to_lowercase().contains(&term) {
                    concept_ids.insert(id.clone());
                }
            }
            found = !concept_ids.is_empty();
            explanation = "Candidate concept names are navigational labels, not proof of semantic equivalence.".into();
        }
        "LIST_MEMBERS" => {
            let key = selector.get("concept_id")?.str()?;
            if let Some(concept) = concepts_by_id.get(key) {
                found = true;
                concept_ids.insert(key.into());
                for item in concept.get("membership_claims")?.array_ref()? {
                    let claim_id = item.str()?;
                    let claim = claims_by_id
                        .get(claim_id)
                        .ok_or("concept membership references missing claim")?;
                    selected.push((**claim).clone());
                    for subject in claim.get("subjects")?.array_ref()? {
                        subject_ids.insert(subject.str()?.to_owned());
                    }
                }
                explanation =
                    "Membership is a proposal until independently reviewed and verified.".into();
            }
        }
        "EXPLAIN_CLAIM" => {
            let key = selector.get("claim_id")?.str()?;
            if let Some(claim) = claims_by_id.get(key) {
                found = true;
                selected.push((**claim).clone());
                for subject in claim.get("subjects")?.array_ref()? {
                    subject_ids.insert(subject.str()?.to_owned());
                }
                explanation = "The returned claim preserves its predicate, support and behavioral verification status.".into();
            }
        }
        "TRAVERSE_CLAIMS" => {
            // A direct local relationship lookup only; inferred transitive
            // closure is unsupported until predicate-specific algebra exists.
            if max_depth != 1 {
                return refuse("concept traversal currently supports only max_depth=1");
            }
            let key = selector.get("subject_id")?.str()?;
            for claim in claims_by_id.values() {
                if claim
                    .get("subjects")?
                    .array_ref()?
                    .iter()
                    .any(|x| x.str().ok() == Some(key))
                {
                    found = true;
                    selected.push((**claim).clone());
                    for subject in claim.get("subjects")?.array_ref()? {
                        subject_ids.insert(subject.str()?.to_owned());
                    }
                }
            }
            explanation =
                "Direct claims only. Traversal does not create transitive dependencies.".into();
        }
        "ANSWERABILITY" => {
            explanation = "Only bounded, source-bound candidate concepts are supported; no closed-world or behavioral closure assertion is available.".into();
        }
        _ => return refuse("unsupported concept query operation"),
    }
    if selected.len() > max_results || concept_ids.len() > max_results {
        return refuse("concept query exceeds explicit result limit");
    }
    selected.sort_by_key(Value::canonical);
    let mut claims = BTreeSet::new();
    let mut evidence = BTreeSet::new();
    for claim in &selected {
        claims.insert(claim.get("claim_id")?.str()?.to_owned());
        for support in claim.get("support")?.array_ref()? {
            evidence.insert(support.get("receipt_id")?.str()?.to_owned());
        }
    }
    let status = if operation == "FIND_CONCEPTS" && concept_ids.len() > 1 {
        "AMBIGUOUS"
    } else if found {
        // The source corpus is only candidate-complete.
        "PARTIAL"
    } else if operation == "ANSWERABILITY" {
        "UNSUPPORTED"
    } else {
        "UNKNOWN"
    };
    let gaps = model
        .get("coverage")?
        .get("gaps")?
        .array_ref()?
        .iter()
        .map(|gap| {
            let reason = gap.get("reason")?.str()?;
            let suffix = gap
                .get("source_path")
                .ok()
                .and_then(|p| p.str().ok())
                .map(|p| format!(" at {p}"))
                .unwrap_or_default();
            Ok(obj(&[(
                "reason",
                Value::string(format!("{reason}{suffix}")),
            )]))
        })
        .collect::<Result<Vec<_>>>()?;
    let response = obj(&[
        ("schema", Value::string("excession.concepts-answer.v1")),
        ("query_id", query.get("query_id")?.clone()),
        ("snapshot_id", query.get("snapshot_id")?.clone()),
        ("source_commit", model.get("source_commit")?.clone()),
        ("answerability", Value::string(status)),
        (
            "concept_ids",
            Value::array(concept_ids.into_iter().map(Value::string)),
        ),
        (
            "subject_ids",
            Value::array(subject_ids.into_iter().map(Value::string)),
        ),
        (
            "claim_ids",
            Value::array(claims.into_iter().map(Value::string)),
        ),
        ("claims", Value::array(selected)),
        (
            "evidence_receipts",
            Value::array(evidence.into_iter().map(Value::string)),
        ),
        ("gaps", Value::array(gaps)),
        ("limits_applied", Value::Bool(false)),
        ("explanation", Value::string(explanation)),
    ]);
    wire(&response, ANSWER_SCHEMA)?;
    Ok(response)
}

pub fn run_file(db: &Db, path: &Path) -> Result<Value> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_QUERY_BYTES {
        return refuse("concept query JSON exceeds byte limit");
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    run(db, &crate::json::parse(&bytes)?)
}
