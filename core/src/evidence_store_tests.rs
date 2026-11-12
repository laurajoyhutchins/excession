//! Exact-head persistence tests: no native semantic correctness is inferred.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn database() -> (Db, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "excession-evidence-store-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    (Db::open(&path).unwrap(), path)
}
fn batch() -> Batch {
    let receipt = Receipt {
        producer: "scip-rust".into(),
        version: "1.0".into(),
        native_id: "original-index-1".into(),
        raw_json: "{\n \"symbols\": []\n}".into(),
    };
    let receipt_id = receipt
        .document()
        .unwrap()
        .get("receipt_id")
        .unwrap()
        .str()
        .unwrap()
        .to_owned();
    Batch {
        source_commit: "a".repeat(40),
        receipts: vec![receipt],
        observations: vec![Observation {
            receipt_id,
            subject_id: "b".repeat(64),
            native_kind: "SCIP_REFERENCE".into(),
            payload: obj(&[("status", Value::string("CANDIDATE"))]),
        }],
    }
}
#[test]
fn repeat_import_is_idempotent_and_reopens_exactly() {
    let (db, path) = database();
    let data = batch();
    let id = persist(&db, &data).unwrap();
    let manifest = audit(&db, &id).unwrap();
    assert_eq!(persist(&db, &data).unwrap(), id);
    assert_eq!(audit(&db, &id).unwrap(), manifest);
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_observation_v1", &[])
            .unwrap(),
        "1"
    );
    drop(db);
    let db = Db::open(&path).unwrap();
    assert_eq!(audit(&db, &id).unwrap(), manifest);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rejected_inputs_do_not_leave_partial_snapshot() {
    let (db, path) = database();
    init(&db).unwrap();
    let mut data = batch();
    data.observations[0].receipt_id = "f".repeat(64);
    assert!(persist(&db, &data).is_err());
    assert_eq!(
        db.scalar("SELECT count(*) FROM evidence_snapshot_v1", &[])
            .unwrap(),
        "0"
    );
    let data = batch();
    let id = persist(&db, &data).unwrap();
    assert!(
        db.transaction::<()>(|db| {
            db.execute(
                "DELETE FROM evidence_observation_v1 WHERE snapshot_id=?",
                &[Param::Text(&id)],
            )?;
            refuse("injected rollback")
        })
        .is_err()
    );
    audit(&db, &id).unwrap();
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn missing_extra_or_tampered_rows_refuse_audit_and_reimport() {
    let mutations = [
        "DELETE FROM evidence_observation_v1",
        "UPDATE evidence_receipt_v1 SET document='{}'",
        "UPDATE evidence_snapshot_v1 SET source_commit='ffffffffffffffffffffffffffffffffffffffff'",
        "INSERT INTO evidence_observation_v1(snapshot_id,observation_id,receipt_id,document) SELECT snapshot_id,'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',receipt_id,document FROM evidence_observation_v1",
    ];
    for mutation in mutations {
        let (db, path) = database();
        let data = batch();
        let id = persist(&db, &data).unwrap();
        db.exec(mutation).unwrap();
        assert!(audit(&db, &id).is_err(), "accepted tampering: {mutation}");
        assert!(
            persist(&db, &data).is_err(),
            "silently repaired invalid snapshot: {mutation}"
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn duplicate_keys_and_forged_source_claims_refuse_import() {
    let (db, path) = database();
    let mut data = batch();
    data.receipts[0].raw_json = "{\"symbols\":[],\"symbols\":[]}".into();
    assert!(persist(&db, &data).is_err());
    let mut data = batch();
    data.source_commit = "not-a-commit".into();
    assert!(persist(&db, &data).is_err());
    let mut data = batch();
    data.observations.push(data.observations[0].clone());
    assert!(persist(&db, &data).is_err());
    drop(db);
    std::fs::remove_file(path).unwrap();
}
