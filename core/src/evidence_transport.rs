//! Evidence relationships across arbitrary Git revisions, not agents or branches.
//!
//! A transport edge records an independently replayable comparison of *declared*
//! source dependencies. It never promotes an unsigned historical execution to
//! evidence of behavior on another revision.
use crate::digest;
use crate::input_contract;
use crate::json::{Value, obj};
use crate::slice_evidence::{self, Source};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeMap;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS evidence_transport (
    transport_id TEXT PRIMARY KEY,
    origin_receipt_id TEXT NOT NULL REFERENCES executable_slice_receipt(receipt_id),
    source_commit TEXT NOT NULL,
    target_commit TEXT NOT NULL,
    source_view_id TEXT NOT NULL,
    target_view_id TEXT NOT NULL,
    target_manifest_json TEXT NOT NULL,
    comparison_json TEXT NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('SOURCE_MATCH_ONLY','REVERIFY_REQUIRED')),
    scope TEXT NOT NULL CHECK(scope='DECLARED_SOURCE_CLOSURE_ONLY'),
    target_behavior_verified INTEGER NOT NULL CHECK(target_behavior_verified=0)
);
CREATE INDEX IF NOT EXISTS evidence_transport_origin
    ON evidence_transport(origin_receipt_id, target_commit);
"#;

struct Snapshot {
    view: String,
    commit: String,
    entry: String,
    assets: Vec<Value>,
    sources: Vec<Source>,
}
impl Snapshot {
    fn read(repo: &Path, manifest: &Value) -> Result<Self> {
        let (view, commit, _, entry, sources) = slice_evidence::source_rows(repo, manifest)?;
        Ok(Self {
            view,
            commit,
            entry,
            assets: manifest.get("assets")?.array_ref()?.to_vec(),
            sources,
        })
    }
}

fn index(entries: &[Source]) -> BTreeMap<&str, &str> {
    entries
        .iter()
        .map(|v| (v.path.as_str(), v.blob.as_str()))
        .collect()
}

/// Compare the source identities of two independently authenticated snapshots.
/// The Python adapter must separately establish closure completeness for each.
fn comparison(left: &Snapshot, right: &Snapshot, receipt_id: &str) -> Value {
    let a = index(&left.sources);
    let b = index(&right.sources);
    let old = a
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let new = b
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let changed: Vec<Value> = input_contract::changed_paths(&old, &new)
        .into_iter()
        .map(Value::string)
        .collect();
    let same_entry = left.entry == right.entry;
    let same_assets = left.assets == right.assets;
    let match_only = changed.is_empty() && same_entry && same_assets;
    let reason = if match_only {
        "Declared Git source closure unchanged; runtime inputs and behavior on target are unverified"
    } else {
        "Source closure, entrypoint, or explicit assets changed; fresh target verification required"
    };
    obj(&[
        ("schema", Value::string("excession.evidence-transport.v1")),
        ("source_commit", Value::string(&left.commit)),
        ("target_commit", Value::string(&right.commit)),
        ("source_view_id", Value::string(&left.view)),
        ("target_view_id", Value::string(&right.view)),
        ("origin_receipt_id", Value::string(receipt_id)),
        ("changed_source_paths", Value::array(changed)),
        ("entrypoint_unchanged", Value::Bool(same_entry)),
        ("explicit_assets_unchanged", Value::Bool(same_assets)),
        (
            "decision",
            Value::string(if match_only {
                "SOURCE_MATCH_ONLY"
            } else {
                "REVERIFY_REQUIRED"
            }),
        ),
        ("scope", Value::string("DECLARED_SOURCE_CLOSURE_ONLY")),
        ("target_behavior_verified", Value::Bool(false)),
        ("reason", Value::string(reason)),
    ])
}

fn row(
    id: &str,
    receipt: &str,
    left: &Snapshot,
    right: &Snapshot,
    target_manifest: &str,
    compare: &str,
    decision: &str,
) -> Vec<Option<String>> {
    vec![
        Some(id.to_owned()),
        Some(receipt.to_owned()),
        Some(left.commit.clone()),
        Some(right.commit.clone()),
        Some(left.view.clone()),
        Some(right.view.clone()),
        Some(target_manifest.to_owned()),
        Some(compare.to_owned()),
        Some(decision.to_owned()),
        Some("DECLARED_SOURCE_CLOSURE_ONLY".to_owned()),
        Some("0".to_owned()),
    ]
}

