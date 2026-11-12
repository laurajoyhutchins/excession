//! Git-bound SCIP observations in the existing immutable evidence store.
//! Producer reports are evidence, not proof of correct or complete semantics.
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_coverage::{Accounting, Coverage, Facet};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse, scip_refactor};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_QUERY_RELATIONS: usize = 256;
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;

fn source_subject(
    repo: &Path,
    commit: &str,
    tree: &str,
    native: &Value,
    seen: &mut BTreeMap<String, (String, String)>,
) -> Result<String> {
    let path = native.get("path")?.str()?;
    let blob = git::oid(native.get("blob")?.str()?)?;
    if let Some((old_blob, id)) = seen.get(path) {
        if old_blob != &blob {
            return refuse("conflicting SCIP source blob for the same Git path");
        }
        return Ok(id.clone());
    }
    let src = SourceRef::from_git(repo, commit, path)?;
    if src.tree != tree || src.blob != blob {
        return refuse("SCIP source differs from authenticated Git identity");
    }
    let id = Subject::Source(src).id()?;
    seen.insert(path.to_owned(), (blob, id.clone()));
    Ok(id)
}

fn batch_from_bytes(
    repo: &Path,
    revision: &str,
    raw: &[u8],
    prefix: Option<&str>,
) -> Result<(Batch, Value)> {
    if raw.len() as u64 > MAX_INDEX_BYTES {
        return refuse("SCIP evidence source exceeds the 4 MiB immutable receipt budget");
    }
    let report = scip_refactor::evidence_from_bytes(repo, revision, raw, prefix)?;
    if report.get("schema")?.str()? != "excession.scip-observations.v1" {
        return refuse("not the source-bound SCIP observation report");
    }
    if digest::sha256(raw) != report.get("index_sha256")?.str()? {
        return refuse("SCIP raw input differs from the source-bound report");
    }
    let commit = report.get("source_commit")?.str()?;
    let tree = report.get("source_tree")?.str()?;
    let indexer = report.get("indexer")?;
    let tool = indexer.get("name")?.str()?;
    let version = indexer.get("version")?.str()?;
    let producer = format!("scip:{tool}");
    let receipt = Receipt {
        producer,
        version: version.to_owned(),
        native_id: report.get("report_id")?.str()?.to_owned(),
        raw_json: std::str::from_utf8(raw)
            .map_err(|_| "SCIP evidence input must be UTF-8 JSON")?
            .to_owned(),
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    let mut subjects = BTreeMap::new();
    let mut observations = Vec::new();
    let relations = report.get("observations")?.array_ref()?;
    for item in relations {
        if item.get("relation")?.str()? != "SCIP_SYMBOL_REFERENCE" {
            return refuse("native SCIP relation was unexpectedly promoted");
        }
        let from_id = source_subject(repo, commit, tree, item.get("from")?, &mut subjects)?;
        // Independently verify the destination too, even though the
        // observation is indexed by the source subject.
        source_subject(repo, commit, tree, item.get("to")?, &mut subjects)?;
        observations.push(Observation {
            receipt_id: receipt_id.clone(),
            subject_id: from_id,
            native_kind: "SCIP_SYMBOL_REFERENCE".to_owned(),
            payload: item.clone(),
        });
    }
    let prior_coverage = report.get("coverage")?;
    let declared_documents = usize::try_from(prior_coverage.get("indexed_documents")?.integer()?)
        .map_err(|_| "negative SCIP document count")?;
    let unresolved = usize::try_from(prior_coverage.get("unresolved_references")?.integer()?)
        .map_err(|_| "negative SCIP unresolved reference count")?;
    let source_prefix = prefix.unwrap_or("<root>");
    let scoped = Coverage {
        source_commit: commit.to_owned(),
        producer_receipt_id: receipt_id.clone(),
        facet: Facet::ScipReferences,
        selection: format!("SCIP indexed documents at {source_prefix}"),
        configuration: format!(
            "externally supplied SCIP {tool}@{version}, configuration unverified"
        ),
        configuration_resolved: false,
        declared_total: None,
        accounting: Accounting {
            examined: declared_documents,
            unsupported: 0,
            excluded: 0,
            unresolved,
            truncated: false,
        },
        limitations: vec![prior_coverage.get("limitations")?.str()?.to_owned()],
        unsupported_facet: false,
    };
    let coverage = scoped.report()?;
    let index_subject = Subject::Configuration {
        sha256: report.get("index_sha256")?.str()?.to_owned(),
    }
    .id()?;
    observations.push(Observation {
        receipt_id,
        subject_id: index_subject,
        native_kind: "SCIP_INDEX_COVERAGE".to_owned(),
        payload: coverage,
    });
    Ok((
        Batch {
            source_commit: commit.to_owned(),
            receipts: vec![receipt],
            observations,
        },
        report,
    ))
}

/// Import only after independently checking source paths, blobs and spans
/// against the pinned Git object tree. The original decoded SCIP JSON is kept.
pub fn import(
    repo: &Path,
    revision: &str,
    index: &Path,
    prefix: Option<&str>,
    db: &Db,
) -> Result<Value> {
    let size = std::fs::metadata(index)
        .map_err(|e| format!("SCIP evidence metadata: {e}"))?
        .len();
    if size > MAX_INDEX_BYTES {
        return refuse("SCIP evidence index exceeds immutable receipt budget");
    }
    let raw = std::fs::read(index).map_err(|e| format!("SCIP evidence read: {e}"))?;
    let (batch, report) = batch_from_bytes(repo, revision, &raw, prefix)?;
    let id = evidence_store::persist(db, &batch)?;
    Ok(obj(&[
        ("schema", Value::string("excession.scip-evidence-import.v1")),
        ("snapshot_id", Value::string(id)),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        (
            "source_binding",
            Value::string("GIT_OBJECTS_REPLAYED_ON_IMPORT"),
        ),
        ("producer", report.get("indexer")?.clone()),
        (
            "matched_cross_file_references",
            report
                .get("coverage")?
                .get("matched_cross_file_references")?
                .clone(),
        ),
        (
            "unresolved_references",
            report
                .get("coverage")?
                .get("unresolved_references")?
                .clone(),
        ),
        (
            "interpretation",
            Value::string("UNVERIFIED_SCIP_SYMBOL_REFERENCES"),
        ),
    ]))
}

/// Rebuild the snapshot from its retained original bytes and the pinned Git
/// tree, without relying on an external user-supplied report or running SCIP.
pub fn audit_source(
    repo: &Path,
    revision: &str,
    prefix: Option<&str>,
    db: &Db,
    snapshot: &str,
) -> Result<Value> {
    let stored = evidence_store::audit(db, snapshot)?;
    let rows = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot)],
    )?;
    if rows.len() != 1 {
        return refuse("SCIP source replay requires exactly one retained producer receipt");
    }
    let document = rows[0][0]
        .as_ref()
        .ok_or("empty persisted producer document")?;
    let item = json::parse(document.as_bytes())?;
    let producer = item.get("producer")?.str()?;
    if !producer.starts_with("scip:") {
        return refuse("snapshot is not a SCIP producer observation");
    }
    let original = item.get("raw_json")?.str()?;
    let (batch, report) = batch_from_bytes(repo, revision, original.as_bytes(), prefix)?;
    if evidence_store::manifest(&batch)? != stored {
        return refuse("persisted evidence differs from Git-bound SCIP source replay");
    }
    Ok(obj(&[
        ("schema", Value::string("excession.scip-source-audit.v1")),
        (
            "status",
            Value::string("REPLAYED_GIT_BOUND_SCIP_OBSERVATIONS"),
        ),
        ("snapshot_id", Value::string(snapshot)),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("index_sha256", report.get("index_sha256")?.clone()),
        ("indexer_independently_executed", Value::Bool(false)),
        (
            "symbol_semantics_independently_verified",
            Value::Bool(false),
        ),
        ("relationship_absence_proven", Value::Bool(false)),
    ]))
}

