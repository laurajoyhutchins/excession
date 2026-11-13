//! Artifact-first Ghidra Function ID and BSim observations, joined to pinned Git
//! source candidates without claiming the two are causally or semantically equivalent.
use crate::evidence::{
    build,
    identity::{SourceRef, Subject},
};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fs;
use std::path::Path;

const REPORT_SCHEMA: &str = "excession.ghidra-matches.v1";
const BINDING_SCHEMA: &str = "excession.ghidra-source-binding.v1";
const MAX_MATCHES: usize = 512;
const MAX_REPORT: usize = 2 * 1024 * 1024;
const MAX_NATIVE: usize = 2 * 1024 * 1024;
const MAX_RESULTS: usize = 32;

fn exact(value: &Value, keys: &[&str]) -> Result<()> {
    let fields = value.entries()?;
    if fields.len() != keys.len() || keys.iter().any(|key| !fields.contains_key(*key)) {
        return refuse("Ghidra evidence has missing or unsupported fields");
    }
    Ok(())
}
fn text(value: &Value, max: usize) -> Result<&str> {
    let v = value.str()?;
    if v.is_empty() || v.len() > max || v.chars().any(char::is_control) {
        return refuse("Ghidra evidence string is missing, oversized, or contains controls");
    }
    Ok(v)
}
fn hex(value: &str, bytes: usize) -> Result<()> {
    if value.len() != bytes * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return refuse("Ghidra identity requires lowercase fixed-width hex");
    }
    Ok(())
}
fn decimal(value: &Value, min: f64, max: f64) -> Result<()> {
    let raw = text(value, 32)?;
    let parsed = raw.parse::<f64>().map_err(|_| "invalid provider score")?;
    if !parsed.is_finite() || parsed < min || parsed > max {
        return refuse("provider score outside supported range");
    }
    Ok(())
}
fn database(value: &Value) -> Result<()> {
    exact(value, &["basis", "identity"])?;
    let basis = value.get("basis")?.str()?;
    let identity = text(value.get("identity")?, 256)?;
    match basis {
        "CONTENT_SHA256" => hex(identity, 32),
        "DECLARED_NAME" => Ok(()),
        _ => refuse("unrecognized Ghidra database identity basis"),
    }
}
fn optional_label(value: &Value) -> Result<()> {
    match value {
        Value::Null => Ok(()),
        Value::String(s) if s.len() <= 256 && !s.chars().any(char::is_control) => Ok(()),
        _ => refuse("unsupported Ghidra optional library metadata"),
    }
}
fn candidate(value: &Value) -> Result<String> {
    match value.get("engine")?.str()? {
        "FID" => {
            exact(
                value,
                &[
                    "engine",
                    "database",
                    "library_family",
                    "library_version",
                    "library_variant",
                    "function",
                ],
            )?;
            database(value.get("database")?)?;
            for key in ["library_family", "function"] {
                text(value.get(key)?, 256)?;
            }
            for key in ["library_version", "library_variant"] {
                optional_label(value.get(key)?)?;
            }
        }
        "BSIM" => {
            exact(
                value,
                &["engine", "database", "executable_md5", "entry", "function"],
            )?;
            database(value.get("database")?)?;
            hex(text(value.get("executable_md5")?, 32)?, 16)?;
            text(value.get("entry")?, 128)?;
            text(value.get("function")?, 256)?;
        }
        _ => return refuse("Ghidra candidate engine must be FID or BSIM"),
    }
    let mut bytes = b"excession.ghidra-candidate.v1\0".to_vec();
    bytes.extend_from_slice(value.canonical().as_bytes());
    Ok(digest::sha256(&bytes))
}
fn match_record(value: &Value) -> Result<(String, String)> {
    exact(value, &["entry", "candidate", "evidence"])?;
    let entry = text(value.get("entry")?, 128)?;
    let key = candidate(value.get("candidate")?)?;
    let evidence = value.get("evidence")?;
    match value.get("candidate")?.get("engine")?.str()? {
        "FID" => {
            exact(evidence, &["engine", "overall_score", "match_mode"])?;
            if evidence.get("engine")?.str()? != "FID" {
                return refuse("FID score engine mismatch");
            }
            decimal(evidence.get("overall_score")?, 0.0, f64::MAX)?;
            text(evidence.get("match_mode")?, 128)?;
        }
        "BSIM" => {
            exact(evidence, &["engine", "similarity", "confidence"])?;
            if evidence.get("engine")?.str()? != "BSIM" {
                return refuse("BSim score engine mismatch");
            }
            decimal(evidence.get("similarity")?, 0.0, 1.0)?;
            decimal(evidence.get("confidence")?, 0.0, f64::MAX)?;
        }
        _ => unreachable!(),
    }
    Ok((entry.to_owned(), key))
}
fn report(
    value: &Value,
    sha: &str,
    size: u64,
    native_sha: &str,
) -> Result<Vec<(String, String, String, Value)>> {
    exact(value, &["schema", "artifact", "producer", "matches"])?;
    if value.get("schema")?.str()? != REPORT_SCHEMA {
        return refuse("unsupported Ghidra evidence schema");
    }
    let artifact = value.get("artifact")?;
    exact(artifact, &["sha256", "size_bytes"])?;
    if artifact.get("sha256")?.str()? != sha
        || artifact.get("size_bytes")?.integer()?
            != i64::try_from(size).map_err(|_| "artifact size too large")?
    {
        return refuse("Ghidra report artifact bytes disagree with local artifact");
    }
    let producer = value.get("producer")?;
    exact(
        producer,
        &[
            "tool",
            "version",
            "analysis_profile_sha256",
            "native_receipt_sha256",
        ],
    )?;
    if producer.get("tool")?.str()? != "Ghidra"
        || producer.get("native_receipt_sha256")?.str()? != native_sha
    {
        return refuse("Ghidra producer native receipt mismatch");
    }
    text(producer.get("version")?, 128)?;
    hex(text(producer.get("analysis_profile_sha256")?, 64)?, 32)?;
    let matches = value.get("matches")?.array_ref()?;
    if matches.is_empty() || matches.len() > MAX_MATCHES {
        return refuse("Ghidra report must contain 1..512 matches");
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for item in matches {
        let (entry, key) = match_record(item)?;
        let canonical = item.canonical();
        if !seen.insert(canonical.clone()) {
            return refuse("duplicate Ghidra match");
        }
        output.push((entry, key, canonical, item.clone()));
    }
    Ok(output)
}
fn bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(|e| format!("Ghidra input metadata: {e}"))?;
    if !meta.is_file() || meta.len() > max as u64 {
        return refuse("Ghidra input must be a bounded regular file");
    }
    let bytes = fs::read(path).map_err(|e| format!("Ghidra input read: {e}"))?;
    if bytes.is_empty() || bytes.len() > max {
        return refuse("Ghidra input size outside budget");
    }
    Ok(bytes)
}
fn schema(db: &Db) -> Result<()> {
    db.exec(
        "CREATE TABLE IF NOT EXISTS evidence_ghidra_report_v1 (
           report_id TEXT PRIMARY KEY,
           artifact_subject_id TEXT NOT NULL,
           native_sha256 TEXT NOT NULL,
           raw_report TEXT NOT NULL,
           native_receipt TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS evidence_ghidra_match_v1 (
           match_id TEXT PRIMARY KEY,
           report_id TEXT NOT NULL REFERENCES evidence_ghidra_report_v1(report_id),
           artifact_subject_id TEXT NOT NULL,
           function_entry TEXT NOT NULL,
           candidate_id TEXT NOT NULL,
           raw_match TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS evidence_ghidra_function_lookup
           ON evidence_ghidra_match_v1(artifact_subject_id,function_entry,match_id);
         CREATE INDEX IF NOT EXISTS evidence_ghidra_candidate_lookup
           ON evidence_ghidra_match_v1(candidate_id,match_id);
         CREATE TABLE IF NOT EXISTS evidence_ghidra_source_v1 (
           binding_id TEXT PRIMARY KEY,
           candidate_id TEXT NOT NULL,
           raw_binding TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS evidence_ghidra_source_lookup
           ON evidence_ghidra_source_v1(candidate_id,binding_id);
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_report_immutable_update
           BEFORE UPDATE ON evidence_ghidra_report_v1 BEGIN SELECT RAISE(ABORT,'immutable Ghidra report'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_report_immutable_delete
           BEFORE DELETE ON evidence_ghidra_report_v1 BEGIN SELECT RAISE(ABORT,'immutable Ghidra report'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_match_immutable_update
           BEFORE UPDATE ON evidence_ghidra_match_v1 BEGIN SELECT RAISE(ABORT,'immutable Ghidra match'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_match_immutable_delete
           BEFORE DELETE ON evidence_ghidra_match_v1 BEGIN SELECT RAISE(ABORT,'immutable Ghidra match'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_source_immutable_update
           BEFORE UPDATE ON evidence_ghidra_source_v1 BEGIN SELECT RAISE(ABORT,'immutable source association'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_ghidra_source_immutable_delete
           BEFORE DELETE ON evidence_ghidra_source_v1 BEGIN SELECT RAISE(ABORT,'immutable source association'); END;",
    )
}
fn report_id(raw: &str, native_sha: &str) -> String {
    let mut data = b"excession.ghidra-report.v1\0".to_vec();
    data.extend_from_slice(raw.as_bytes());
    data.extend_from_slice(native_sha.as_bytes());
    digest::sha256(&data)
}
fn match_id(report_id: &str, canonical: &str) -> String {
    let mut data = b"excession.ghidra-match.v1\0".to_vec();
    data.extend_from_slice(report_id.as_bytes());
    data.extend_from_slice(canonical.as_bytes());
    digest::sha256(&data)
}
fn source_doc(source: &SourceRef) -> Value {
    obj(&[
        ("commit", Value::string(&source.commit)),
        ("tree", Value::string(&source.tree)),
        ("path", Value::string(&source.path)),
        ("blob", Value::string(&source.blob)),
        ("mode", Value::string(&source.mode)),
    ])
}
fn parse_source(value: &Value) -> Result<SourceRef> {
    exact(value, &["commit", "tree", "path", "blob", "mode"])?;
    let source = SourceRef {
        commit: value.get("commit")?.str()?.to_owned(),
        tree: value.get("tree")?.str()?.to_owned(),
        path: value.get("path")?.str()?.to_owned(),
        blob: value.get("blob")?.str()?.to_owned(),
        mode: value.get("mode")?.str()?.to_owned(),
    };
    source.validate()?;
    Ok(source)
}
pub fn import(db: &Db, artifact: &Path, normalized: &Path, native: &Path) -> Result<Value> {
    let (sha, size) = build::read_artifact(artifact)?;
    let subject_id = Subject::Artifact {
        sha256: sha.clone(),
        size_bytes: size,
    }
    .id()?;
    let normalized_bytes = bounded(normalized, MAX_REPORT)?;
    let raw = String::from_utf8(normalized_bytes).map_err(|_| "Ghidra report must be UTF-8")?;
    let parsed = json::parse(raw.as_bytes())?;
    let native_bytes = bounded(native, MAX_NATIVE)?;
    let native_raw =
        String::from_utf8(native_bytes).map_err(|_| "Ghidra native receipt must be UTF-8")?;
    let native_sha = digest::sha256(native_raw.as_bytes());
    let records = report(&parsed, &sha, size, &native_sha)?;
    let id = report_id(&raw, &native_sha);
    schema(db)?;
    db.transaction(|db| {
        db.execute(
            "INSERT OR IGNORE INTO evidence_ghidra_report_v1
             (report_id,artifact_subject_id,native_sha256,raw_report,native_receipt) VALUES (?,?,?,?,?)",
            &[Param::Text(&id),Param::Text(&subject_id),Param::Text(&native_sha),
              Param::Text(&raw),Param::Text(&native_raw)],
        )?;
        for (entry,key,canonical,_) in &records {
            let mid = match_id(&id,canonical);
            db.execute(
                "INSERT OR IGNORE INTO evidence_ghidra_match_v1
                 (match_id,report_id,artifact_subject_id,function_entry,candidate_id,raw_match)
                 VALUES (?,?,?,?,?,?)",
                &[Param::Text(&mid),Param::Text(&id),Param::Text(&subject_id),
                  Param::Text(entry),Param::Text(key),Param::Text(canonical)],
            )?;
        }
        Ok(())
    })?;
    Ok(obj(&[
        ("schema", Value::string("excession.ghidra-import.v1")),
        ("report_id", Value::string(id)),
        ("artifact_subject_id", Value::string(subject_id)),
        ("match_count", Value::number(records.len())),
        ("native_receipt_sha256", Value::string(native_sha)),
        ("artifact_bytes_verified", Value::Bool(true)),
        ("producer_execution_attested", Value::Bool(false)),
        (
            "semantic_source_correspondence",
            Value::string("NOT_ESTABLISHED"),
        ),
    ]))
}

/// Associate a provider's *native candidate identity* with immutable Git source
/// objects. Registration does not assert that the code generated any binary.
pub fn bind_source(
    db: &Db,
    repo: &Path,
    revision: &str,
    path: &str,
    candidate_file: &Path,
    symbol: &str,
) -> Result<Value> {
    let candidate_bytes = bounded(candidate_file, 16 * 1024)?;
    let value = json::parse(&candidate_bytes)?;
    let key = candidate(&value)?;
    text(&Value::string(symbol), 1024)?;
    let canonical_repo = fs::canonicalize(repo).map_err(|e| format!("repo path: {e}"))?;
    git::verify_repo(&canonical_repo)?;
    let commit = git::rev(&canonical_repo, &format!("{revision}^{{commit}}"))?;
    let source = SourceRef::from_git(&canonical_repo, &commit, path)?;
    let document = obj(&[
        ("schema", Value::string(BINDING_SCHEMA)),
        ("candidate", value),
        ("candidate_id", Value::string(&key)),
        ("source", source_doc(&source)),
        (
            "repository_path",
            Value::string(canonical_repo.to_str().ok_or("non-UTF8 repo path")?),
        ),
        ("symbol", Value::string(symbol)),
        (
            "relationship",
            Value::string("DECLARED_CANDIDATE_NOT_PROVEN"),
        ),
    ]);
    let id = document.sha256();
    schema(db)?;
    db.execute(
        "INSERT OR IGNORE INTO evidence_ghidra_source_v1(binding_id,candidate_id,raw_binding)
         VALUES (?,?,?)",
        &[
            Param::Text(&id),
            Param::Text(&key),
            Param::Text(&document.canonical()),
        ],
    )?;
    let mut result = document;
    result
        .entries_mut()?
        .insert("binding_id".into(), Value::string(id));
    Ok(result)
}
fn checked_binding(id: &str, expected_key: &str, raw: &str) -> Result<Value> {
    let document = json::parse(raw.as_bytes())?;
    if document.canonical() != raw || document.sha256() != id {
        return refuse("source binding content identity changed");
    }
    exact(
        &document,
        &[
            "schema",
            "candidate",
            "candidate_id",
            "source",
            "repository_path",
            "symbol",
            "relationship",
        ],
    )?;
    if document.get("schema")?.str()? != BINDING_SCHEMA
        || document.get("relationship")?.str()? != "DECLARED_CANDIDATE_NOT_PROVEN"
        || document.get("candidate_id")?.str()? != expected_key
        || candidate(document.get("candidate")?)? != expected_key
    {
        return refuse("source association is not a supported candidate binding");
    }
    let repo = Path::new(text(document.get("repository_path")?, 4096)?);
    let source = parse_source(document.get("source")?)?;
    text(document.get("symbol")?, 1024)?;
    // Never silently promote previously verified objects after checkout changes
    // or when the local repository no longer exists at this path.
    source.verify_git(repo)?;
    let mut result = document;
    result
        .entries_mut()?
        .insert("binding_id".into(), Value::string(id));
    result
        .entries_mut()?
        .insert("source_git_replayed".into(), Value::Bool(true));
    result.entries_mut()?.insert(
        "binary_source_equivalence".into(),
        Value::string("NOT_ESTABLISHED"),
    );
    Ok(result)
}
fn checked_report(db: &Db, id: &str, subject_id: &str, sha: &str, size: u64) -> Result<Value> {
    let rows = db.query(
        "SELECT artifact_subject_id,native_sha256,raw_report,native_receipt
         FROM evidence_ghidra_report_v1 WHERE report_id=?",
        &[Param::Text(id)],
    )?;
    if rows.len() != 1 {
        return refuse("missing Ghidra producer report");
    }
    let row = &rows[0];
    let stored_subject = row[0].as_deref().ok_or("missing Ghidra subject")?;
    let native_sha = row[1].as_deref().ok_or("missing native SHA")?;
    let raw = row[2].as_deref().ok_or("missing Ghidra report bytes")?;
    let native = row[3]
        .as_deref()
        .ok_or("missing Ghidra raw producer receipt")?;
    if stored_subject != subject_id
        || digest::sha256(native.as_bytes()) != native_sha
        || report_id(raw, native_sha) != id
    {
        return refuse("Ghidra report identity, native receipt, or artifact association changed");
    }
    let parsed = json::parse(raw.as_bytes())?;
    report(&parsed, sha, size, native_sha)?;
    Ok(parsed)
}
pub fn candidates(db: &Db, artifact: &Path, entry: &str, limit: usize) -> Result<Value> {
    if !(1..=MAX_RESULTS).contains(&limit) {
        return refuse("Ghidra candidate limit must be 1..32");
    }
    text(&Value::string(entry), 128)?;
    let (sha, size) = build::read_artifact(artifact)?;
    let subject_id = Subject::Artifact {
        sha256: sha.clone(),
        size_bytes: size,
    }
    .id()?;
    schema(db)?;
    db.read_transaction(|db| {
        let total = db
            .scalar(
                "SELECT count(*) FROM evidence_ghidra_match_v1
             WHERE artifact_subject_id=? AND function_entry=?",
                &[Param::Text(&subject_id), Param::Text(entry)],
            )?
            .parse::<usize>()
            .map_err(|_| "invalid Ghidra corpus match count")?;
        let matches = db.query(
            "SELECT match_id,report_id,candidate_id,raw_match FROM evidence_ghidra_match_v1
             WHERE artifact_subject_id=? AND function_entry=?
             ORDER BY match_id LIMIT ?",
            &[
                Param::Text(&subject_id),
                Param::Text(entry),
                Param::Integer(limit as i64),
            ],
        )?;
        let mut output = Vec::new();
        // Each producer receipt and candidate binding is audited once per query.
        let mut report_cache = BTreeMap::new();
        let mut binding_cache = BTreeMap::new();
        for row in matches {
            let mid = row[0].as_deref().ok_or("missing Ghidra match ID")?;
            let report_id_ref = row[1].as_deref().ok_or("missing Ghidra report ID")?;
            let candidate_id = row[2]
                .as_deref()
                .ok_or("missing Ghidra candidate identity")?;
            let raw = row[3].as_deref().ok_or("missing Ghidra match record")?;
            let native_report = match report_cache.entry(report_id_ref.to_owned()) {
                Entry::Occupied(existing) => existing.into_mut(),
                Entry::Vacant(empty) => empty.insert(
                    checked_report(db, report_id_ref, &subject_id, &sha, size)?
                ),
            };
            let checked = json::parse(raw.as_bytes())?;
            let (actual_entry, actual_candidate) = match_record(&checked)?;
            let found = native_report
                .get("matches")?
                .array_ref()?
                .iter()
                .any(|item| item == &checked);
            if checked.canonical() != raw
                || match_id(report_id_ref, raw) != mid
                || !found
                || actual_entry != entry
                || actual_candidate != candidate_id
            {
                return refuse("Ghidra match index disagrees with immutable native report");
            }
            let known = match binding_cache.entry(candidate_id.to_owned()) {
                Entry::Occupied(existing) => existing.get().clone(),
                Entry::Vacant(empty) => {
                    let sources = db.query(
                        "SELECT binding_id,raw_binding FROM evidence_ghidra_source_v1
                         WHERE candidate_id=? ORDER BY binding_id LIMIT 16",
                        &[Param::Text(candidate_id)],
                    )?;
                    let mut verified = Vec::new();
                    for binding in sources {
                        verified.push(checked_binding(
                            binding[0].as_deref().ok_or("missing binding ID")?,
                            candidate_id,
                            binding[1].as_deref().ok_or("missing source binding")?,
                        )?);
                    }
                    empty.insert(verified).clone()
                }
            };
            output.push(obj(&[
                ("match_id", Value::string(mid)),
                ("report_id", Value::string(report_id_ref)),
                ("candidate_id", Value::string(candidate_id)),
                ("native_observation", checked),
                ("source_candidates", Value::array(known)),
                (
                    "recognition_authority",
                    Value::string("PRODUCER_SUPPLIED_UNATTESTED"),
                ),
                (
                    "binary_source_correspondence",
                    Value::string("NOT_ESTABLISHED"),
                ),
            ]));
        }
        Ok(obj(&[
            (
                "schema",
                Value::string("excession.ghidra-corpus-candidates.v1"),
            ),
            ("artifact_subject_id", Value::string(subject_id)),
            ("function_entry", Value::string(entry)),
            ("total_matches", Value::number(total)),
            ("results_truncated", Value::Bool(total > limit)),
            ("matches", Value::array(output)),
            ("semantic_equivalence", Value::string("NOT_ESTABLISHED")),
        ]))
    })
}
