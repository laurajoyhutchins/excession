//! LLM-authored, Git-bound architectural proposals with no semantic promotion.
//!
//! The LLM authors concepts. Rust provides bounded source evidence, validates
//! exact SCIP observation membership and stores only UNREVIEWED interpretations.
use crate::concepts;
use crate::concepts_compile;
use crate::concepts_normalize;
use crate::concepts_scip;
use crate::concepts_store;
use crate::digest;
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::Db;
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SCHEMA: &str = include_str!("../../contracts/concepts/ontology-proposal.v1.schema.json");
const MAX_DRAFT_BYTES: u64 = 256 * 1024;
const MAX_CONTEXT_OBSERVATIONS: usize = 40;
const MAX_SNIPPET_LINES: usize = 40;
const SOURCE_BYTES: usize = 512 * 1024;
const COMPILER: &str = "excession.llm-ontology-authoring.v1";

fn validate_draft(draft: &Value, batch: &Value) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|e| format!("ontology proposal schema: {e}"))?;
    let instance: serde_json::Value = serde_json::from_str(&draft.canonical())
        .map_err(|e| format!("ontology draft JSON: {e}"))?;
    let validator = jsonschema::draft202012::new(&schema)
        .map_err(|e| format!("compile ontology proposal schema: {e}"))?;
    if let Err(e) = validator.validate(&instance) {
        return refuse(format!("ontology proposal contract: {e}"));
    }
    for (draft_key, batch_key) in [
        ("source_commit", "source_commit"),
        ("source_tree", "source_tree"),
        ("batch_id", "batch_id"),
    ] {
        if draft.get(draft_key)? != batch.get(batch_key)? {
            return refuse(format!(
                "ontology proposal does not match pinned {draft_key}"
            ));
        }
    }
    Ok(())
}

pub fn read_draft(path: &Path) -> Result<Value> {
    if std::fs::metadata(path)
        .map_err(|e| format!("ontology draft metadata: {e}"))?
        .len()
        > MAX_DRAFT_BYTES
    {
        return refuse("ontology proposal exceeds 256 KiB");
    }
    let bytes = std::fs::read(path).map_err(|e| format!("ontology draft read: {e}"))?;
    json::parse(&bytes)
}

/// Canonical *file* subjects, rather than accidental per-occurrence file spans.
/// Original observation identities remain available to independently audit support.
fn files(batch: &Value) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    for subject in batch.get("subjects")?.array_ref()? {
        if subject.get("kind")?.str()? != "SOURCE_FILE" {
            continue;
        }
        let path = subject.get("path")?.str()?.to_owned();
        git::validate_path(&path)?;
        let mut value = subject.clone();
        value.entries_mut()?.remove("span");
        value.entries_mut()?.remove("subject_id");
        let value = concepts::seal(value, "subject_id")?;
        if let Some(old) = result.insert(path.clone(), value.clone())
            && old != value
        {
            return refuse(format!("ambiguous SCIP file identity: {path}"));
        }
    }
    Ok(result)
}

struct Observations<'a> {
    by_id: BTreeMap<String, &'a Value>,
    by_subject: BTreeMap<String, String>,
    receipt: String,
}

