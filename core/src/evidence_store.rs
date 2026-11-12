//! Immutable producer evidence on the existing Rust/SQLite authority.
//! A successful audit proves record integrity, not Git or semantic truth.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RECEIPTS: usize = 4096;
const MAX_OBSERVATIONS: usize = 20000;
const MAX_RAW_BYTES: usize = 4 * 1024 * 1024;
const MAX_OBSERVATION_BYTES: usize = 512 * 1024;

/// Original UTF-8 JSON, including its whitespace, is preserved exactly.
#[derive(Clone, Debug)]
pub struct Receipt {
    pub producer: String,
    pub version: String,
    pub native_id: String,
    pub raw_json: String,
}

/// Never reinterpret a producer's native predicate as a generic call/dependency.
#[derive(Clone, Debug)]
pub struct Observation {
    pub receipt_id: String,
    pub subject_id: String,
    pub native_kind: String,
    pub payload: Value,
}
#[derive(Clone, Debug)]
pub struct Batch {
    pub source_commit: String,
    pub receipts: Vec<Receipt>,
    pub observations: Vec<Observation>,
}

fn hex_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return refuse("expected lowercase SHA-256");
    }
    Ok(())
}
fn bounded(value: &str, limit: usize) -> Result<()> {
    if value.is_empty() || value.len() > limit || value.contains('\0') {
        return refuse("invalid or oversized evidence field");
    }
    Ok(())
}
fn digest_record(domain: &str, body: &Value) -> String {
    let mut bytes = Vec::from(domain.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(body.canonical().as_bytes());
    digest::sha256(&bytes)
}
fn seal(mut body: Value, key: &str, domain: &str) -> Result<Value> {
    let id = digest_record(domain, &body);
    body.entries_mut()?
        .insert(key.to_owned(), Value::string(id));
    Ok(body)
}
fn checked_id(record: &Value, key: &str, domain: &str) -> Result<String> {
    let claimed = record.get(key)?.str()?;
    hex_sha256(claimed)?;
    let mut body = record.clone();
    body.entries_mut()?.remove(key);
    if digest_record(domain, &body) != claimed {
        return refuse("evidence record digest mismatch");
    }
    Ok(claimed.to_owned())
}
impl Receipt {
    pub fn document(&self) -> Result<Value> {
        bounded(&self.producer, 256)?;
        bounded(&self.version, 256)?;
        bounded(&self.native_id, 16384)?;
        if self.raw_json.len() > MAX_RAW_BYTES {
            return refuse("raw receipt exceeds byte budget");
        }
        json::parse(self.raw_json.as_bytes())?;
        let body = obj(&[
            ("schema", Value::string("excession.evidence-receipt.v1")),
            ("producer", Value::string(&self.producer)),
            ("version", Value::string(&self.version)),
            ("native_id", Value::string(&self.native_id)),
            (
                "raw_sha256",
                Value::string(digest::sha256(self.raw_json.as_bytes())),
            ),
            ("raw_json", Value::string(&self.raw_json)),
            ("provenance", Value::string("PRODUCER_SUPPLIED_UNATTESTED")),
        ]);
        seal(body, "receipt_id", "excession.evidence-receipt.v1")
    }
}
impl Observation {
    pub fn document(&self) -> Result<Value> {
        hex_sha256(&self.receipt_id)?;
        hex_sha256(&self.subject_id)?;
        bounded(&self.native_kind, 256)?;
        if self.payload.canonical().len() > MAX_OBSERVATION_BYTES {
            return refuse("observation exceeds byte budget");
        }
        let body = obj(&[
            ("schema", Value::string("excession.evidence-observation.v1")),
            ("receipt_id", Value::string(&self.receipt_id)),
            ("subject_id", Value::string(&self.subject_id)),
            ("native_kind", Value::string(&self.native_kind)),
            ("payload", self.payload.clone()),
            ("authority", Value::string("PRODUCER_OBSERVATION_ONLY")),
        ]);
        seal(body, "observation_id", "excession.evidence-observation.v1")
    }
}
struct Prepared {
    id: String,
    manifest: Value,
    receipts: BTreeMap<String, Value>,
    observations: BTreeMap<String, Value>,
}
fn prepare(batch: &Batch) -> Result<Prepared> {
    git::oid(&batch.source_commit)?;
    if batch.receipts.len() > MAX_RECEIPTS || batch.observations.len() > MAX_OBSERVATIONS {
        return refuse("evidence batch exceeds record budget");
    }
    let mut receipts = BTreeMap::new();
    for item in &batch.receipts {
        let doc = item.document()?;
        let id = doc.get("receipt_id")?.str()?.to_owned();
        if receipts.insert(id, doc).is_some() {
            return refuse("duplicate receipt");
        }
    }
    let mut observations = BTreeMap::new();
    for item in &batch.observations {
        if !receipts.contains_key(&item.receipt_id) {
            return refuse("missing producer receipt");
        }
        let doc = item.document()?;
        let id = doc.get("observation_id")?.str()?.to_owned();
        if observations.insert(id, doc).is_some() {
            return refuse("duplicate observation");
        }
    }
    let manifest = seal(
        obj(&[
            ("schema", Value::string("excession.evidence-snapshot.v1")),
            ("source_commit", Value::string(&batch.source_commit)),
            ("source_authentication", Value::string("NOT_CHECKED")),
            (
                "receipt_ids",
                Value::array(receipts.keys().map(Value::string).collect::<Vec<_>>()),
            ),
            (
                "observation_ids",
                Value::array(observations.keys().map(Value::string).collect::<Vec<_>>()),
            ),
        ]),
        "snapshot_id",
        "excession.evidence-snapshot.v1",
    )?;
    let id = manifest.get("snapshot_id")?.str()?.to_owned();
    Ok(Prepared {
        id,
        manifest,
        receipts,
        observations,
    })
}

/// Additive tables, leaving existing repository and configured graph schema alone.
pub fn init(db: &Db) -> Result<()> {
    db.exec("CREATE TABLE IF NOT EXISTS evidence_snapshot_v1(
        snapshot_id TEXT PRIMARY KEY, source_commit TEXT NOT NULL, document TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS evidence_receipt_v1(
        snapshot_id TEXT NOT NULL, receipt_id TEXT NOT NULL, document TEXT NOT NULL,
        PRIMARY KEY(snapshot_id,receipt_id),
        FOREIGN KEY(snapshot_id) REFERENCES evidence_snapshot_v1(snapshot_id));
        CREATE TABLE IF NOT EXISTS evidence_observation_v1(
        snapshot_id TEXT NOT NULL, observation_id TEXT NOT NULL, receipt_id TEXT NOT NULL,
        document TEXT NOT NULL, PRIMARY KEY(snapshot_id,observation_id),
        FOREIGN KEY(snapshot_id,receipt_id) REFERENCES evidence_receipt_v1(snapshot_id,receipt_id));")
}
/// Reimport refuses previously damaged rows rather than silently repairing them.
pub fn persist(db: &Db, batch: &Batch) -> Result<String> {
    let prepared = prepare(batch)?;
    init(db)?;
    db.transaction(|db| {
        let prior = db.query(
            "SELECT document FROM evidence_snapshot_v1 WHERE snapshot_id=?",
            &[Param::Text(&prepared.id)],
        )?;
        if !prior.is_empty() {
            if audit(db, &prepared.id)? != prepared.manifest {
                return refuse("existing evidence snapshot differs");
            }
            return Ok(prepared.id.clone());
        }
        db.execute(
            "INSERT INTO evidence_snapshot_v1(snapshot_id,source_commit,document) VALUES(?,?,?)",
            &[Param::Text(&prepared.id), Param::Text(&batch.source_commit),
              Param::Text(&prepared.manifest.canonical())],
        )?;
        for (id, doc) in &prepared.receipts {
            db.execute(
                "INSERT INTO evidence_receipt_v1(snapshot_id,receipt_id,document) VALUES(?,?,?)",
                &[Param::Text(&prepared.id), Param::Text(id), Param::Text(&doc.canonical())],
            )?;
        }
        for (id, doc) in &prepared.observations {
            db.execute(
                "INSERT INTO evidence_observation_v1(snapshot_id,observation_id,receipt_id,document) VALUES(?,?,?,?)",
                &[Param::Text(&prepared.id), Param::Text(id),
                  Param::Text(doc.get("receipt_id")?.str()?), Param::Text(&doc.canonical())],
            )?;
        }
        audit(db, &prepared.id)?;
        Ok(prepared.id.clone())
    })
}
fn exact_ids(manifest: &Value, key: &str, max: usize) -> Result<Vec<String>> {
    let items = manifest.get(key)?.array_ref()?;
    if items.len() > max {
        return refuse("snapshot exceeds record budget");
    }
    let mut result = Vec::new();
    for item in items {
        let id = item.str()?;
        hex_sha256(id)?;
        if result
            .last()
            .is_some_and(|previous: &String| previous.as_str() >= id)
        {
            return refuse("manifest IDs must be strictly ordered");
        }
        result.push(id.to_owned());
    }
    Ok(result)
}
fn column(row: &[Option<String>], n: usize) -> Result<&str> {
    row.get(n)
        .and_then(|v| v.as_deref())
        .ok_or("missing evidence SQL column".into())
}
/// Check every row, exact manifest membership, native receipt payload, and FK.
/// This is not independent verification of producer accuracy or Git source.
pub fn audit(db: &Db, snapshot_id: &str) -> Result<Value> {
    hex_sha256(snapshot_id)?;
    let snapshots = db.query(
        "SELECT source_commit,document FROM evidence_snapshot_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot_id)],
    )?;
    if snapshots.len() != 1 {
        return refuse("missing or duplicated snapshot");
    }
    let source = column(&snapshots[0], 0)?;
    git::oid(source)?;
    let manifest = json::parse(column(&snapshots[0], 1)?.as_bytes())?;
    if manifest.get("schema")?.str()? != "excession.evidence-snapshot.v1"
        || manifest.get("source_commit")?.str()? != source
        || manifest.get("source_authentication")?.str()? != "NOT_CHECKED"
        || checked_id(&manifest, "snapshot_id", "excession.evidence-snapshot.v1")? != snapshot_id
    {
        return refuse("invalid manifest identity");
    }
    let receipt_ids = exact_ids(&manifest, "receipt_ids", MAX_RECEIPTS)?;
    let observation_ids = exact_ids(&manifest, "observation_ids", MAX_OBSERVATIONS)?;
    let receipts = db.query(
        "SELECT receipt_id,document FROM evidence_receipt_v1 WHERE snapshot_id=? ORDER BY receipt_id",
        &[Param::Text(snapshot_id)],
    )?;
    if receipts.len() != receipt_ids.len() {
        return refuse("missing or extra receipt row");
    }
    let mut received = BTreeSet::new();
    for (row, expected) in receipts.iter().zip(receipt_ids.iter()) {
        let key = column(row, 0)?;
        if key != expected {
            return refuse("receipt membership differs");
        }
        let raw = column(row, 1)?;
        let doc = json::parse(raw.as_bytes())?;
        if doc.canonical() != raw
            || checked_id(&doc, "receipt_id", "excession.evidence-receipt.v1")? != key
            || doc.get("schema")?.str()? != "excession.evidence-receipt.v1"
            || doc.get("provenance")?.str()? != "PRODUCER_SUPPLIED_UNATTESTED"
        {
            return refuse("receipt document corrupted");
        }
        let source = doc.get("raw_json")?.str()?;
        if source.len() > MAX_RAW_BYTES
            || json::parse(source.as_bytes()).is_err()
            || digest::sha256(source.as_bytes()) != doc.get("raw_sha256")?.str()?
        {
            return refuse("raw producer evidence corrupted");
        }
        bounded(doc.get("producer")?.str()?, 256)?;
        bounded(doc.get("version")?.str()?, 256)?;
        bounded(doc.get("native_id")?.str()?, 16384)?;
        received.insert(key.to_owned());
    }
    let observations = db.query(
        "SELECT observation_id,receipt_id,document FROM evidence_observation_v1 WHERE snapshot_id=? ORDER BY observation_id",
        &[Param::Text(snapshot_id)],
    )?;
    if observations.len() != observation_ids.len() {
        return refuse("missing or extra observation row");
    }
    for (row, expected) in observations.iter().zip(observation_ids.iter()) {
        if column(row, 0)? != expected || !received.contains(column(row, 1)?) {
            return refuse("observation identity or producer receipt mismatch");
        }
        let raw = column(row, 2)?;
        let doc = json::parse(raw.as_bytes())?;
        if doc.canonical() != raw
            || checked_id(&doc, "observation_id", "excession.evidence-observation.v1")?.as_str()
                != expected
            || doc.get("schema")?.str()? != "excession.evidence-observation.v1"
            || doc.get("authority")?.str()? != "PRODUCER_OBSERVATION_ONLY"
            || doc.get("receipt_id")?.str()? != column(row, 1)?
        {
            return refuse("observation record corrupted");
        }
        hex_sha256(doc.get("subject_id")?.str()?)?;
        bounded(doc.get("native_kind")?.str()?, 256)?;
        if doc.get("payload")?.canonical().len() > MAX_OBSERVATION_BYTES {
            return refuse("observation payload exceeds budget");
        }
    }
    db.foreign_keys_valid()?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "evidence_store_tests.rs"]
mod tests;
