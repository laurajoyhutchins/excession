//! First-class REA artifact evidence in the existing SQLite authority.
//! Foreign native evidence and its uncertainty remain distinct from Git facts.
use crate::evidence::{build, identity::Subject};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, refuse};
use serde_json::{Map, Value as JsonValue};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_BUNDLE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORDS: usize = 128;
const MAX_UNKNOWNS: usize = 256;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub struct BuildLink<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub snapshot: &'a str,
}
pub struct ImportSpec<'a> {
    pub artifact: &'a Path,
    pub bundle: &'a Path,
    pub build: Option<BuildLink<'a>>,
}

fn native_id(value: &str) -> Result<()> {
    if value.len() != 67
        || !value.starts_with("ev_")
        || !value[3..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return refuse("invalid REA native evidence identity");
    }
    Ok(())
}
fn sha256(value: &str) -> Result<()> {
    Subject::Configuration {
        sha256: value.to_owned(),
    }
    .validate()
}
fn bounded_bundle(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).map_err(|e| format!("REA bundle open: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BUNDLE_BYTES as u64 {
        return refuse("REA evidence bundle is not a bounded regular file");
    }
    let bytes = std::fs::read(path).map_err(|e| format!("REA bundle read: {e}"))?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        return refuse("REA evidence bundle exceeds byte limit");
    }
    json::parse(&bytes)?; // Strict duplicate-key and depth validation.
    let original = String::from_utf8(bytes).map_err(|_| "REA bundle is not UTF-8 JSON")?;
    Ok(original)
}

