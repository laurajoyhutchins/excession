//! Classify external source-bound references against an isolated DAG mapping.
//! Classification is not authentication of a GitHub API response, review,
//! build receipt, REA claim, or reconstructed Git object.
use crate::{Result, digest, git, refuse};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_MAPPING_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PR_RECORDS: usize = 10_000;
const MAX_RECEIPTS: usize = 10_000;

fn load(path: &Path, max_bytes: u64) -> Result<(Value, String)> {
    let metadata = fs::metadata(path).map_err(|e| format!("external evidence metadata: {e}"))?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return refuse(format!("input is not a regular file <= {max_bytes} bytes"));
    }
    let bytes = fs::read(path).map_err(|e| format!("read external evidence: {e}"))?;
    let hash = digest::sha256(&bytes);
    let data = serde_json::from_slice(&bytes).map_err(|e| format!("parse evidence JSON: {e}"))?;
    Ok((data, hash))
}

fn fields(value: &Value, required: &[&str], label: &str) -> Result<()> {
    let object = value.as_object().ok_or_else(|| format!("{label} must be an object"))?;
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return refuse(format!("{label} contains missing or unrecognized fields"));
    }
    Ok(())
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or_else(|| format!("{key} must be a string"))
}

fn checked_oid(value: &Value, key: &str) -> Result<String> {
    git::oid(str_field(value, key)?)
}

fn required_array<'a>(value: &'a Value, field: &str, limit: usize) -> Result<&'a Vec<Value>> {
    let values = value[field]
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?;
    if values.len() > limit {
        return refuse(format!("{field} exceeds {limit} source records"));
    }
    Ok(values)
}

fn digest_field(value: &Value, field: &str) -> Result<String> {
    let digest = str_field(value, field)?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return refuse(format!("{field} must be a lowercase SHA-256 digest"));
    }
    Ok(digest.to_owned())
}

fn locate(map: &BTreeMap<String, String>, original: &str) -> Value {
    match map.get(original) {
        None => json!({
            "original_commit":original,
            "candidate_commit":null,
            "status":"NOT_IN_LOCAL_GIT_DAG"
        }),
        Some(mapped) if mapped == original => json!({
            "original_commit":original,
            "candidate_commit":mapped,
            "status":"ORIGINAL_COMMIT_ID_UNCHANGED"
        }),
        Some(mapped) => json!({
            "original_commit":original,
            "candidate_commit":mapped,
            "status":"COMMIT_ID_CHANGED_EVIDENCE_INVALIDATED"
        }),
    }
}

fn classify_reference(map: &BTreeMap<String, String>, original: &str) -> (Value, &'static str) {
    let classified = locate(map, original);
    let status = match classified["status"].as_str().unwrap_or("") {
        "NOT_IN_LOCAL_GIT_DAG" => "NOT_FETCHED",
        "COMMIT_ID_CHANGED_EVIDENCE_INVALIDATED" => "CHANGED",
        _ => "UNCHANGED",
    };
    (classified, status)
}