fn observations(batch: &Value) -> Result<Observations<'_>> {
    let inputs = batch.get("inputs")?.array_ref()?;
    if inputs.len() != 1 || inputs[0].get("kind")?.str()? != "SCIP" {
        return refuse("LLM proposal requires exactly one source-bound SCIP index receipt");
    }
    let receipt = inputs[0].get("receipt_id")?.str()?.to_owned();
    let by_subject = batch
        .get("subjects")?
        .array_ref()?
        .iter()
        .map(|s| {
            Ok((
                s.get("subject_id")?.str()?.to_owned(),
                s.get("path")?.str()?.to_owned(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut by_id = BTreeMap::new();
    for obs in batch.get("observations")?.array_ref()? {
        if obs.get("kind")?.str()? != "SCIP_REFERENCE"
            || obs.get("status")?.str()? != "OBSERVED"
            || obs.get("native_relation")?.str()? != "SCIP_SYMBOL_REFERENCE"
            || obs.get("receipt_id")?.str()? != receipt
        {
            return refuse("ontology authoring requires observed native SCIP references");
        }
        let id = obs.get("observation_id")?.str()?.to_owned();
        if by_id.insert(id, obs).is_some() {
            return refuse("duplicate native observation ID");
        }
    }
    Ok(Observations {
        by_id,
        by_subject,
        receipt,
    })
}

fn endpoints<'a>(obs: &Value, evidence: &'a Observations<'_>) -> Result<(&'a str, &'a str)> {
    let source = evidence
        .by_subject
        .get(obs.get("source_subject_id")?.str()?)
        .ok_or("observed reference has unknown source")?;
    let target = evidence
        .by_subject
        .get(obs.get("target_subject_id")?.str()?)
        .ok_or("observed reference has unknown target")?;
    Ok((source, target))
}

fn support(
    ids: &[Value],
    evidence: &Observations<'_>,
    subject_paths: &[&str],
) -> Result<Vec<Value>> {
    let mut result = BTreeMap::new();
    for raw in ids {
        let id = raw.str()?;
        let obs = evidence
            .by_id
            .get(id)
            .ok_or("ontology proposal cites absent SCIP observation")?;
        let (source, target) = endpoints(obs, evidence)?;
        if !subject_paths
            .iter()
            .all(|path| source == *path || target == *path)
        {
            return refuse("ontology evidence does not touch each asserted source");
        }
        // Use original native reference identity for compatibility with
        // existing SCIP claim support and its receipt replay.
        let native_id = obs.get("source_observation_id")?.str()?;
        result.insert(
            native_id.to_owned(),
            obj(&[
                ("receipt_id", Value::string(&evidence.receipt)),
                ("observation_id", Value::string(native_id)),
                ("role", Value::string("SUPPORTS")),
                ("observation_kind", Value::string("SCIP_REFERENCE")),
            ]),
        );
    }
    if result.is_empty() {
        return refuse("each proposed claim needs named source evidence");
    }
    Ok(result.into_values().collect())
}

fn claim(
    predicate: &str,
    subject_ids: Vec<String>,
    concept_id: Option<&str>,
    supports: Vec<Value>,
    rationale: &str,
) -> Result<Value> {
    let mut fields = obj(&[
        ("predicate", Value::string(predicate)),
        (
            "subjects",
            Value::array(subject_ids.into_iter().map(Value::string)),
        ),
        ("support", Value::array(supports)),
        ("assessment", Value::string("PROPOSED")),
        ("behavioral_verification", Value::string("UNTESTED")),
        ("rationale", Value::string(rationale)),
    ]);
    if let Some(id) = concept_id {
        fields
            .entries_mut()?
            .insert("concept_id".into(), Value::string(id));
    }
    concepts::seal(fields, "claim_id")
}

fn coverage(batch: &Value, receipt: &str) -> Result<Value> {
    let mut gaps = Vec::new();
    for gap in batch.get("gaps")?.array_ref()? {
        let kind = match gap.get("reason")?.str()? {
            "RESOURCE_LIMIT" => "RESOURCE_LIMIT",
            "MISSING_PRODUCER" => "MISSING_INDEXER",
            "UNRESOLVED_SYMBOL" => "UNRESOLVED_SYMBOL",
            "UNSUPPORTED_SOURCE" => "UNSUPPORTED_LANGUAGE",
            _ => "UNKNOWN_COVERAGE",
        };
        let mut entry = obj(&[("reason", Value::string(kind))]);
        if let Ok(path) = gap.get("source_path") {
            entry
                .entries_mut()?
                .insert("source_path".into(), path.clone());
        }
        gaps.push(entry);
    }
    if gaps.is_empty() {
        gaps.push(obj(&[("reason", Value::string("UNKNOWN_COVERAGE"))]));
    }
    Ok(obj(&[
        ("status", Value::string("PARTIAL")),
        (
            "producer_receipts",
            Value::array(vec![Value::string(receipt)]),
        ),
        ("gaps", Value::array(gaps)),
    ]))
}

/// Create a source-bound ontology authored by the caller, never by inference
/// from reciprocal reference counts. No LLM-authored field is proof.
pub fn model(batch: &Value, draft: &Value) -> Result<Value> {
    concepts_normalize::validate_batch(batch)?;
    validate_draft(draft, batch)?;
    let file_map = files(batch)?;
    let refs = observations(batch)?;
    let author = draft.get("author_label")?.str()?;
    let batch_id = batch.get("batch_id")?.str()?;
    let mut selected = BTreeMap::<String, Value>::new();
    let mut claims = BTreeMap::<String, Value>::new();
    let mut definitions = Vec::new();
    let mut used_keys = BTreeSet::new();

    for proposed in draft.get("concepts")?.array_ref()? {
        let key = proposed.get("key")?.str()?;
        if !used_keys.insert(key.to_owned()) {
            return refuse("duplicate ontology concept key");
        }
        let id = digest::sha256(format!("excession.llm-concept.v1\0{batch_id}\0{key}").as_bytes());
        let mut members = Vec::new();
        let mut member_paths = BTreeSet::new();
        for member in proposed.get("members")?.array_ref()? {
            let path = member.get("path")?.str()?;
            if !member_paths.insert(path.to_owned()) {
                return refuse("repeated member path in one ontology concept");
            }
            let subject = file_map
                .get(path)
                .ok_or("ontology member is not an indexed source file")?;
            selected.insert(path.to_owned(), subject.clone());
            let member_id = subject.get("subject_id")?.str()?.to_owned();
            let evidence = support(member.get("observations")?.array_ref()?, &refs, &[path])?;
            let item = claim(
                "REALIZES_CONCEPT",
                vec![member_id],
                Some(&id),
                evidence,
                proposed.get("rationale")?.str()?,
            )?;
            members.push(item.get("claim_id")?.clone());
            claims.insert(item.get("claim_id")?.str()?.to_owned(), item);
        }
        members.sort_by_key(Value::canonical);
        let aliases = proposed
            .get("aliases")
            .ok()
            .map(Value::array_ref)
            .transpose()?
            .unwrap_or(&[]);
        let concept = concepts::seal(
            obj(&[
                ("concept_id", Value::string(id)),
                ("name", proposed.get("name")?.clone()),
                ("definition", proposed.get("definition")?.clone()),
                ("rationale", proposed.get("rationale")?.clone()),
                ("aliases", Value::array(aliases.to_vec())),
                ("membership_claims", Value::array(members)),
                ("review_status", Value::string("UNREVIEWED")),
                ("origin", Value::string("LLM_PROPOSAL")),
                ("author_label", Value::string(author)),
            ]),
            "revision_id",
        )?;
        definitions.push(concept);
    }

    let mut relation_keys = BTreeSet::new();
    for relation in draft
        .get("relations")
        .ok()
        .map(Value::array_ref)
        .transpose()?
        .unwrap_or(&[])
    {
        let paths = relation.get("paths")?.array_ref()?;
        let a = paths[0].str()?;
        let b = paths[1].str()?;
        if a == b {
            return refuse("ontology relation must connect two distinct source paths");
        }
        let mut ordered = [a, b];
        ordered.sort();
        let predicate = relation.get("predicate")?.str()?;
        if !relation_keys.insert((
            predicate.to_owned(),
            ordered[0].to_owned(),
            ordered[1].to_owned(),
        )) {
            return refuse("duplicate ontology relation");
        }
        let left = file_map
            .get(a)
            .ok_or("ontology relation source not in SCIP index")?;
        let right = file_map
            .get(b)
            .ok_or("ontology relation target not in SCIP index")?;
        selected.insert(a.to_owned(), left.clone());
        selected.insert(b.to_owned(), right.clone());
        let supports = support(relation.get("observations")?.array_ref()?, &refs, &[a, b])?;
        let mut ids = vec![
            left.get("subject_id")?.str()?.to_owned(),
            right.get("subject_id")?.str()?.to_owned(),
        ];
        ids.sort();
        let item = claim(
            predicate,
            ids,
            None,
            supports,
            relation.get("rationale")?.str()?,
        )?;
        claims.insert(item.get("claim_id")?.str()?.to_owned(), item);
    }
    definitions.sort_by_key(Value::canonical);
    let result = concepts::seal(
        obj(&[
            ("schema", Value::string("excession.concepts-model.v1")),
            ("source_commit", batch.get("source_commit")?.clone()),
            ("source_tree", batch.get("source_tree")?.clone()),
            ("subjects", Value::array(selected.into_values())),
            ("claims", Value::array(claims.into_values())),
            ("concepts", Value::array(definitions)),
            ("coverage", coverage(batch, &refs.receipt)?),
        ]),
        "model_id",
    )?;
    concepts::validate_model(&result)?;
    Ok(result)
}

fn manifest(batch: &Value, draft: &Value, model: &Value) -> Result<Value> {
    let mut result = concepts_compile::manifest(batch, model)?;
    result
        .entries_mut()?
        .insert("compiler_version".into(), Value::string(COMPILER));
    result.entries_mut()?.insert(
        "rule_set_id".into(),
        Value::string(digest::sha256(
            format!("excession.llm-ontology-proposal.v1\0{}", draft.sha256()).as_bytes(),
        )),
    );
    concepts::seal(result, "snapshot_id")
}

pub fn preview(
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    draft: &Value,
) -> Result<Value> {
    let batch = concepts_scip::normalize_index(repo, revision, index, prefix)?;
    let model = model(&batch, draft)?;
    let receipt = manifest(&batch, draft, &model)?;
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.concepts-ontology-preview.v1"),
        ),
        ("proposal_id", Value::string(draft.sha256())),
        ("model", model),
        ("snapshot", receipt),
        ("status", Value::string("UNREVIEWED_LLM_PROPOSAL")),
        ("behavioral_verification_inferred", Value::Bool(false)),
    ]))
}

