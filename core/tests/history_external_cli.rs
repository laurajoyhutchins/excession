//! External references are classified, never promoted to transferred approvals.
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (std::path::PathBuf, Value, Value) {
    let dir = std::env::temp_dir().join(format!(
        "excession-history-external-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let c = "c".repeat(40);
    let d = "d".repeat(40);
    let e = "e".repeat(40);
    let mapping = json!({
        "schema":"excession.history-dag-mapping.v1",
        "original_head":a,
        "candidate_head":b,
        "source_commit_count":2,
        "rewrite_authorized":false,
        "mapping":[
            {"original":a, "candidate":b},
            {"original":c, "candidate":c}
        ]
    });
    let snapshot = json!({
        "schema":"excession.history-external-snapshot.v1",
        "repository":"laurajoyhutchins/excession",
        "pinned_original_head":a,
        "captured_at":"2026-11-15T00:00:00Z",
        "pull_requests":[
            {"number":1,"state":"closed","head_commit":a,"base_commit":c,"merge_commit":null},
            {"number":2,"state":"open","head_commit":d,"base_commit":c,"merge_commit":e}
        ],
        "receipts":[
            {"id":"build/1","kind":"BUILD_PROVENANCE","source_commit":a,"receipt_sha256":"f".repeat(64)},
            {"id":"rea/1","kind":"REA_EVIDENCE","source_commit":c,"receipt_sha256":"0".repeat(64)}
        ]
    });
    (dir, mapping, snapshot)
}

fn invoke(dir: &Path, mapping: &Value, snapshot: &Value) -> std::process::Output {
    let mapping_path = dir.join("mapping.json");
    let snapshot_path = dir.join("snapshot.json");
    fs::write(&mapping_path, serde_json::to_vec(mapping).unwrap()).unwrap();
    fs::write(&snapshot_path, serde_json::to_vec(snapshot).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["history", "reconcile-external", "--mapping"])
        .arg(mapping_path)
        .args(["--snapshot"])
        .arg(snapshot_path)
        .output()
        .unwrap()
}

#[test]
fn reclassifies_changed_id_without_misrepresenting_pr_or_receipt_trust() {
    let (dir, mapping, snapshot) = fixture();
    let result = invoke(&dir, &mapping, &snapshot);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        report["classification"],
        "SOURCE_IDENTITIES_CLASSIFIED_NOT_ATTESTED"
    );
    assert_eq!(report["pull_request_count"], 2);
    assert_eq!(report["changed_pull_request_count"], 1);
    assert_eq!(report["unfetched_pull_request_count"], 1);
    assert_eq!(report["receipt_count"], 2);
    assert_eq!(report["changed_receipt_count"], 1);
    assert_eq!(report["unfetched_receipt_count"], 0);
    assert_eq!(
        report["pull_requests"][0]["obligation"],
        "REVIEW_AND_COMMIT_REAPPROVAL_REQUIRED"
    );
    assert_eq!(
        report["pull_requests"][1]["obligation"],
        "UNFETCHED_GITHUB_PR_SOURCE"
    );
    assert_eq!(
        report["receipts"][0]["obligation"],
        "INDEPENDENT_REBUILD_OR_REA_REBIND_REQUIRED"
    );
    assert_eq!(
        report["receipts"][1]["obligation"],
        "ORIGINAL_SOURCE_ID_UNCHANGED_RECEIPT_NOT_AUTHENTICATED"
    );
    assert_eq!(report["github_review_approvals_transferred"], false);
    assert_eq!(report["build_provenance_verified"], false);
    assert_eq!(report["rea_evidence_verified"], false);
    assert_eq!(report["rewrite_authorized"], false);
    assert_eq!(report["report_id"].as_str().unwrap().len(), 64);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stale_or_forged_inventory_refuses_without_substitution() {
    let (dir, mapping, mut snapshot) = fixture();
    snapshot["pinned_original_head"] = json!("f".repeat(40));
    assert!(!invoke(&dir, &mapping, &snapshot).status.success());
    snapshot["pinned_original_head"] = mapping["original_head"].clone();
    snapshot["pull_requests"][1]["number"] = json!(1);
    assert!(!invoke(&dir, &mapping, &snapshot).status.success());
    snapshot["pull_requests"][1]["number"] = json!(2);
    snapshot["receipts"][0]["receipt_sha256"] = json!("not-a-sha256");
    assert!(!invoke(&dir, &mapping, &snapshot).status.success());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_mapping_with_contradictory_head_or_rewrite_permission() {
    let (dir, mut mapping, snapshot) = fixture();
    mapping["rewrite_authorized"] = json!(true);
    assert!(!invoke(&dir, &mapping, &snapshot).status.success());
    mapping["rewrite_authorized"] = json!(false);
    mapping["candidate_head"] = json!("e".repeat(40));
    assert!(!invoke(&dir, &mapping, &snapshot).status.success());
    fs::remove_dir_all(dir).unwrap();
}
