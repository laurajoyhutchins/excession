//! Single artifact-centric evidence view over the native Git/SQLite store.
//! REA's native observations and build recipe facts are independent facets.
use crate::evidence::{build, identity::Subject, rea};
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::path::Path;

const MAX_RESULTS: usize = 16;

fn string(row: &[Option<String>], at: usize) -> Result<&str> {
    row.get(at)
        .and_then(|value| value.as_deref())
        .ok_or_else(|| "missing artifact evidence SQL field".into())
}

/// Examine a bounded artifact and join independently replayable build and
/// REA evidence by its typed artifact identity, never by matching filenames.
pub fn dossier(
    db: &Db,
    artifact: &Path,
    link: Option<&rea::BuildLink<'_>>,
    limit: usize,
) -> Result<Value> {
    if !(1..=MAX_RESULTS).contains(&limit) {
        return refuse("artifact evidence limit must be 1..16");
    }
    let (digest, bytes) = build::read_artifact(artifact)?;
    let subject_id = Subject::Artifact {
        sha256: digest.clone(),
        size_bytes: bytes,
    }
    .id()?;
    let source = if let Some(link) = link {
        let report = build::audit(db, link.repo, link.revision, artifact, link.snapshot)?;
        if report.get("artifact_subject_id")?.str()? != subject_id {
            return refuse("build source and REA subject do not identify the same artifact bytes");
        }
        report
    } else {
        Value::Null
    };
    let (rows, total) = db.read_transaction(|db| {
        let installed = db.scalar(
            "SELECT count(*) FROM sqlite_master
             WHERE type='table' AND name='evidence_rea_bundle_v1'",
            &[],
        )?;
        if installed == "0" {
            return Ok((Vec::new(), 0_usize));
        }
        let total = db.scalar(
            "SELECT count(*) FROM evidence_rea_bundle_v1 WHERE artifact_subject_id=?",
            &[Param::Text(&subject_id)],
        )?;
        let count = total.parse::<usize>().map_err(|_| "invalid REA evidence count")?;
        let rows = db.query(
            "SELECT bundle_id,build_snapshot_id FROM evidence_rea_bundle_v1
             WHERE artifact_subject_id=? ORDER BY bundle_id LIMIT ?",
            &[
                Param::Text(&subject_id),
                Param::Integer(i64::try_from(limit).map_err(|_| "invalid artifact query limit")?),
            ],
        )?;
        Ok((rows, count))
    })?;
    let mut evidence = Vec::with_capacity(rows.len());
    for row in rows {
        let id = string(&row, 0)?;
        let build_snapshot = row.get(1).and_then(|v| v.as_deref());
        match build_snapshot {
            Some(stored) if link.is_none_or(|link| stored != link.snapshot) => {
                // Never claim to have authenticated an unreplayed source link.
                evidence.push(obj(&[
                    ("bundle_id", Value::string(id)),
                    ("build_snapshot_id", Value::string(stored)),
                    ("state", Value::string("BUILD_REPLAY_REQUIRED")),
                    ("semantic_correctness", Value::string("NOT_ESTABLISHED")),
                ]));
            }
            _ => {
                let checked = rea::audit(
                    db,
                    artifact,
                    id,
                    if build_snapshot.is_some() { link } else { None },
                )?;
                evidence.push(obj(&[
                    ("bundle_id", Value::string(id)),
                    ("build_snapshot_id", checked.get("build_snapshot_id")?.clone()),
                    ("state", Value::string("ARTIFACT_AND_NATIVE_RECORDS_REPLAYED")),
                    ("native_record_ids", checked.get("native_record_ids")?.clone()),
                    ("semantic_correctness", Value::string("NOT_ESTABLISHED")),
                ]));
            }
        }
    }
    let mut report = obj(&[
        ("schema", Value::string("excession.artifact-evidence.v1")),
        ("artifact_subject_id", Value::string(subject_id)),
        ("artifact_sha256", Value::string(digest)),
        ("artifact_size_bytes", Value::number(bytes)),
        ("build_evidence", source),
        ("rea_evidence", Value::array(evidence)),
        ("rea_total_stored", Value::number(total)),
        ("rea_results_truncated", Value::Bool(total > limit)),
        ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
        ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ("coverage_completeness", Value::string("NOT_ESTABLISHED")),
    ]);
    let id = report.sha256();
    report.entries_mut()?.insert("report_id".into(), Value::string(id));
    Ok(report)
}
