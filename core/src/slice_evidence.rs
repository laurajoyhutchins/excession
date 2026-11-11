//! Independent Git and SQLite binding for bounded Python executable-slice observations.
//!
//! The Python adapter owns static import semantics. This module verifies only
//! the declared source closure and the finite, unsigned local receipt. Neither
//! the receipt hash nor SQLite constitutes an executor attestation.
use crate::digest;
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS executable_slice_receipt (
 receipt_id TEXT PRIMARY KEY, view_id TEXT NOT NULL, commit_oid TEXT NOT NULL,
 root_tree TEXT NOT NULL, entry TEXT NOT NULL,
 manifest_json TEXT NOT NULL, receipt_json TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state='UNSIGNED_LOCAL_OBSERVATION')
);
CREATE TABLE IF NOT EXISTS executable_slice_source (
 receipt_id TEXT NOT NULL REFERENCES executable_slice_receipt(receipt_id),
 path TEXT NOT NULL, blob_oid TEXT NOT NULL, sha256 TEXT NOT NULL,
 byte_count INTEGER NOT NULL CHECK(byte_count >= 0),
 PRIMARY KEY(receipt_id,path)
);
"#;
const MAX_FILES: usize = 512;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 5 * 1024 * 1024;
const MAX_RECEIPT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Source {
    path: String,
    blob: String,
    sha256: String,
    bytes: i64,
}
impl Source {
    fn row(&self, id: &str) -> Vec<Option<String>> {
        vec![
            Some(id.to_owned()),
            Some(self.path.clone()),
            Some(self.blob.clone()),
            Some(self.sha256.clone()),
            Some(self.bytes.to_string()),
        ]
    }
}

fn read_json(path: &Path, bound: usize) -> Result<Value> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.len() > bound {
        return refuse("slice evidence input exceeds size limit");
    }
    json::parse(&data)
}

