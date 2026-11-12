//! Append-only uncertainty history bound to immutable producer-evidence snapshots.
//! A recorded resolution is an operator assertion, never semantic verification.
use crate::evidence_store;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, refuse};
use std::collections::BTreeSet;

const MAX_EVENTS: usize = 128;
const MAX_RECEIPTS_PER_SIDE: usize = 64;
const MAX_QUESTION_BYTES: usize = 4096;
const DOMAIN: &str = "excession.evidence-unknown-event.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    AddEvidence,
    ProposeResolution,
    Reopen,
}
impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Create => "CREATE",
            Self::AddEvidence => "ADD_EVIDENCE",
            Self::ProposeResolution => "PROPOSE_RESOLUTION",
            Self::Reopen => "REOPEN",
        }
    }
    fn from_name(name: &str) -> Result<Self> {
        match name {
            "CREATE" => Ok(Self::Create),
            "ADD_EVIDENCE" => Ok(Self::AddEvidence),
            "PROPOSE_RESOLUTION" => Ok(Self::ProposeResolution),
            "REOPEN" => Ok(Self::Reopen),
            _ => refuse("unknown evidence history action"),
        }
    }
    fn state(self) -> &'static str {
        match self {
            Self::Create | Self::AddEvidence | Self::Reopen => "OPEN",
            Self::ProposeResolution => "RESOLUTION_REPORTED_NOT_VERIFIED",
        }
    }
}

/// Receipts are cumulative on each revision. An explicit expected revision is
/// required for every write, including creation (expected_revision = 0).
pub struct Event<'a> {
    pub snapshot_id: &'a str,
    pub question: &'a str,
    pub expected_revision: usize,
    pub action: Action,
    pub support_receipts: &'a [&'a str],
    pub counterevidence_receipts: &'a [&'a str],
    pub resolution_receipt: Option<&'a str>,
}

fn sha256_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return refuse("evidence history requires lowercase SHA-256 identity");
    }
    Ok(())
}

pub fn unknown_id(snapshot: &str, question: &str) -> Result<String> {
    sha256_id(snapshot)?;
    if question.is_empty() || question.len() > MAX_QUESTION_BYTES || question.contains('\0') {
        return refuse("invalid bounded unknown question");
    }
    let mut bytes = b"excession.unknown.v1\0".to_vec();
    bytes.extend_from_slice(snapshot.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(question.as_bytes());
    Ok(digest::sha256(&bytes))
}

fn ids(input: &[&str], manifest: &BTreeSet<String>) -> Result<Vec<String>> {
    if input.len() > MAX_RECEIPTS_PER_SIDE {
        return refuse("unknown history receipt budget exceeded");
    }
    let mut values = BTreeSet::new();
    for id in input {
        sha256_id(id)?;
        if !manifest.contains(*id) || !values.insert((*id).to_owned()) {
            return refuse("unknown history receipt not unique or absent from snapshot");
        }
    }
    Ok(values.into_iter().collect())
}

fn read_ids(value: &Value, key: &str, manifest: &BTreeSet<String>) -> Result<Vec<String>> {
    let values = value.get(key)?.array_ref()?;
    let raw_ids = values.iter().map(Value::str).collect::<Result<Vec<_>>>()?;
    let normalized = ids(raw_ids.as_slice(), manifest)?;
    if raw_ids.to_vec() != normalized.iter().map(String::as_str).collect::<Vec<_>>() {
        return refuse("history receipt IDs must be sorted and unique");
    }
    Ok(normalized)
}

fn snapshot_receipts(db: &Db, snapshot: &str) -> Result<BTreeSet<String>> {
    let manifest = evidence_store::audit(db, snapshot)?;
    let receipts = manifest.get("receipt_ids")?.array_ref()?;
    let mut ids = BTreeSet::new();
    for id in receipts {
        ids.insert(id.str()?.to_owned());
    }
    Ok(ids)
}

pub fn init(db: &Db) -> Result<()> {
    evidence_store::init(db)?;
    db.exec(
        "CREATE TABLE IF NOT EXISTS evidence_unknown_event_v1(
            unknown_id TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 128),
            event_id TEXT NOT NULL UNIQUE,
            previous_event_id TEXT,
            snapshot_id TEXT NOT NULL,
            document TEXT NOT NULL,
            PRIMARY KEY(unknown_id, revision),
            FOREIGN KEY(snapshot_id) REFERENCES evidence_snapshot_v1(snapshot_id));
         CREATE TRIGGER IF NOT EXISTS evidence_unknown_no_update_v1
           BEFORE UPDATE ON evidence_unknown_event_v1 BEGIN
             SELECT RAISE(ABORT, 'unknown history is append-only');
           END;
         CREATE TRIGGER IF NOT EXISTS evidence_unknown_no_delete_v1
           BEFORE DELETE ON evidence_unknown_event_v1 BEGIN
             SELECT RAISE(ABORT, 'unknown history is append-only');
           END;",
    )
}