/// Bind a supplied PR/receipt inventory to the exact source commit mapping.
/// Every changed identity remains an unverified obligation; no approval or
/// attestation can be copied by this classifier.
pub fn reconcile(mapping_path: &Path, snapshot_path: &Path) -> Result<Value> {
    let (mapping, mapping_sha) = load(mapping_path, MAX_MAPPING_BYTES)?;
    let (snapshot, snapshot_sha) = load(snapshot_path, MAX_SNAPSHOT_BYTES)?;
    if mapping["schema"] != "excession.history-dag-mapping.v1"
        || snapshot["schema"] != "excession.history-external-snapshot.v1"
    {
        return refuse("unknown DAG mapping or external inventory schema");
    }
    if mapping["rewrite_authorized"] != false {
        return refuse("imported DAG mapping unexpectedly authorizes rewrite");
    }
    fields(&snapshot, &[
        "schema", "repository", "pinned_original_head", "captured_at",
        "pull_requests", "receipts"
    ], "external snapshot")?;
    let original_head = checked_oid(&mapping, "original_head")?;
    let candidate_head = checked_oid(&mapping, "candidate_head")?;
    if checked_oid(&snapshot, "pinned_original_head")? != original_head {
        return refuse("external snapshot does not pin the mapped source HEAD");
    }
    let repository = str_field(&snapshot, "repository")?;
    if repository.is_empty() || repository.len() > 256 || !repository.contains('/') {
        return refuse("external snapshot repository identity is invalid");
    }
    let captured_at = str_field(&snapshot, "captured_at")?;
    if captured_at.len() > 64 || captured_at.is_empty() {
        return refuse("external snapshot capture metadata is missing or oversized");
    }
    let rows = required_array(&mapping, "mapping", 100_000)?;
    if mapping["source_commit_count"].as_u64() != Some(rows.len() as u64) {
        return refuse("DAG mapping count disagrees with its records");
    }
    let mut map = BTreeMap::<String, String>::new();
    for row in rows {
        let original = checked_oid(row, "original")?;
        let candidate = checked_oid(row, "candidate")?;
        if map.insert(original, candidate).is_some() {
            return refuse("DAG mapping has duplicate original commit identities");
        }
    }
    if map.get(&original_head) != Some(&candidate_head) {
        return refuse("candidate HEAD is not bound to the original HEAD mapping");
    }
    let mut numbers = BTreeSet::new();
    let mut prs = Vec::new();
    let mut changed_prs = 0;
    let mut missing_prs = 0;
    for row in required_array(&snapshot, "pull_requests", MAX_PR_RECORDS)? {
        fields(row, &[
            "number", "state", "head_commit", "base_commit", "merge_commit"
        ], "pull request")?;
        let number = row["number"].as_u64().ok_or("PR number must be a positive integer")?;
        if number == 0 || !numbers.insert(number) {
            return refuse("duplicate or invalid PR number");
        }
        let state = str_field(row, "state")?;
        if !matches!(state, "open" | "closed") {
            return refuse("unsupported PR state");
        }
        let head = checked_oid(row, "head_commit")?;
        let base = checked_oid(row, "base_commit")?;
        let merge = match &row["merge_commit"] {
            Value::Null => None,
            Value::String(s) => Some(git::oid(s)?),
            _ => return refuse("PR merge_commit must be a Git SHA-1 or null"),
        };
        let (head_status, head_class) = classify_reference(&map, &head);
        let (base_status, base_class) = classify_reference(&map, &base);
        let (merge_status, merge_class) = match merge {
            Some(oid) => {
                let (status, class) = classify_reference(&map, &oid);
                (Some(status), Some(class))
            }
            None => (None, None),
        };
        let missing = head_class == "NOT_FETCHED" || base_class == "NOT_FETCHED"
            || merge_class == Some("NOT_FETCHED");
        let changed = head_class == "CHANGED" || base_class == "CHANGED"
            || merge_class == Some("CHANGED");
        missing_prs += usize::from(missing);
        changed_prs += usize::from(changed);
        let obligation = if missing {
            "UNFETCHED_GITHUB_PR_SOURCE"
        } else if changed {
            "REVIEW_AND_COMMIT_REAPPROVAL_REQUIRED"
        } else {
            "NO_COMMIT_ID_CHANGE_NO_APPROVAL_TRANSFER"
        };
        prs.push(json!({
            "number":number,
            "state":state,
            "head":head_status,
            "base":base_status,
            "merge":merge_status,
            "obligation":obligation
        }));
    }
    let mut seen_receipts = BTreeSet::new();
    let mut receipts = Vec::new();
    let mut changed_receipts = 0;
    let mut missing_receipts = 0;
    let mut kinds = BTreeMap::<String, usize>::new();
    for row in required_array(&snapshot, "receipts", MAX_RECEIPTS)? {
        fields(row, &["id", "kind", "source_commit", "receipt_sha256"], "receipt")?;
        let id = str_field(row, "id")?;
        if id.trim().is_empty() || id.len() > 256 || !seen_receipts.insert(id.to_owned()) {
            return refuse("duplicate, empty or oversized receipt ID");
        }
        let kind = str_field(row, "kind")?;
        if !matches!(kind, "BUILD_PROVENANCE" | "REA_EVIDENCE") {
            return refuse("unsupported external evidence kind");
        }
        *kinds.entry(kind.to_owned()).or_default() += 1;
        let source = checked_oid(row, "source_commit")?;
        let receipt_digest = digest_field(row, "receipt_sha256")?;
        let (located, disposition) = classify_reference(&map, &source);
        changed_receipts += usize::from(disposition == "CHANGED");
        missing_receipts += usize::from(disposition == "NOT_FETCHED");
        receipts.push(json!({
            "id":id,
            "kind":kind,
            "source":located,
            "receipt_sha256":receipt_digest,
            "obligation":match disposition {
                "CHANGED" => "INDEPENDENT_REBUILD_OR_REA_REBIND_REQUIRED",
                "NOT_FETCHED" => "SOURCE_UNAVAILABLE_RECEIPT_UNASSESSED",
                _ => "ORIGINAL_SOURCE_ID_UNCHANGED_RECEIPT_NOT_AUTHENTICATED"
            }
        }));
    }
    let mut report = json!({
        "schema":"excession.history-external-reconciliation.v1",
        "repository":repository,
        "original_head":original_head,
        "candidate_head":candidate_head,
        "captured_at_unverified":captured_at,
        "mapping_file_sha256":mapping_sha,
        "snapshot_file_sha256":snapshot_sha,
        "classification":"SOURCE_IDENTITIES_CLASSIFIED_NOT_ATTESTED",
        "pull_request_count":prs.len(),
        "changed_pull_request_count":changed_prs,
        "unfetched_pull_request_count":missing_prs,
        "receipt_count":receipts.len(),
        "changed_receipt_count":changed_receipts,
        "unfetched_receipt_count":missing_receipts,
        "receipt_kind_counts":kinds,
        "pull_requests":prs,
        "receipts":receipts,
        "source_completeness":"SUPPLIED_SNAPSHOT_ONLY",
        "inventory_authenticity":"NOT_CRYPTOGRAPHICALLY_VERIFIED",
        "github_review_approvals_transferred":false,
        "build_provenance_verified":false,
        "rea_evidence_verified":false,
        "rewrite_authorized":false
    });
    let canonical = serde_json::to_vec(&report).map_err(|e| format!("serialize external evidence: {e}"))?;
    report["report_id"] = Value::String(digest::sha256(&canonical));
    Ok(report)
}