fn hex64(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn check_digest(value: &Value) -> Result<String> {
    let hash = value.str()?;
    if !hex64(hash) {
        return refuse("invalid SHA-256 digest");
    }
    Ok(hash.to_owned())
}

fn without_identity(value: &Value, key: &str) -> Result<Value> {
    let mut copy = value.clone();
    if copy.entries_mut()?.remove(key).is_none() {
        return refuse(format!("missing identity: {key}"));
    }
    Ok(copy)
}

fn source_rows(
    repo: &Path,
    manifest: &Value,
) -> Result<(String, String, String, String, Vec<Source>)> {
    if manifest.get("schema")?.str()? != "excession.executable-slice.v1"
        || manifest.get("scope")?.str()? != "STATIC_IMPORT_CLOSURE_ONLY"
        || manifest.get("semantic_equivalence")?.str()? != "UNVERIFIED"
    {
        return refuse("unsupported executable slice schema or scope");
    }
    let view = check_digest(manifest.get("view_id")?)?;
    if without_identity(manifest, "view_id")?.sha256() != view {
        return refuse("manifest view_id does not bind canonical contents");
    }
    git::clean(repo)?;
    let commit = git::oid(manifest.get("commit")?.str()?)?;
    if commit != git::rev(repo, "HEAD")? {
        return refuse("manifest commit is not checkout HEAD");
    }
    let tree = git::oid(manifest.get("tree")?.str()?)?;
    if tree != git::rev(repo, "HEAD^{tree}")? {
        return refuse("manifest root tree does not match Git");
    }
    let entry = manifest.get("entry")?.str()?.to_owned();
    git::validate_path(&entry)?;
    if !entry.ends_with(".py") {
        return refuse("slice entry must be a Python file");
    }
    let assets = manifest.get("assets")?.array_ref()?;
    let mut prev_asset: Option<&str> = None;
    for asset in assets {
        let p = asset.str()?;
        git::validate_path(p)?;
        if prev_asset.is_some_and(|last| p <= last) {
            return refuse("slice assets must be sorted and unique");
        }
        prev_asset = Some(p);
    }
    let entries = manifest.get("entries")?.array_ref()?;
    if entries.is_empty() || entries.len() > MAX_FILES {
        return refuse("invalid executable slice entry count");
    }
    let mut result = Vec::new();
    let mut previous = String::new();
    let mut total = 0usize;
    let mut paths = BTreeSet::new();
    for item in entries {
        let path = item.get("path")?.str()?.to_owned();
        git::validate_path(&path)?;
        if path <= previous {
            return refuse("slice source paths must be strictly ordered");
        }
        previous = path.clone();
        paths.insert(path.clone());
        let blob = git::oid(item.get("blob")?.str()?)?;
        let sha = check_digest(item.get("sha256")?)?;
        let bytes = item.get("bytes")?.integer()?;
        if bytes < 0 || bytes > MAX_SOURCE_BYTES as i64 {
            return refuse("source blob size out of bounds");
        }
        total = total
            .checked_add(bytes as usize)
            .ok_or("source bytes overflow")?;
        if total > MAX_SOURCE_BYTES {
            return refuse("slice source budget exceeded");
        }
        let mode = git::text(repo, &["ls-tree", "HEAD", "--", &path])?;
        if !mode.starts_with("100644 blob ") {
            return refuse(format!(
                "source is not a regular tracked nonexecutable blob: {path}"
            ));
        }
        if git::blob(repo, "HEAD", &path)? != blob {
            return refuse(format!("source Git blob identity differs: {path}"));
        }
        let declared_size = git::text(repo, &["cat-file", "-s", &blob])?
            .parse::<usize>()
            .map_err(|_| "invalid Git blob size")?;
        if declared_size != bytes as usize {
            return refuse("Git blob size differs from manifest");
        }
        let data = git::command(repo, &["cat-file", "blob", &blob])?;
        if data.len() != bytes as usize || digest::sha256(&data) != sha {
            return refuse(format!("source bytes differ from manifest: {path}"));
        }
        result.push(Source {
            path,
            blob,
            sha256: sha,
            bytes,
        });
    }
    if !paths.contains(&entry)
        || assets
            .iter()
            .any(|a| !paths.contains(a.str().unwrap_or("")))
    {
        return refuse("slice entry or declared asset absent from source closure");
    }
    Ok((view, commit, tree, entry, result))
}

fn validated_receipt(receipt: &Value, view: &str, commit: &str, entry: &str) -> Result<String> {
    if receipt.get("schema")?.str()? != "excession.slice-observation.v1"
        || receipt.get("scope")?.str()? != "FINITE_TRUSTED_LOCAL_EXECUTION"
        || receipt.get("view_id")?.str()? != view
        || receipt.get("commit")?.str()? != commit
        || receipt.get("entry")?.str()? != entry
    {
        return refuse("receipt is not bound to the executable slice");
    }
    let id = check_digest(receipt.get("receipt_id")?)?;
    if without_identity(receipt, "receipt_id")?.sha256() != id {
        return refuse("receipt_id does not bind canonical contents");
    }
    let exit = receipt.get("exit_code")?.integer()?;
    let status = receipt.get("status")?.str()?;
    if !(-255..=255).contains(&exit)
        || (exit == 0 && status != "OBSERVED_PASS")
        || (exit != 0 && status != "OBSERVED_FAIL")
    {
        return refuse("inconsistent execution exit and status");
    }
    check_digest(receipt.get("stdout_sha256")?)?;
    check_digest(receipt.get("stderr_sha256")?)?;
    let python = receipt.get("python")?.str()?;
    if python.is_empty() || python.len() > 128 {
        return refuse("invalid runtime identity");
    }
    Ok(id)
}

/// Record a source-bound, UNSIGNED receipt or independently replay its storage.
/// A successful audit does not authenticate the Python execution environment.
pub fn evidence(
    repo: &Path,
    manifest_path: &Path,
    receipt_path: &Path,
    db: &Db,
    audit: bool,
) -> Result<Value> {
    let manifest = read_json(manifest_path, MAX_MANIFEST_BYTES)?;
    let receipt = read_json(receipt_path, MAX_RECEIPT_BYTES)?;
    let (view, commit, tree, entry, sources) = source_rows(repo, &manifest)?;
    let id = validated_receipt(&receipt, &view, &commit, &entry)?;
    let manifest_json = manifest.canonical();
    let receipt_json = receipt.canonical();
    let state = "UNSIGNED_LOCAL_OBSERVATION";
    let expected = vec![vec![
        Some(id.clone()),
        Some(view.clone()),
        Some(commit.clone()),
        Some(tree.clone()),
        Some(entry.clone()),
        Some(manifest_json.clone()),
        Some(receipt_json.clone()),
        Some(state.to_owned()),
    ]];
    if audit {
        let actual = db.query(
            "SELECT receipt_id,view_id,commit_oid,root_tree,entry,manifest_json,receipt_json,state FROM executable_slice_receipt WHERE receipt_id=?",
            &[Param::Text(&id)],
        )?;
        if actual != expected {
            return refuse("independent slice receipt audit differed from SQL");
        }
        let rows = db.query(
            "SELECT receipt_id,path,blob_oid,sha256,byte_count FROM executable_slice_source WHERE receipt_id=? ORDER BY path",
            &[Param::Text(&id)],
        )?;
        if rows != sources.iter().map(|s| s.row(&id)).collect::<Vec<_>>() {
            return refuse("independent source closure audit differed from SQL");
        }
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query(
                    "SELECT receipt_id FROM executable_slice_receipt WHERE receipt_id=?",
                    &[Param::Text(&id)],
                )?
                .is_empty()
            {
                return refuse("duplicate slice receipt; use audit for existing evidence");
            }
            db.execute(
                "INSERT INTO executable_slice_receipt VALUES(?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&id),
                    Param::Text(&view),
                    Param::Text(&commit),
                    Param::Text(&tree),
                    Param::Text(&entry),
                    Param::Text(&manifest_json),
                    Param::Text(&receipt_json),
                    Param::Text(state),
                ],
            )?;
            for source in &sources {
                db.execute(
                    "INSERT INTO executable_slice_source VALUES(?,?,?,?,?)",
                    &[
                        Param::Text(&id),
                        Param::Text(&source.path),
                        Param::Text(&source.blob),
                        Param::Text(&source.sha256),
                        Param::Integer(source.bytes),
                    ],
                )?;
            }
            Ok(())
        })?;
    }
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("schema", Value::string("excession.slice-evidence-store.v1")),
        ("receipt_id", Value::string(id)),
        ("view_id", Value::string(view)),
        ("source_commit", Value::string(commit)),
        ("source_count", Value::number(sources.len())),
        ("state", Value::string(state)),
        ("independently_audited", Value::Bool(audit)),
        ("target_behavior_verified", Value::Bool(false)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_noncanonical_digests() {
        assert!(!hex64("ab"));
        assert!(!hex64(&"A".repeat(64)));
        assert!(hex64(&"f".repeat(64)));
    }
    #[test]
    fn reject_modified_receipt_content() {
        let mut receipt = obj(&[
            ("schema", Value::string("excession.slice-observation.v1")),
            ("scope", Value::string("FINITE_TRUSTED_LOCAL_EXECUTION")),
            ("view_id", Value::string("1".repeat(64))),
            ("commit", Value::string("1".repeat(40))),
            ("entry", Value::string("check.py")),
            ("status", Value::string("OBSERVED_PASS")),
            ("exit_code", Value::number(0)),
            ("stdout_sha256", Value::string("2".repeat(64))),
            ("stderr_sha256", Value::string("3".repeat(64))),
            ("python", Value::string("3.13")),
        ]);
        let id = receipt.sha256();
        receipt
            .entries_mut()
            .unwrap()
            .insert("receipt_id".into(), Value::string(id));
        assert!(validated_receipt(&receipt, &"1".repeat(64), &"1".repeat(40), "check.py").is_ok());
        receipt
            .entries_mut()
            .unwrap()
            .insert("status".into(), Value::string("OBSERVED_FAIL"));
        assert!(validated_receipt(&receipt, &"1".repeat(64), &"1".repeat(40), "check.py").is_err());
    }
    #[test]
    fn git_sqlite_roundtrip_rejects_forged_rows() {
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("excession-slice-{}-{nonce}", std::process::id()));
        let repo = dir.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let command = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        };
        command(&["init", "-q"]);
        command(&["config", "user.name", "Slice Test"]);
        command(&["config", "user.email", "test@example.invalid"]);
        fs::write(repo.join("check.py"), b"assert 1 + 1 == 2\n").unwrap();
        command(&["add", "check.py"]);
        command(&["commit", "-qm", "fixture"]);
        let commit = git::rev(&repo, "HEAD").unwrap();
        let tree = git::rev(&repo, "HEAD^{tree}").unwrap();
        let blob = git::blob(&repo, "HEAD", "check.py").unwrap();
        let data = fs::read(repo.join("check.py")).unwrap();
        let source = obj(&[
            ("path", Value::string("check.py")),
            ("blob", Value::string(blob)),
            ("bytes", Value::number(data.len())),
            ("sha256", Value::string(digest::sha256(&data))),
        ]);
        let mut manifest = obj(&[
            ("schema", Value::string("excession.executable-slice.v1")),
            ("commit", Value::string(commit.clone())),
            ("tree", Value::string(tree)),
            ("entry", Value::string("check.py")),
            ("assets", Value::array(Vec::new())),
            ("entries", Value::array(vec![source])),
            ("scope", Value::string("STATIC_IMPORT_CLOSURE_ONLY")),
            ("semantic_equivalence", Value::string("UNVERIFIED")),
        ]);
        let view = manifest.sha256();
        manifest
            .entries_mut()
            .unwrap()
            .insert("view_id".into(), Value::string(&view));
        let mut receipt = obj(&[
            ("schema", Value::string("excession.slice-observation.v1")),
            ("view_id", Value::string(view)),
            ("commit", Value::string(commit)),
            ("entry", Value::string("check.py")),
            ("python", Value::string("3.13.5")),
            ("exit_code", Value::number(0)),
            ("stdout_sha256", Value::string(digest::sha256(b"ok"))),
            ("stderr_sha256", Value::string(digest::sha256(b""))),
            ("status", Value::string("OBSERVED_PASS")),
            ("scope", Value::string("FINITE_TRUSTED_LOCAL_EXECUTION")),
        ]);
        let id = receipt.sha256();
        receipt
            .entries_mut()
            .unwrap()
            .insert("receipt_id".into(), Value::string(id.clone()));
        let manifest_file = dir.join("manifest.json");
        let receipt_file = dir.join("receipt.json");
        fs::write(&manifest_file, manifest.canonical()).unwrap();
        fs::write(&receipt_file, receipt.canonical()).unwrap();
        let db = Db::open(&dir.join("evidence.sqlite")).unwrap();
        assert!(evidence(&repo, &manifest_file, &receipt_file, &db, false).is_ok());
        assert!(evidence(&repo, &manifest_file, &receipt_file, &db, true).is_ok());
        assert!(evidence(&repo, &manifest_file, &receipt_file, &db, false).is_err());
        db.execute(
            "UPDATE executable_slice_source SET blob_oid=? WHERE receipt_id=?",
            &[Param::Text(&"0".repeat(40)), Param::Text(&id)],
        )
        .unwrap();
        assert!(evidence(&repo, &manifest_file, &receipt_file, &db, true).is_err());
        drop(db);
        fs::remove_dir_all(dir).unwrap();
    }
}