fn event_digest(body: &Value) -> String {
    let mut bytes = Vec::from(DOMAIN.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(body.canonical().as_bytes());
    digest::sha256(&bytes)
}

fn records(db: &Db, id: &str) -> Result<Vec<Value>> {
    let rows = db.query(
        "SELECT revision,event_id,previous_event_id,snapshot_id,document
         FROM evidence_unknown_event_v1 WHERE unknown_id=? ORDER BY revision",
        &[Param::Text(id)],
    )?;
    if rows.is_empty() || rows.len() > MAX_EVENTS {
        return refuse("missing or oversized unknown history");
    }
    let mut previous: Option<String> = None;
    let mut support = BTreeSet::new();
    let mut counterevidence = BTreeSet::new();
    let mut prior_action: Option<Action> = None;
    let mut question: Option<String> = None;
    let mut snapshot: Option<String> = None;
    let mut allowed: Option<BTreeSet<String>> = None;
    let mut result = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let field = |n: usize| -> Result<&str> {
            row.get(n)
                .and_then(|v| v.as_deref())
                .ok_or_else(|| "unknown history missing SQL column".to_owned())
        };
        let revision = i + 1;
        if field(0)?.parse::<usize>().ok() != Some(revision) {
            return refuse("unknown history revision gap or reorder");
        }
        if field(4)?.len() > 32 * 1024 {
            return refuse("oversized unknown history event document");
        }
        let record = json::parse(field(4)?.as_bytes())?;
        if record.entries()?.len() != 13
            || record.get("schema")?.str()? != DOMAIN
            || record.get("unknown_id")?.str()? != id
            || record.get("revision")?.integer()? != i64::try_from(revision).unwrap_or(-1)
        {
            return refuse("unknown event schema/identity mismatch");
        }
        let snap = record.get("snapshot_id")?.str()?;
        if field(3)? != snap || snapshot.as_deref().is_some_and(|s| s != snap) {
            return refuse("unknown event snapshot drift");
        }
        if snapshot.is_none() {
            snapshot = Some(snap.to_owned());
            allowed = Some(snapshot_receipts(db, snap)?);
        }
        let q = record.get("question")?.str()?;
        if unknown_id(snap, q)? != id || question.as_deref().is_some_and(|v| v != q) {
            return refuse("unknown event question identity drift");
        }
        question = Some(q.to_owned());
        let old = record.get("previous_event_id")?;
        let claimed_old = match old {
            Value::Null => None,
            _ => Some(old.str()?),
        };
        if claimed_old != previous.as_deref()
            || row.get(2).and_then(|v| v.as_deref()) != previous.as_deref()
        {
            return refuse("unknown history predecessor mismatch");
        }
        let action = Action::from_name(record.get("action")?.str()?)?;
        if (revision == 1 && action != Action::Create) || (revision > 1 && action == Action::Create)
        {
            return refuse("invalid initial or repeated unknown creation");
        }
        if record.get("state")?.str()? != action.state()
            || record.get("semantic_verification")?.str()? != "NOT_ESTABLISHED"
        {
            return refuse("unknown resolution falsely promoted to verification");
        }
        let permitted = allowed.as_ref().ok_or("unknown snapshot unvalidated")?;
        let next_support = read_ids(&record, "support_receipt_ids", permitted)?;
        let next_counter = read_ids(&record, "counterevidence_receipt_ids", permitted)?;
        let new_support = next_support.iter().cloned().collect::<BTreeSet<_>>();
        let new_counter = next_counter.iter().cloned().collect::<BTreeSet<_>>();
        if !support.is_subset(&new_support) || !counterevidence.is_subset(&new_counter) {
            return refuse("unknown history cannot retract evidence");
        }
        let extra_support = new_support.len() > support.len();
        let extra_counter = new_counter.len() > counterevidence.len();
        let resolution = match record.get("resolution_receipt_id")? {
            Value::Null => None,
            value => Some(value.str()?),
        };
        match action {
            Action::Create if resolution.is_some() => {
                return refuse("unknown creation cannot report resolution");
            }
            Action::AddEvidence
                if prior_action == Some(Action::ProposeResolution)
                    || (!extra_support && !extra_counter)
                    || resolution.is_some() =>
            {
                return refuse("invalid or empty unknown evidence update");
            }
            Action::ProposeResolution
                if revision == 1
                    || prior_action == Some(Action::ProposeResolution)
                    || !resolution.is_some_and(|id| new_support.contains(id)) =>
            {
                return refuse("resolution proposal requires supporting snapshot receipt");
            }
            Action::Reopen
                if prior_action != Some(Action::ProposeResolution)
                    || !extra_counter
                    || resolution.is_some() =>
            {
                return refuse("reopening a proposed resolution requires new counterevidence");
            }
            _ => {}
        }
        let claimed = record.get("event_id")?.str()?;
        sha256_id(claimed)?;
        let mut body = record.clone();
        body.entries_mut()?.remove("event_id");
        if field(1)? != claimed || event_digest(&body) != claimed || field(4)? != record.canonical()
        {
            return refuse("unknown history event digest or canonical bytes differ");
        }
        previous = Some(claimed.to_owned());
        support = new_support;
        counterevidence = new_counter;
        prior_action = Some(action);
        result.push(record);
    }
    Ok(result)
}