/// Persist or replay one evidence relationship between any two Git commits.
/// The source observation must have been ingested and independently audited.
/// Neither ancestry, agent identity, nor current checkout HEAD is relevant.
pub fn transport(
    repo: &Path,
    source_manifest_path: &Path,
    target_manifest_path: &Path,
    receipt_path: &Path,
    db: &Db,
    audit: bool,
) -> Result<Value> {
    // Revalidate the origin receipt and every source row before transport;
    // do not accept an arbitrary JSON blob as an established observation.
    slice_evidence::evidence(repo, source_manifest_path, receipt_path, db, true)?;
    let source =
        slice_evidence::read_json(source_manifest_path, slice_evidence::MAX_MANIFEST_BYTES)?;
    let target =
        slice_evidence::read_json(target_manifest_path, slice_evidence::MAX_MANIFEST_BYTES)?;
    let receipt = slice_evidence::read_json(receipt_path, slice_evidence::MAX_RECEIPT_BYTES)?;
    let left = Snapshot::read(repo, &source)?;
    let right = Snapshot::read(repo, &target)?;
    let receipt_id =
        slice_evidence::validated_receipt(&receipt, &left.view, &left.commit, &left.entry)?;
    if receipt.get("status")?.str()? != "OBSERVED_PASS" {
        return refuse("only successful historical test observations can be transported");
    }
    let comparison = comparison(&left, &right, &receipt_id);
    let transport_id = digest::sha256(
        format!(
            "excession-evidence-transport-v1\0{}",
            comparison.canonical()
        )
        .as_bytes(),
    );
    let decision = comparison.get("decision")?.str()?;
    let expected = vec![row(
        &transport_id,
        &receipt_id,
        &left,
        &right,
        &target.canonical(),
        &comparison.canonical(),
        decision,
    )];
    if audit {
        let stored = db.query(
            "SELECT transport_id,origin_receipt_id,source_commit,target_commit,source_view_id,target_view_id,target_manifest_json,comparison_json,decision,scope,target_behavior_verified FROM evidence_transport WHERE transport_id=?",
            &[Param::Text(&transport_id)],
        )?;
        if stored != expected {
            return refuse("evidence transport record differs from independent replay");
        }
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query(
                    "SELECT transport_id FROM evidence_transport WHERE transport_id=?",
                    &[Param::Text(&transport_id)],
                )?
                .is_empty()
            {
                return refuse("duplicate evidence transport; use transport-audit");
            }
            db.execute(
                "INSERT INTO evidence_transport VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&transport_id),
                    Param::Text(&receipt_id),
                    Param::Text(&left.commit),
                    Param::Text(&right.commit),
                    Param::Text(&left.view),
                    Param::Text(&right.view),
                    Param::Text(&target.canonical()),
                    Param::Text(&comparison.canonical()),
                    Param::Text(decision),
                    Param::Text("DECLARED_SOURCE_CLOSURE_ONLY"),
                    Param::Integer(0),
                ],
            )?;
            Ok(())
        })?;
    }
    db.foreign_keys_valid()?;
    let mut result = comparison;
    result
        .entries_mut()?
        .insert("transport_id".into(), Value::string(transport_id));
    result
        .entries_mut()?
        .insert("independently_audited".into(), Value::Bool(audit));
    Ok(result)
}

fn field(row: &[Option<String>], index: usize) -> Result<&str> {
    row.get(index)
        .and_then(Option::as_deref)
        .ok_or("missing stored evidence field".to_owned())
}