/// RFC 8785 for the supported safe-integer subset. Refuse floating-point or
/// unsafe integer numbers rather than falsely validating foreign Evidence IDs.
fn jcs(value: &JsonValue, output: &mut String) -> Result<()> {
    match value {
        JsonValue::Null => output.push_str("null"),
        JsonValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        JsonValue::String(value) => {
            output.push_str(&serde_json::to_string(value).map_err(|e| e.to_string())?)
        }
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                if value.unsigned_abs() > MAX_SAFE_INTEGER {
                    return refuse("REA integer outside exact RFC8785 support");
                }
                output.push_str(&value.to_string());
            } else if let Some(value) = value.as_u64() {
                if value > MAX_SAFE_INTEGER {
                    return refuse("REA integer outside exact RFC8785 support");
                }
                output.push_str(&value.to_string());
            } else {
                return refuse("REA non-integer numeric value requires an external JCS validator");
            }
        }
        JsonValue::Array(items) => {
            output.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                jcs(item, output)?;
            }
            output.push(']');
        }
        JsonValue::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            output.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).map_err(|e| e.to_string())?);
                output.push(':');
                jcs(
                    map.get(key.as_str()).ok_or("missing JCS map entry")?,
                    output,
                )?;
            }
            output.push('}');
        }
    }
    Ok(())
}
const SEMANTIC_FIELDS: &[&str] = &[
    "provider",
    "analysis_profile",
    "predicate_type",
    "operation",
    "parameters",
    "raw_result",
    "normalized_result",
    "confidence",
    "authority",
    "environment",
    "limitations",
    "relationship_coverage",
    "locations",
    "evidence_links",
];
fn object_field<'a>(value: &'a JsonValue, name: &str) -> Result<&'a JsonValue> {
    value
        .get(name)
        .ok_or_else(|| format!("REA record missing {name}"))
}
fn require_string<'a>(value: &'a JsonValue, name: &str) -> Result<&'a str> {
    object_field(value, name)?
        .as_str()
        .ok_or_else(|| format!("REA field {name} is not text"))
}
fn verified_record_id(record: &JsonValue) -> Result<String> {
    let map = record.as_object().ok_or("REA record must be JSON object")?;
    if map.len() > SEMANTIC_FIELDS.len() + 4 {
        return refuse("unsupported REA record extension");
    }
    for key in map.keys() {
        if key != "evidence_id"
            && key != "subject"
            && key != "caller_input_path"
            && key != "caller_input_paths"
            && !SEMANTIC_FIELDS.contains(&key.as_str())
        {
            return refuse("unrecognized REA native record field");
        }
    }
    let id = require_string(record, "evidence_id")?;
    native_id(id)?;
    let subject = object_field(record, "subject")?;
    let semantic_subject = if subject.is_null() {
        JsonValue::Null
    } else {
        let map = subject.as_object().ok_or("malformed REA subject")?;
        if map.len() != 5
            || require_string(subject, "name")?.is_empty()
            || require_string(subject, "local_path")?.is_empty()
        {
            return refuse("REA artifact subject must include original display and digest fields");
        }
        let digest = object_field(subject, "digest")?;
        let artifact_id = require_string(digest, "sha256")?;
        sha256(artifact_id)?;
        if !object_field(digest, "sha256")?.is_string()
            || digest.as_object().is_none_or(|x| x.len() != 1)
        {
            return refuse("invalid REA subject digest");
        }
        require_string(subject, "format")?;
        let architecture = object_field(subject, "architecture")?;
        if !(architecture.is_null() || architecture.is_string()) {
            return refuse("invalid REA subject architecture");
        }
        let mut identity = Map::new();
        for key in ["digest", "format", "architecture"] {
            identity.insert(key.to_owned(), object_field(subject, key)?.clone());
        }
        JsonValue::Object(identity)
    };
    if !object_field(record, "parameters")?.is_object()
        || !object_field(record, "locations")?.is_array()
        || !object_field(record, "limitations")?.is_array()
        || !object_field(record, "evidence_links")?.is_array()
    {
        return refuse("malformed REA native evidence arrays");
    }
    require_string(record, "operation")?;
    require_string(record, "predicate_type")?;
    if !matches!(
        require_string(record, "confidence")?,
        "observed" | "derived" | "inferred"
    ) {
        return refuse("unsupported REA confidence");
    }
    if !matches!(
        require_string(record, "authority")?,
        "shipped-artifact"
            | "controlled-replay"
            | "historical-reference"
            | "external-service"
            | "analyst-inference"
    ) {
        return refuse("unsupported REA authority");
    }
    let provider = object_field(record, "provider")?;
    if provider.as_object().is_none_or(|m| m.len() != 3)
        || require_string(provider, "id")?.is_empty()
        || require_string(provider, "name")?.is_empty()
        || !(object_field(provider, "version")?.is_null()
            || object_field(provider, "version")?.is_string())
    {
        return refuse("malformed REA provider");
    }
    for link in object_field(record, "evidence_links")?
        .as_array()
        .ok_or("bad links")?
    {
        native_id(link.as_str().ok_or("bad native REA link")?)?;
    }
    let mut projection = Map::new();
    projection.insert("subject".to_owned(), semantic_subject);
    for key in SEMANTIC_FIELDS {
        projection.insert((*key).to_owned(), object_field(record, key)?.clone());
    }
    let mut canonical = String::new();
    jcs(&JsonValue::Object(projection), &mut canonical)?;
    if id != format!("ev_{}", digest::sha256(canonical.as_bytes())) {
        return refuse("REA native semantic Evidence ID mismatch");
    }
    Ok(id.to_owned())
}

struct Prepared {
    sha: String,
    size: u64,
    subject_id: String,
    raw: String,
    raw_sha: String,
    build_snapshot: Option<String>,
    selected: Vec<JsonValue>,
    ignored: usize,
    unknowns: usize,
    bundle_id: String,
}

