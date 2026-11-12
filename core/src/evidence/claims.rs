//! Predicate-scoped read-only audit over existing immutable native evidence.
//! Git source replay and receipt integrity do not prove indexer semantics.
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, git, refuse};
use std::path::Path;

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

fn native_source(repo: &Path, commit: &str, native: &Value) -> Result<SourceRef> {
    let path = native.get("path")?.str()?;
    let blob = native.get("blob")?.str()?;
    let source = SourceRef::from_git(repo, commit, path)?;
    if source.blob != blob {
        return refuse("native predicate refers to a different Git blob");
    }
    Ok(source)
}

fn source_record(source: &SourceRef) -> Value {
    obj(&[
        ("source_commit", Value::string(&source.commit)),
        ("source_tree", Value::string(&source.tree)),
        ("path", Value::string(&source.path)),
        ("blob", Value::string(&source.blob)),
        ("mode", Value::string(&source.mode)),
    ])
}

/// Authenticate an immutable SCIP reference's source identity and original
/// producer receipt, while explicitly leaving semantic validity unresolved.
///
/// This recognizes only the SCIP-native predicate. It cannot be called to
/// reinterpret a symbol reference as a call, build input or vulnerability.
pub fn verify_scip_reference(
    db: &Db,
    repo: &Path,
    commit: &str,
    snapshot_id: &str,
    observation_id: &str,
    source_path: &str,
    predicate: &str,
) -> Result<Value> {
    if predicate != "SCIP_SYMBOL_REFERENCE" {
        return refuse("unsupported predicate: only native SCIP_SYMBOL_REFERENCE is auditable");
    }
    db.read_transaction(|db| {
        let manifest = evidence_store::audit(db, snapshot_id)?;
        if manifest.get("source_commit")?.str()? != commit {
            return refuse("native evidence snapshot does not match selected Git commit");
        }
        let rows = db.query(
            "SELECT receipt_id, document FROM evidence_observation_v1
             WHERE snapshot_id=? AND observation_id=?",
            &[Param::Text(snapshot_id), Param::Text(observation_id)],
        )?;
        if rows.len() != 1 {
            return refuse("native predicate observation is not in the audited snapshot");
        }
        let receipt_id = rows[0][0]
            .as_deref()
            .ok_or("missing stored producer receipt identity")?;
        let document = json::parse(
            rows[0][1]
                .as_deref()
                .ok_or("missing native observation bytes")?
                .as_bytes(),
        )?;
        if document.get("observation_id")?.str()? != observation_id
            || document.get("receipt_id")?.str()? != receipt_id
            || document.get("native_kind")?.str()? != predicate
        {
            return refuse("native predicate identity, receipt or kind mismatch");
        }
        let receipt_rows = db.query(
            "SELECT document FROM evidence_receipt_v1
             WHERE snapshot_id=? AND receipt_id=?",
            &[Param::Text(snapshot_id), Param::Text(receipt_id)],
        )?;
        if receipt_rows.len() != 1 {
            return refuse("original producer receipt missing from snapshot");
        }
        let receipt = json::parse(
            receipt_rows[0][0]
                .as_deref()
                .ok_or("missing original producer receipt document")?
                .as_bytes(),
        )?;
        if !receipt.get("producer")?.str()?.starts_with("scip:") {
            return refuse("SCIP observation has no matching native SCIP producer receipt");
        }
        // These checks bind both endpoints and the recorded line to Git.
        // They do not replay the independent upstream SCIP indexer.
        let payload = document.get("payload")?;
        if payload.get("relation")?.str()? != predicate {
            return refuse("native producer predicate was reinterpreted");
        }
        let from = native_source(repo, commit, payload.get("from")?)?;
        let to = native_source(repo, commit, payload.get("to")?)?;
        if from.path != source_path
            || Subject::Source(from.clone()).id()? != document.get("subject_id")?.str()?
        {
            return refuse("native relation source subject differs from authenticated Git");
        }
        let line = payload.get("line")?.integer()?;
        let mut reader = git::BlobReader::open(repo)?;
        let bytes = reader.read_capped(&from.blob, MAX_SOURCE_BYTES)?;
        reader.finish()?;
        let mut lines = bytes.split(|byte| *byte == b'\n').count();
        if bytes.last() == Some(&b'\n') {
            lines -= 1;
        }
        if line < 1 || usize::try_from(line).ok().is_none_or(|n| n > lines) {
            return refuse("SCIP source line does not exist in exact Git blob");
        }
        let mut report = obj(&[
            ("schema", Value::string("excession.predicate-audit.v1")),
            ("snapshot_id", Value::string(snapshot_id)),
            ("observation_id", Value::string(observation_id)),
            ("receipt_id", Value::string(receipt_id)),
            ("predicate", Value::string(predicate)),
            ("from", source_record(&from)),
            ("to", source_record(&to)),
            ("line", Value::number(line)),
            ("source_identity", Value::string("INDEPENDENT_GIT_MATCH")),
            (
                "producer_payload_integrity",
                Value::string("IMMUTABLE_SQLITE_REPLAY_MATCH"),
            ),
            ("producer_execution", Value::string("UNATTESTED")),
            ("semantic_correctness", Value::string("NOT_ESTABLISHED")),
            ("coverage_completeness", Value::string("NOT_ESTABLISHED")),
            ("verified_call_or_dependency", Value::Bool(false)),
            ("verified_security_finding", Value::Bool(false)),
            ("status", Value::string("SOURCE_BOUND_NATIVE_CANDIDATE")),
            (
                "obligations",
                Value::array([
                    Value::string("INDEPENDENT_NATIVE_INDEXER_ORACLE_REQUIRED"),
                    Value::string("CONFIGURATION_COVERAGE_NOT_ESTABLISHED"),
                ]),
            ),
        ]);
        let id = report.sha256();
        report
            .entries_mut()?
            .insert("report_id".into(), Value::string(id));
        Ok(report)
    })
}