pub fn publish(
    db: &Db,
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    draft: &Value,
) -> Result<Value> {
    let evaluated = preview(repo, revision, index, prefix, draft)?;
    concepts_store::publish(db, evaluated.get("model")?, evaluated.get("snapshot")?)?;
    Ok(obj(&[
        ("proposal_id", evaluated.get("proposal_id")?.clone()),
        ("model_id", evaluated.get("model")?.get("model_id")?.clone()),
        (
            "snapshot_id",
            evaluated.get("snapshot")?.get("snapshot_id")?.clone(),
        ),
        ("status", Value::string("PUBLISHED_UNREVIEWED")),
        ("verified_architectural_truth", Value::Bool(false)),
    ]))
}

pub fn audit(
    db: &Db,
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    draft: &Value,
    snapshot: &str,
) -> Result<Value> {
    let expected = preview(repo, revision, index, prefix, draft)?;
    let (model, saved) = concepts_store::audit(db, snapshot)?;
    if expected.get("model")? != &model
        || expected.get("snapshot")? != &saved
        || expected.get("snapshot")?.get("snapshot_id")?.str()? != snapshot
    {
        return refuse("LLM ontology snapshot differs from replayed Git, SCIP and proposal");
    }
    Ok(obj(&[
        ("snapshot_id", Value::string(snapshot)),
        ("status", Value::string("LLM_PROPOSAL_REPLAYED_UNREVIEWED")),
        ("behavioral_verification_inferred", Value::Bool(false)),
    ]))
}