fn prepare(artifact: &Path, raw: String, build_snapshot: Option<&str>) -> Result<Prepared> {
    json::parse(raw.as_bytes())?;
    let root: JsonValue = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let object = root.as_object().ok_or("REA bundle must be object")?;
    if object.len() != 7
        || ![
            "artifacts",
            "providers",
            "environments",
            "scenarios",
            "captures",
            "unknowns",
            "records",
        ]
        .iter()
        .all(|key| object.get(*key).is_some_and(JsonValue::is_array))
    {
        return refuse("expected actual REA Evidence v2 bundle manifests and records");
    }
    let (sha, size) = build::read_artifact(artifact)?;
    let subject_id = Subject::Artifact {
        sha256: sha.clone(),
        size_bytes: size,
    }
    .id()?;
    let artifacts = object_field(&root, "artifacts")?
        .as_array()
        .ok_or("bad manifest")?;
    if !artifacts.iter().any(|item| {
        item.get("digest")
            .and_then(|digest| digest.get("sha256"))
            .and_then(JsonValue::as_str)
            == Some(sha.as_str())
    }) {
        return refuse("REA bundle does not declare selected exact artifact digest");
    }
    let records = object_field(&root, "records")?
        .as_array()
        .ok_or("bad records")?;
    let unknowns = object_field(&root, "unknowns")?
        .as_array()
        .ok_or("bad unknowns")?
        .len();
    if records.is_empty() || records.len() > MAX_RECORDS || unknowns > MAX_UNKNOWNS {
        return refuse("REA record or residual-unknown budget exceeded");
    }
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    let mut previous: Option<String> = None;
    for record in records {
        let id = verified_record_id(record)?;
        if previous.as_deref().is_some_and(|last| last >= id.as_str()) || !seen.insert(id.clone()) {
            return refuse("REA native records must be sorted and unique");
        }
        previous = Some(id);
        let matches = record
            .get("subject")
            .and_then(|s| s.get("digest"))
            .and_then(|s| s.get("sha256"))
            .and_then(JsonValue::as_str)
            == Some(sha.as_str());
        if matches {
            selected.push(record.clone());
        }
    }
    if selected.is_empty() {
        return refuse("no REA records describe the exact selected artifact");
    }
    let raw_sha = digest::sha256(raw.as_bytes());
    let build_snapshot = build_snapshot.map(ToOwned::to_owned);
    if let Some(id) = &build_snapshot {
        sha256(id)?;
    }
    let mut bytes = b"excession.rea-artifact-evidence.v1\0".to_vec();
    for value in [
        &subject_id,
        &raw_sha,
        build_snapshot.as_deref().unwrap_or(""),
    ] {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    let bundle_id = digest::sha256(&bytes);
    Ok(Prepared {
        sha,
        size,
        subject_id,
        raw,
        raw_sha,
        build_snapshot,
        ignored: records.len() - selected.len(),
        selected,
        unknowns,
        bundle_id,
    })
}

fn verify_build(db: &Db, artifact: &Path, link: Option<&BuildLink<'_>>) -> Result<Option<String>> {
    let Some(link) = link else {
        return Ok(None);
    };
    let report = build::audit(db, link.repo, link.revision, artifact, link.snapshot)?;
    if !report.get("artifact_bytes_verified")?.bool()?
        || report.get("source_artifact_relation")?.str()? != "NOT_ESTABLISHED"
    {
        return refuse("REA build join requires independently replayed source/artifact bytes");
    }
    Ok(Some(link.snapshot.to_owned()))
}
pub fn init(db: &Db) -> Result<()> {
    crate::evidence_store::init(db)?;
    db.exec(
        "CREATE TABLE IF NOT EXISTS evidence_rea_bundle_v1 (
            bundle_id TEXT PRIMARY KEY,
            artifact_subject_id TEXT NOT NULL,
            artifact_sha256 TEXT NOT NULL,
            artifact_size INTEGER NOT NULL,
            raw_sha256 TEXT NOT NULL,
            build_snapshot_id TEXT,
            raw_json TEXT NOT NULL,
            FOREIGN KEY(build_snapshot_id) REFERENCES evidence_snapshot_v1(snapshot_id)
        );
        CREATE TRIGGER IF NOT EXISTS evidence_rea_bundle_no_update_v1
          BEFORE UPDATE ON evidence_rea_bundle_v1 BEGIN
            SELECT RAISE(ABORT, 'REA evidence immutable');
          END;
        CREATE TRIGGER IF NOT EXISTS evidence_rea_bundle_no_delete_v1
          BEFORE DELETE ON evidence_rea_bundle_v1 BEGIN
            SELECT RAISE(ABORT, 'REA evidence immutable');
          END;",
    )
}
fn report(data: &Prepared) -> Value {
    let ids = data
        .selected
        .iter()
        .map(|item| Value::string(item["evidence_id"].as_str().unwrap_or("")))
        .collect::<Vec<_>>();
    let mut result = obj(&[
        ("schema", Value::string("excession.rea-import.v1")),
        ("bundle_id", Value::string(&data.bundle_id)),
        ("bundle_sha256", Value::string(&data.raw_sha)),
        ("producer", Value::string("REA")),
        ("format", Value::string("EVIDENCE_V2_EXPORT")),
        ("artifact_sha256", Value::string(&data.sha)),
        ("artifact_size_bytes", Value::number(data.size)),
        ("artifact_subject_id", Value::string(&data.subject_id)),
        (
            "build_snapshot_id",
            data.build_snapshot
                .as_ref()
                .map_or(Value::Null, Value::string),
        ),
        ("native_record_ids", Value::array(ids)),
        ("imported_record_count", Value::number(data.selected.len())),
        ("unselected_record_count", Value::number(data.ignored)),
        ("unknown_revisions_retained", Value::number(data.unknowns)),
        (
            "native_identity",
            Value::string("REHASHED_SUPPORTED_RFC8785_INTEGER_DOMAIN"),
        ),
        ("raw_retention", Value::string("RETAIN_ORIGINAL_PAYLOAD")),
        (
            "artifact_binding",
            Value::string("INDEPENDENT_ARTIFACT_SHA256"),
        ),
        ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
        ("semantic_correctness", Value::string("NOT_ESTABLISHED")),
        (
            "unknown_graph_verification",
            Value::string("NOT_ESTABLISHED"),
        ),
        (
            "full_bundle_manifest_verification",
            Value::string("NOT_ESTABLISHED"),
        ),
        (
            "security",
            Value::string("IMPORT_ONLY_NO_UNTRUSTED_EXECUTION"),
        ),
    ]);
    let id = result.sha256();
    result
        .entries_mut()
        .expect("object")
        .insert("report_id".into(), Value::string(id));
    result
}
pub fn import(db: &Db, spec: &ImportSpec<'_>) -> Result<Value> {
    let link = verify_build(db, spec.artifact, spec.build.as_ref())?;
    let raw = bounded_bundle(spec.bundle)?;
    let prepared = prepare(spec.artifact, raw, link.as_deref())?;
    init(db)?;
    db.transaction(|db| {
        let existing = db.query(
            "SELECT artifact_subject_id,artifact_sha256,artifact_size,raw_sha256,build_snapshot_id,raw_json FROM evidence_rea_bundle_v1 WHERE bundle_id=?",
            &[Param::Text(&prepared.bundle_id)],
        )?;
        if !existing.is_empty() {
            if existing.len() != 1 {
                return refuse("duplicate immutable REA bundle");
            }
            let row = &existing[0];
            if row[0].as_deref() != Some(prepared.subject_id.as_str())
                || row[1].as_deref() != Some(prepared.sha.as_str())
                || row[2].as_deref() != Some(prepared.size.to_string().as_str())
                || row[3].as_deref() != Some(prepared.raw_sha.as_str())
                || row[4].as_deref() != prepared.build_snapshot.as_deref()
                || row[5].as_deref() != Some(prepared.raw.as_str()) {
                return refuse("existing immutable REA record is inconsistent");
            }
            return Ok(());
        }
        db.execute(
            "INSERT INTO evidence_rea_bundle_v1
             (bundle_id,artifact_subject_id,artifact_sha256,artifact_size,raw_sha256,build_snapshot_id,raw_json)
             VALUES(?,?,?,?,?,?,?)",
            &[
                Param::Text(&prepared.bundle_id),
                Param::Text(&prepared.subject_id),
                Param::Text(&prepared.sha),
                Param::Integer(i64::try_from(prepared.size).map_err(|_| "size overflow")?),
                Param::Text(&prepared.raw_sha),
                prepared.build_snapshot.as_deref().map_or(Param::Null, Param::Text),
                Param::Text(&prepared.raw),
            ],
        )
    })?;
    Ok(report(&prepared))
}
pub fn audit(db: &Db, artifact: &Path, id: &str, link: Option<&BuildLink<'_>>) -> Result<Value> {
    sha256(id)?;
    let rows = db.query(
        "SELECT artifact_subject_id,artifact_sha256,artifact_size,raw_sha256,build_snapshot_id,raw_json
         FROM evidence_rea_bundle_v1 WHERE bundle_id=?",
        &[Param::Text(id)],
    )?;
    if rows.len() != 1 {
        return refuse("REA bundle not found");
    }
    let row = &rows[0];
    let get = |n: usize| -> Result<&str> {
        row.get(n)
            .and_then(|value| value.as_deref())
            .ok_or_else(|| "missing REA SQL field".into())
    };
    let source = row[4].as_deref();
    match (source, link) {
        (Some(stored), Some(request)) if stored == request.snapshot => {
            let checked = verify_build(db, artifact, Some(request))?;
            if checked.as_deref() != source {
                return refuse("REA linked build no longer authentic");
            }
        }
        (None, None) => {}
        _ => return refuse("REA build link must be explicitly and independently audited"),
    }
    let prepared = prepare(artifact, get(5)?.to_owned(), source)?;
    if prepared.bundle_id != id
        || prepared.subject_id != get(0)?
        || prepared.sha != get(1)?
        || prepared.size.to_string() != get(2)?
        || prepared.raw_sha != get(3)?
    {
        return refuse("REA artifact, source link or original raw payload changed");
    }
    Ok(report(&prepared))
}
pub fn inspect_record(
    db: &Db,
    artifact: &Path,
    id: &str,
    native_id: &str,
    link: Option<&BuildLink<'_>>,
) -> Result<Value> {
    let provenance = audit(db, artifact, id, link)?;
    check_native_id(native_id)?;
    let rows = db.query(
        "SELECT raw_json FROM evidence_rea_bundle_v1 WHERE bundle_id=?",
        &[Param::Text(id)],
    )?;
    let raw = rows[0][0].as_deref().ok_or("missing REA raw payload")?;
    let parsed: JsonValue = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let record = parsed["records"]
        .as_array()
        .ok_or("no REA native records")?
        .iter()
        .find(|record| record["evidence_id"].as_str() == Some(native_id))
        .ok_or("REA native evidence ID not found")?;
    if record["subject"]["digest"]["sha256"].as_str()
        != Some(provenance.get("artifact_sha256")?.str()?)
    {
        return refuse("foreign REA evidence record is not bound to selected artifact");
    }
    let record_json = json::parse(
        serde_json::to_string(record)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    Ok(obj(&[
        ("schema", Value::string("excession.rea-record.v1")),
        ("bundle_id", Value::string(id)),
        (
            "artifact_subject_id",
            provenance.get("artifact_subject_id")?.clone(),
        ),
        ("native_evidence_id", Value::string(native_id)),
        ("original_record", record_json),
        ("semantic_correctness", Value::string("NOT_ESTABLISHED")),
        ("source_artifact_relation", Value::string("NOT_ESTABLISHED")),
    ]))
}
fn check_native_id(id: &str) -> Result<()> {
    native_id(id)
}
