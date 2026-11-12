//! Validate actual persisted evidence events against the published wire shape.
//! Canonical row-chain replay and semantic non-promotion remain Rust/SQLite checks.
use excession::evidence::unknowns::{self, Action, Event};
use excession::evidence_store::{self, Batch, Receipt};
use excession::sqlite::Db;
use serde_json::{Value as JsonValue, json};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn event_schema() -> jsonschema::Validator {
    let schema: JsonValue = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/unknown-event.v1.schema.json"
    ))
    .unwrap();
    jsonschema::draft202012::new(&schema).unwrap()
}

#[test]
fn actual_sqlite_events_conform_and_remain_unverified() {
    let path = std::env::temp_dir().join(format!(
        "excession-unknown-contract-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let db = Db::open(&path).unwrap();
    let receipt = Receipt {
        producer: "upstream-native".into(),
        version: "1.0".into(),
        native_id: "one".into(),
        raw_json: "{\"observed\":true}".into(),
    };
    let receipt_id = receipt
        .document()
        .unwrap()
        .get("receipt_id")
        .unwrap()
        .str()
        .unwrap()
        .to_owned();
    let snapshot = evidence_store::persist(
        &db,
        &Batch {
            source_commit: "a".repeat(40),
            receipts: vec![receipt],
            observations: vec![],
        },
    )
    .unwrap();
    let first = unknowns::append(
        &db,
        &Event {
            snapshot_id: &snapshot,
            question: "Which call target exists?",
            expected_revision: 0,
            action: Action::Create,
            support_receipts: &[&receipt_id],
            counterevidence_receipts: &[],
            resolution_receipt: None,
        },
    )
    .unwrap();
    let second = unknowns::append(
        &db,
        &Event {
            snapshot_id: &snapshot,
            question: "Which call target exists?",
            expected_revision: 1,
            action: Action::ProposeResolution,
            support_receipts: &[&receipt_id],
            counterevidence_receipts: &[],
            resolution_receipt: Some(&receipt_id),
        },
    )
    .unwrap();
    let schema = event_schema();
    for event in [&first, &second] {
        let document: JsonValue = serde_json::from_str(&event.canonical()).unwrap();
        assert!(schema.is_valid(&document), "{document}");
    }
    let report = unknowns::audit(
        &db,
        &unknowns::unknown_id(&snapshot, "Which call target exists?").unwrap(),
    )
    .unwrap();
    assert_eq!(
        report.get("state").unwrap().str().unwrap(),
        "RESOLUTION_REPORTED_NOT_VERIFIED"
    );
    assert_eq!(
        report.get("semantic_verification").unwrap().str().unwrap(),
        "NOT_ESTABLISHED"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn schema_rejects_false_verification_and_missing_identity() {
    let schema = event_schema();
    let valid = json!({
        "schema":"excession.evidence-unknown-event.v1",
        "unknown_id":"a".repeat(64),
        "snapshot_id":"b".repeat(64),
        "question":"What is known?",
        "revision":1,
        "previous_event_id":null,
        "action":"CREATE",
        "state":"OPEN",
        "support_receipt_ids":[],
        "counterevidence_receipt_ids":[],
        "resolution_receipt_id":null,
        "semantic_verification":"NOT_ESTABLISHED",
        "event_id":"c".repeat(64)
    });
    assert!(schema.is_valid(&valid));
    let mut false_proof = valid.clone();
    false_proof["semantic_verification"] = json!("BEHAVIORALLY_PROVEN");
    assert!(!schema.is_valid(&false_proof));
    let mut no_identity = valid.clone();
    no_identity.as_object_mut().unwrap().remove("snapshot_id");
    assert!(!schema.is_valid(&no_identity));
    let mut no_receipt = valid.clone();
    no_receipt["action"] = json!("PROPOSE_RESOLUTION");
    assert!(!schema.is_valid(&no_receipt));
    let mut duplicate = valid;
    duplicate["support_receipt_ids"] = json!(["c".repeat(64), "c".repeat(64)]);
    assert!(!schema.is_valid(&duplicate));
}