/// Query an immutable receipt without silently upgrading the producer's facts.
/// A missing relationship means no *observed* relationship, never absence.
pub fn explain(db: &Db, snapshot: &str, focus: Option<&str>) -> Result<Value> {
    explain_page(db, snapshot, focus, None)
}

/// Bounded keyset traversal using existing immutable observation IDs.
/// A cursor must identify an observation in the same snapshot and focus scope.
pub fn explain_page(
    db: &Db,
    snapshot: &str,
    focus: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    if let Some(path) = focus {
        git::validate_path(path)?;
    }
    if let Some(cursor) = after
        && (cursor.len() != 64
            || !cursor
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    {
        return refuse("invalid SCIP observation cursor");
    }
    let manifest = evidence_store::audit(db, snapshot)?;
    let rows = db.query(
        "SELECT document FROM evidence_observation_v1 WHERE snapshot_id=? ORDER BY observation_id",
        &[Param::Text(snapshot)],
    )?;
    let mut observed = 0_usize;
    let mut available_after = 0_usize;
    let mut results = Vec::new();
    let mut last_id = None;
    let mut cursor_found = after.is_none();
    let mut coverage = None;
    for row in rows {
        let record = json::parse(
            row[0]
                .as_deref()
                .ok_or("empty persisted observation document")?
                .as_bytes(),
        )?;
        let kind = record.get("native_kind")?.str()?;
        let payload = record.get("payload")?;
        if kind == "SCIP_INDEX_COVERAGE" {
            if coverage.replace(payload.clone()).is_some() {
                return refuse("ambiguous duplicate SCIP coverage observation");
            }
        } else if kind == "SCIP_SYMBOL_REFERENCE" {
            let from = payload.get("from")?.get("path")?.str()?;
            let to = payload.get("to")?.get("path")?.str()?;
            if focus.is_none_or(|path| path == from || path == to) {
                observed += 1;
                let id = record.get("observation_id")?.str()?;
                if !cursor_found {
                    if Some(id) == after {
                        cursor_found = true;
                    }
                    continue;
                }
                available_after += 1;
                if results.len() < MAX_QUERY_RELATIONS {
                    last_id = Some(id.to_owned());
                    results.push(obj(&[
                        ("observation_id", Value::string(id)),
                        ("receipt_id", record.get("receipt_id")?.clone()),
                        (
                            "direction",
                            Value::string(if focus == Some(to) && focus != Some(from) {
                                "INCOMING"
                            } else if focus == Some(from) && focus != Some(to) {
                                "OUTGOING"
                            } else {
                                "BOTH_OR_UNFILTERED"
                            }),
                        ),
                        ("native_kind", Value::string(kind)),
                        ("payload", payload.clone()),
                    ]));
                }
            }
        } else {
            return refuse("unrecognized SCIP evidence observation kind");
        }
    }
    if !cursor_found {
        return refuse("SCIP cursor not found in the specified snapshot and focus");
    }
    let coverage = coverage.ok_or("SCIP snapshot missing coverage observation")?;
    if coverage.get("facet")?.str()? != "SCIP_REFERENCES" {
        return refuse("SCIP snapshot has wrong coverage facet");
    }
    let truncated = available_after > results.len();
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.scip-evidence-explain.v1"),
        ),
        ("snapshot_id", Value::string(snapshot)),
        ("source_commit", manifest.get("source_commit")?.clone()),
        ("focus", focus.map_or(Value::Null, Value::string)),
        ("after", after.map_or(Value::Null, Value::string)),
        (
            "next_cursor",
            if truncated {
                Value::string(last_id.ok_or("paginated SCIP output lost cursor")?)
            } else {
                Value::Null
            },
        ),
        (
            "answerability",
            Value::string("OBSERVED_RELATIONSHIPS_ONLY"),
        ),
        (
            "source_authentication",
            Value::string("REQUIRES_GIT_REPLAY_FOR_CURRENT_QUERY"),
        ),
        ("coverage", coverage),
        ("matched_observation_count", Value::number(observed)),
        ("returned_observation_count", Value::number(results.len())),
        ("truncated", Value::Bool(truncated)),
        ("observations", Value::array(results)),
        ("absence_proven", Value::Bool(false)),
        ("safe_to_skip_tests", Value::Bool(false)),
    ]))
}

/// Source-authenticated inspection holds one consistent read snapshot while
/// replaying the original index and assembling the bounded explanation.
pub fn explain_authenticated(
    db: &Db,
    repo: &Path,
    revision: &str,
    prefix: Option<&str>,
    snapshot: &str,
    focus: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    db.read_transaction(|db| {
        audit_source(repo, revision, prefix, db, snapshot)?;
        let mut answer = explain_page(db, snapshot, focus, after)?;
        answer.entries_mut()?.insert(
            "source_authentication".into(),
            Value::string("GIT_OBJECTS_REPLAYED_FOR_QUERY"),
        );
        Ok(answer)
    })
}