/// Enumerate direct evidence relationships originating at a historical receipt.
/// Every listed edge is reconstructed from its persisted Git-backed manifests;
/// this is not a transitive theorem-propagation engine.
pub fn trace(repo: &Path, receipt_id: &str, db: &Db) -> Result<Value> {
    if receipt_id.len() != 64
        || !receipt_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return refuse("invalid historical receipt identifier");
    }
    let record = db.query(
        "SELECT receipt_id,view_id,commit_oid,root_tree,entry,manifest_json,receipt_json,state FROM executable_slice_receipt WHERE receipt_id=?",
        &[Param::Text(receipt_id)],
    )?;
    if record.len() != 1 {
        return refuse("historical observation was not ingested");
    }
    let stored = &record[0];
    let source_manifest = crate::json::parse(field(stored, 5)?.as_bytes())?;
    let receipt = crate::json::parse(field(stored, 6)?.as_bytes())?;
    let left = Snapshot::read(repo, &source_manifest)?;
    let authenticated_id =
        slice_evidence::validated_receipt(&receipt, &left.view, &left.commit, &left.entry)?;
    if authenticated_id != receipt_id || receipt.get("status")?.str()? != "OBSERVED_PASS" {
        return refuse("historical evidence identity/status mismatch");
    }
    let expected_record = vec![
        Some(receipt_id.to_owned()),
        Some(left.view.clone()),
        Some(left.commit.clone()),
        Some(source_manifest.get("tree")?.str()?.to_owned()),
        Some(left.entry.clone()),
        Some(source_manifest.canonical()),
        Some(receipt.canonical()),
        Some("UNSIGNED_LOCAL_OBSERVATION".into()),
    ];
    if stored != &expected_record {
        return refuse("historical receipt record differs from independent reconstruction");
    }
    let stored_source = db.query(
        "SELECT receipt_id,path,blob_oid,sha256,byte_count FROM executable_slice_source WHERE receipt_id=? ORDER BY path",
        &[Param::Text(receipt_id)],
    )?;
    if stored_source
        != left
            .sources
            .iter()
            .map(|s| s.row(receipt_id))
            .collect::<Vec<_>>()
    {
        return refuse("historical receipt source relations differ from Git");
    }
    let rows = db.query(
        "SELECT transport_id,origin_receipt_id,source_commit,target_commit,source_view_id,target_view_id,target_manifest_json,comparison_json,decision,scope,target_behavior_verified FROM evidence_transport WHERE origin_receipt_id=? ORDER BY target_commit,transport_id",
        &[Param::Text(receipt_id)],
    )?;
    if rows.len() > 256 {
        return refuse("historical evidence trace exceeds 256 edges");
    }
    let mut edges = Vec::new();
    for stored_edge in rows {
        let target_manifest = crate::json::parse(field(&stored_edge, 6)?.as_bytes())?;
        let right = Snapshot::read(repo, &target_manifest)?;
        let compare = comparison(&left, &right, receipt_id);
        let id = digest::sha256(
            format!("excession-evidence-transport-v1\0{}", compare.canonical()).as_bytes(),
        );
        let expected = row(
            &id,
            receipt_id,
            &left,
            &right,
            &target_manifest.canonical(),
            &compare.canonical(),
            compare.get("decision")?.str()?,
        );
        if stored_edge != expected {
            return refuse("historical transport edge differs from independent Git replay");
        }
        let mut edge = compare;
        edge.entries_mut()?
            .insert("transport_id".into(), Value::string(id));
        edges.push(edge);
    }
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("schema", Value::string("excession.evidence-trace.v1")),
        ("origin_receipt_id", Value::string(receipt_id)),
        ("edge_count", Value::number(edges.len())),
        ("edges", Value::array(edges)),
        (
            "scope",
            Value::string("DIRECT_DECLARED_SOURCE_RELATIONSHIPS_ONLY"),
        ),
        ("independently_audited", Value::Bool(true)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_equality_is_not_target_behavior() {
        let item = Source {
            path: "test.py".into(),
            blob: "1".repeat(40),
            sha256: "2".repeat(64),
            bytes: 2,
        };
        let left = Snapshot {
            view: "a".into(),
            commit: "b".into(),
            entry: "test.py".into(),
            assets: vec![],
            sources: vec![item.clone()],
        };
        let right = Snapshot {
            view: "c".into(),
            commit: "d".into(),
            entry: "test.py".into(),
            assets: vec![],
            sources: vec![item],
        };
        let result = comparison(&left, &right, "e");
        assert_eq!(
            result.get("decision").unwrap().str().unwrap(),
            "SOURCE_MATCH_ONLY"
        );
        assert!(
            !result
                .get("target_behavior_verified")
                .unwrap()
                .bool()
                .unwrap()
        );
    }
    #[test]
    fn same_bytes_in_different_paths_is_not_preservation() {
        let one = Source {
            path: "x.py".into(),
            blob: "1".repeat(40),
            sha256: "2".repeat(64),
            bytes: 2,
        };
        let two = Source {
            path: "y.py".into(),
            ..one.clone()
        };
        let left = Snapshot {
            view: "a".into(),
            commit: "b".into(),
            entry: "x.py".into(),
            assets: vec![],
            sources: vec![one],
        };
        let right = Snapshot {
            view: "c".into(),
            commit: "d".into(),
            entry: "y.py".into(),
            assets: vec![],
            sources: vec![two],
        };
        let result = comparison(&left, &right, "e");
        assert_eq!(
            result.get("decision").unwrap().str().unwrap(),
            "REVERIFY_REQUIRED"
        );
        assert_eq!(
            result
                .get("changed_source_paths")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            2
        );
    }
}