/// Full independent row-chain replay within one SQLite read snapshot.
/// It verifies source-snapshot receipt membership but not producer truth.
pub fn audit(db: &Db, id: &str) -> Result<Value> {
    sha256_id(id)?;
    db.read_transaction(|db| {
        let events = records(db, id)?;
        let last = events
            .last()
            .ok_or("unknown history missing latest event")?;
        let mut report = obj(&[
            ("schema", Value::string("excession.unknown-history.v1")),
            ("unknown_id", Value::string(id)),
            ("snapshot_id", last.get("snapshot_id")?.clone()),
            ("state", last.get("state")?.clone()),
            ("events", Value::array(events.clone())),
            ("source_authentication", Value::string("NOT_ESTABLISHED")),
            ("semantic_verification", Value::string("NOT_ESTABLISHED")),
        ]);
        let digest = report.sha256();
        report
            .entries_mut()?
            .insert("report_id".into(), Value::string(digest));
        Ok(report)
    })
}

/// Atomically check expected revision and append a cumulative event.
/// No caller-supplied event ID or state is trusted.
pub fn append(db: &Db, event: &Event<'_>) -> Result<Value> {
    let id = unknown_id(event.snapshot_id, event.question)?;
    init(db)?;
    db.transaction(|db| {
        let manifest = snapshot_receipts(db, event.snapshot_id)?;
        let existing = db.query(
            "SELECT revision FROM evidence_unknown_event_v1 WHERE unknown_id=?
             ORDER BY revision DESC LIMIT 1",
            &[Param::Text(&id)],
        )?;
        let count = if existing.is_empty() {
            0
        } else {
            records(db, &id)?.len()
        };
        if count != event.expected_revision || count >= MAX_EVENTS {
            return refuse("stale or oversized unknown expected revision");
        }
        let support = ids(event.support_receipts, &manifest)?;
        let counter = ids(event.counterevidence_receipts, &manifest)?;
        let previous = if count == 0 {
            None
        } else {
            let last = records(db, &id)?
                .pop()
                .ok_or("missing previous unknown history event")?;
            Some(last.get("event_id")?.str()?.to_owned())
        };
        let body = obj(&[
            ("schema", Value::string(DOMAIN)),
            ("unknown_id", Value::string(&id)),
            ("snapshot_id", Value::string(event.snapshot_id)),
            ("question", Value::string(event.question)),
            ("revision", Value::number(count + 1)),
            (
                "previous_event_id",
                previous.as_ref().map_or(Value::Null, Value::string),
            ),
            ("action", Value::string(event.action.name())),
            ("state", Value::string(event.action.state())),
            (
                "support_receipt_ids",
                Value::array(support.iter().map(Value::string)),
            ),
            (
                "counterevidence_receipt_ids",
                Value::array(counter.iter().map(Value::string)),
            ),
            (
                "resolution_receipt_id",
                event.resolution_receipt.map_or(Value::Null, Value::string),
            ),
            ("semantic_verification", Value::string("NOT_ESTABLISHED")),
        ]);
        let mut record = body.clone();
        let hash = event_digest(&body);
        record
            .entries_mut()?
            .insert("event_id".into(), Value::string(&hash));
        // Verify the complete transition, not just the new event in isolation.
        db.execute(
            "INSERT INTO evidence_unknown_event_v1
             (unknown_id,revision,event_id,previous_event_id,snapshot_id,document)
             VALUES(?,?,?,?,?,?)",
            &[
                Param::Text(&id),
                Param::Integer(i64::try_from(count + 1).map_err(|_| "revision overflow")?),
                Param::Text(&hash),
                previous.as_deref().map_or(Param::Null, Param::Text),
                Param::Text(event.snapshot_id),
                Param::Text(&record.canonical()),
            ],
        )?;
        records(db, &id)?;
        Ok(record)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_store::{Batch, Observation, Receipt};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        db: Db,
        path: std::path::PathBuf,
        snapshot: String,
        receipts: Vec<String>,
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "excession-unknowns-{}-{}.sqlite",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let db = Db::open(&path).unwrap();
            let mut receipt_ids = Vec::new();
            let receipts = (0..3)
                .map(|n| {
                    let receipt = Receipt {
                        producer: "independent-native".into(),
                        version: "1".into(),
                        native_id: format!("observation-{n}"),
                        raw_json: format!("{{\"observation\":{n}}}"),
                    };
                    receipt_ids.push(
                        receipt
                            .document()
                            .unwrap()
                            .get("receipt_id")
                            .unwrap()
                            .str()
                            .unwrap()
                            .to_owned(),
                    );
                    receipt
                })
                .collect();
            let batch = Batch {
                source_commit: "a".repeat(40),
                receipts,
                observations: Vec::<Observation>::new(),
            };
            let snapshot = evidence_store::persist(&db, &batch).unwrap();
            Self {
                db,
                path,
                snapshot,
                receipts: receipt_ids,
            }
        }
        fn write(
            &self,
            rev: usize,
            action: Action,
            support: &[&str],
            counter: &[&str],
            resolution: Option<&str>,
        ) -> Result<Value> {
            append(
                &self.db,
                &Event {
                    snapshot_id: &self.snapshot,
                    question: "What is the indirect call target?",
                    expected_revision: rev,
                    action,
                    support_receipts: support,
                    counterevidence_receipts: counter,
                    resolution_receipt: resolution,
                },
            )
        }
        fn id(&self) -> String {
            unknown_id(&self.snapshot, "What is the indirect call target?").unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn creates_replays_and_reopens_without_promoting_resolution() {
        let f = Fixture::new();
        let a = &f.receipts[0];
        let b = &f.receipts[1];
        let c = &f.receipts[2];
        let first = f.write(0, Action::Create, &[a], &[], None).unwrap();
        assert_eq!(first.get("revision").unwrap().integer().unwrap(), 1);
        assert!(f.write(0, Action::Create, &[a], &[], None).is_err());
        f.write(1, Action::AddEvidence, &[a, b], &[], None).unwrap();
        f.write(2, Action::ProposeResolution, &[a, b], &[], Some(b))
            .unwrap();
        let proposal = audit(&f.db, &f.id()).unwrap();
        assert_eq!(
            proposal.get("state").unwrap().str().unwrap(),
            "RESOLUTION_REPORTED_NOT_VERIFIED"
        );
        assert_eq!(
            proposal
                .get("semantic_verification")
                .unwrap()
                .str()
                .unwrap(),
            "NOT_ESTABLISHED"
        );
        assert!(
            f.write(3, Action::AddEvidence, &[a, b], &[c], None)
                .is_err()
        );
        f.write(3, Action::Reopen, &[a, b], &[c], None).unwrap();
        let report = audit(&f.db, &f.id()).unwrap();
        assert_eq!(report.get("events").unwrap().array_ref().unwrap().len(), 4);
        assert_eq!(report.get("state").unwrap().str().unwrap(), "OPEN");
    }

    #[test]
    fn rejects_forged_receipts_retractions_and_invalid_transitions() {
        let f = Fixture::new();
        let a = &f.receipts[0];
        let b = &f.receipts[1];
        assert!(
            f.write(0, Action::Create, &[&"f".repeat(64)], &[], None)
                .is_err()
        );
        assert!(f.write(0, Action::Create, &[a, a], &[], None).is_err());
        assert!(
            f.write(0, Action::ProposeResolution, &[a], &[], Some(a))
                .is_err()
        );
        f.write(0, Action::Create, &[a], &[], None).unwrap();
        assert!(f.write(1, Action::AddEvidence, &[a], &[], None).is_err());
        assert!(f.write(1, Action::AddEvidence, &[], &[b], None).is_err());
        assert!(
            f.write(1, Action::ProposeResolution, &[a], &[], Some(b))
                .is_err()
        );
        f.write(1, Action::AddEvidence, &[a, b], &[], None).unwrap();
        f.write(2, Action::ProposeResolution, &[a, b], &[], Some(b))
            .unwrap();
        assert!(f.write(3, Action::Reopen, &[a, b], &[], None).is_err());
        assert!(
            f.write(3, Action::ProposeResolution, &[a, b], &[], Some(b))
                .is_err()
        );
        assert_eq!(
            audit(&f.db, &f.id())
                .unwrap()
                .get("events")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn sqlite_append_only_and_digest_tampering_are_refused() {
        let f = Fixture::new();
        let a = &f.receipts[0];
        f.write(0, Action::Create, &[a], &[], None).unwrap();
        assert!(f.db.exec("DELETE FROM evidence_unknown_event_v1").is_err());
        assert!(
            f.db.exec("UPDATE evidence_unknown_event_v1 SET revision=12")
                .is_err()
        );
        f.db.exec("DROP TRIGGER evidence_unknown_no_update_v1")
            .unwrap();
        f.db.exec("UPDATE evidence_unknown_event_v1 SET document='{}'")
            .unwrap();
        assert!(audit(&f.db, &f.id()).is_err());
        assert!(
            f.write(1, Action::AddEvidence, &[a, &f.receipts[1]], &[], None)
                .is_err()
        );
    }

    #[test]
    fn truncated_or_wrong_history_cannot_be_audited() {
        let f = Fixture::new();
        let a = &f.receipts[0];
        f.write(0, Action::Create, &[a], &[], None).unwrap();
        f.write(1, Action::AddEvidence, &[a, &f.receipts[1]], &[], None)
            .unwrap();
        f.db.exec("DROP TRIGGER evidence_unknown_no_delete_v1")
            .unwrap();
        f.db.exec("DELETE FROM evidence_unknown_event_v1 WHERE revision=1")
            .unwrap();
        assert!(audit(&f.db, &f.id()).is_err());
        assert!(
            f.write(
                2,
                Action::ProposeResolution,
                &[a, &f.receipts[1]],
                &[],
                Some(a)
            )
            .is_err()
        );
    }

    #[test]
    fn event_history_survives_reopen() {
        let f = Fixture::new();
        let a = &f.receipts[0];
        f.write(0, Action::Create, &[a], &[], None).unwrap();
        let first = audit(&f.db, &f.id()).unwrap();
        let db2 = Db::open(&f.path).unwrap();
        let second = audit(&db2, &f.id()).unwrap();
        assert_eq!(first, second);
        assert!(f.write(0, Action::Create, &[a], &[], None).is_err());
    }
}