/// Discover indexed source files without forcing an LLM to know path names
/// up front. Strictly bounded, deterministic pages; search is path-only and
/// does not claim semantic classification or index completeness.
pub fn list_sources(
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    search: &str,
    offset: usize,
) -> Result<Value> {
    if search.len() > 128 || offset > 1_000_000 {
        return refuse("ontology source search or offset exceeds bounded limits");
    }
    let batch = concepts_scip::normalize_index(repo, revision, index, prefix)?;
    let known = files(&batch)?;
    let term = search.to_ascii_lowercase();
    let entries = known
        .iter()
        .filter(|(path, _)| path.to_ascii_lowercase().contains(&term))
        .collect::<Vec<_>>();
    let total = entries.len();
    let selected = entries
        .into_iter()
        .skip(offset)
        .take(64)
        .map(|(_, subject)| subject.clone())
        .collect::<Vec<_>>();
    let next = offset.saturating_add(selected.len());
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.concepts-indexed-sources.v1"),
        ),
        ("source_commit", batch.get("source_commit")?.clone()),
        ("source_tree", batch.get("source_tree")?.clone()),
        ("batch_id", batch.get("batch_id")?.clone()),
        ("sources", Value::array(selected)),
        ("total_matched", Value::number(total)),
        ("offset", Value::number(offset)),
        (
            "next_offset",
            if next < total {
                Value::number(next)
            } else {
                Value::Null
            },
        ),
        ("coverage", Value::string("PARTIAL_NATIVE_INDEX_ONLY")),
    ]))
}

/// Give an LLM a bounded page of real source context and exact native
/// observation IDs for its proposed memberships. Never imply exhaustive scope.
pub fn context(
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    path: &str,
    first_line: usize,
) -> Result<Value> {
    git::validate_path(path)?;
    if first_line == 0 {
        return refuse("source line numbers start at one");
    }
    let batch = concepts_scip::normalize_index(repo, revision, index, prefix)?;
    let all = files(&batch)?;
    let subject = all
        .get(path)
        .ok_or("requested source path is not in SCIP index")?;
    let evidence = observations(&batch)?;
    let mut matched = Vec::new();
    let mut total = 0usize;
    for obs in batch.get("observations")?.array_ref()? {
        let (a, b) = endpoints(obs, &evidence)?;
        if a != path && b != path {
            continue;
        }
        total += 1;
        if matched.len() >= MAX_CONTEXT_OBSERVATIONS {
            continue;
        }
        matched.push(obj(&[
            ("observation_id", obs.get("observation_id")?.clone()),
            ("specifier", obs.get("specifier")?.clone()),
            ("relation", obs.get("native_relation")?.clone()),
            ("source_path", Value::string(a)),
            ("target_path", Value::string(b)),
            ("source_subject_id", obs.get("source_subject_id")?.clone()),
            ("target_subject_id", obs.get("target_subject_id")?.clone()),
        ]));
    }
    let mut reader = git::BlobReader::open(repo)?;
    let bytes = reader.read_capped(subject.get("blob")?.str()?, SOURCE_BYTES)?;
    reader.finish()?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "source context is not UTF-8 text")?;
    let total_lines = text.lines().count();
    let start = first_line.saturating_sub(1);
    let lines = text
        .lines()
        .enumerate()
        .skip(start)
        .take(MAX_SNIPPET_LINES)
        .map(|(i, line)| {
            obj(&[
                ("line", Value::number(i + 1)),
                (
                    "text",
                    Value::string(line.chars().take(500).collect::<String>()),
                ),
            ])
        })
        .collect::<Vec<_>>();
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.concepts-ontology-context.v1"),
        ),
        ("source_commit", batch.get("source_commit")?.clone()),
        ("source_tree", batch.get("source_tree")?.clone()),
        ("batch_id", batch.get("batch_id")?.clone()),
        ("source", subject.clone()),
        ("excerpt", Value::array(lines)),
        ("excerpt_first_line", Value::number(first_line)),
        ("source_line_count", Value::number(total_lines)),
        ("observations", Value::array(matched)),
        ("matching_observation_count", Value::number(total)),
        (
            "truncated_observations",
            Value::Bool(total > MAX_CONTEXT_OBSERVATIONS),
        ),
        ("coverage", Value::string("PARTIAL_NATIVE_INDEX_ONLY")),
        (
            "instructions",
            Value::string(
                "Propose names, definitions, source memberships and candidate relations using cited observation_id values. No returned relationship implies semantic correctness, code execution, or review.",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_draft_fields() {
        let batch = obj(&[("source_commit", Value::string("a".repeat(40)))]);
        let draft = obj(&[(
            "schema",
            Value::string("excession.concepts-ontology-proposal.v1"),
        )]);
        assert!(validate_draft(&draft, &batch).is_err());
    }
}
