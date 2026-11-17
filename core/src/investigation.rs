//! Persistent, append-only investigation notes over exact Git revisions.
//! Source review is an operator assertion. No claim is promoted to verified
//! behavior by source identity, a plan, or an agent-verification report.
use crate::discovery;
use crate::evidence::identity::SourceRef;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::BTreeSet;
use std::path::Path;

const SCHEMA: &str = "excession.investigation-event.v1";
const REPORT_SCHEMA: &str = "excession.investigation.v1";
const MAX_TEXT: usize = 8192;
const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

fn bounded(value: &str, what: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_TEXT || value.contains('\0') {
        return refuse(format!("invalid investigation {what}"));
    }
    Ok(())
}

fn digest_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return refuse("investigation expects lowercase SHA-256 ID");
    }
    Ok(())
}

pub fn investigation_id(commit: &str, tree: &str, question: &str) -> Result<String> {
    git::oid(commit)?;
    git::oid(tree)?;
    bounded(question, "question")?;
    let mut bytes = b"excession.investigation.v1\0".to_vec();
    for part in [commit, tree, question] {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    Ok(digest::sha256(&bytes))
}

fn source_value(source: &SourceRef) -> Value {
    obj(&[
        ("commit", Value::string(&source.commit)),
        ("tree", Value::string(&source.tree)),
        ("path", Value::string(&source.path)),
        ("blob", Value::string(&source.blob)),
        ("mode", Value::string(&source.mode)),
    ])
}

fn source_from_value(value: &Value) -> Result<SourceRef> {
    if value.entries()?.len() != 5 {
        return refuse("investigation source has unexpected fields");
    }
    Ok(SourceRef {
        commit: value.get("commit")?.str()?.to_owned(),
        tree: value.get("tree")?.str()?.to_owned(),
        path: value.get("path")?.str()?.to_owned(),
        blob: value.get("blob")?.str()?.to_owned(),
        mode: value.get("mode")?.str()?.to_owned(),
    })
}

fn field(row: &[Option<String>], index: usize) -> Result<&str> {
    row.get(index)
        .and_then(|v| v.as_deref())
        .ok_or_else(|| "missing investigation SQL column".to_owned())
}

pub fn init(db: &Db) -> Result<()> {
    db.exec(
        "CREATE TABLE IF NOT EXISTS investigation_event_v1(
            investigation_id TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision > 0),
            event_id TEXT NOT NULL UNIQUE,
            previous_event_id TEXT,
            document TEXT NOT NULL,
            PRIMARY KEY(investigation_id, revision));
         CREATE TRIGGER IF NOT EXISTS investigation_no_update_v1
           BEFORE UPDATE ON investigation_event_v1 BEGIN
             SELECT RAISE(ABORT, 'investigation history is append-only');
           END;
         CREATE TRIGGER IF NOT EXISTS investigation_no_delete_v1
           BEFORE DELETE ON investigation_event_v1 BEGIN
             SELECT RAISE(ABORT, 'investigation history is append-only');
           END;",
    )
}

fn verify_agent_report(repo: &Path, commit: &str, tree: &str, report: &Value) -> Result<()> {
    if report.get("schema")?.str()? != "excession.agent-verification.v1"
        || report.get("base_commit")?.str()? != commit
        || report.get("base_tree")?.str()? != tree
        || !report.content_id_matches("report_id")?
        || report.get("ready_to_merge")?.bool()?
        || report.get("safe_to_skip_unlisted_tests")?.bool()?
    {
        return refuse("investigation verification report identity or safety mismatch");
    }
    let candidate = report.get("candidate_commit")?.str()?;
    git::oid(candidate)?;
    if candidate == commit
        || git::rev(repo, &format!("{candidate}^{{commit}}"))? != candidate
        || git::rev(repo, &format!("{candidate}^{{tree}}"))?
            != report.get("candidate_tree")?.str()?
    {
        return refuse("investigation verification candidate Git identity mismatch");
    }
    if !matches!(
        report.get("status")?.str()?,
        "OPEN_OBLIGATIONS" | "REPORTED_FAILURE"
    ) {
        return refuse("investigation cannot promote agent verification to proved");
    }
    if report.get("obligations")?.array_ref()?.is_empty() {
        return refuse("investigation verification is missing open obligations");
    }
    Ok(())
}

/// Reconstruct the entire immutable event chain in one read snapshot.
/// Source identities and agent-report digest are independently rechecked.
fn records(db: &Db, repo: &Path, id: &str) -> Result<Vec<Value>> {
    digest_id(id)?;
    let rows = db.query(
        "SELECT revision,event_id,previous_event_id,document
         FROM investigation_event_v1 WHERE investigation_id=? ORDER BY revision",
        &[Param::Text(id)],
    )?;
    if rows.is_empty() {
        return refuse("investigation not found");
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut prior: Option<String> = None;
    let mut question: Option<String> = None;
    let mut commit: Option<String> = None;
    let mut tree: Option<String> = None;
    let mut claims = BTreeSet::new();
    let mut plans = BTreeSet::new();
    for (i, row) in rows.iter().enumerate() {
        let raw = field(row, 3)?;
        if raw.len() > MAX_EVENT_BYTES {
            return refuse("oversized investigation event");
        }
        let event = json::parse(raw.as_bytes())?;
        if event.entries()?.len() != 10
            || event.get("schema")?.str()? != SCHEMA
            || event.get("investigation_id")?.str()? != id
            || event.get("revision")?.integer()? != (i + 1) as i64
            || field(row, 0)?.parse::<usize>().ok() != Some(i + 1)
            || event.canonical() != raw
        {
            return refuse("investigation row or canonical representation differs");
        }
        let claimed_prev = match event.get("previous_event_id")? {
            Value::Null => None,
            v => Some(v.str()?),
        };
        if claimed_prev != prior.as_deref()
            || row.get(2).and_then(|v| v.as_deref()) != prior.as_deref()
        {
            return refuse("investigation event predecessor mismatch");
        }
        let event_id = event.get("event_id")?.str()?;
        digest_id(event_id)?;
        if event_id != field(row, 1)? || !event.content_id_matches("event_id")? {
            return refuse("investigation event digest mismatch");
        }
        let q = event.get("question")?.str()?;
        let c = event.get("source_commit")?.str()?;
        let t = event.get("source_tree")?.str()?;
        if investigation_id(c, t, q)? != id
            || question.as_deref().is_some_and(|old| old != q)
            || commit.as_deref().is_some_and(|old| old != c)
            || tree.as_deref().is_some_and(|old| old != t)
        {
            return refuse("investigation source or question drift");
        }
        if question.is_none() {
            if git::rev(repo, &format!("{c}^{{commit}}"))? != c
                || git::rev(repo, &format!("{c}^{{tree}}"))? != t
            {
                return refuse("investigation source commit/tree not authenticated");
            }
            question = Some(q.to_owned());
            commit = Some(c.to_owned());
            tree = Some(t.to_owned());
        }
        let payload = event.get("payload")?;
        match event.get("kind")?.str()? {
            "CREATE" if i == 0 && payload.entries()?.is_empty() => {}
            "CLAIM" if i > 0 => {
                if payload.entries()?.len() != 4 {
                    return refuse("invalid investigation claim payload");
                }
                bounded(payload.get("statement")?.str()?, "claim")?;
                bounded(payload.get("assumption")?.str()?, "claim assumption")?;
                if !matches!(
                    payload.get("assessment")?.str()?,
                    "HYPOTHESIS" | "SOURCE_REVIEW_REPORTED"
                ) {
                    return refuse("investigation claim cannot assert unverified status");
                }
                let source = source_from_value(payload.get("source")?)?;
                if source.commit != c || source.tree != t {
                    return refuse("investigation claim source differs from root");
                }
                source.verify_git(repo)?;
                claims.insert(event_id.to_owned());
            }
            "PLAN" if i > 0 => {
                if payload.entries()?.len() != 2
                    || !claims.contains(payload.get("claim_event_id")?.str()?)
                {
                    return refuse("investigation plan must reference an earlier claim");
                }
                bounded(payload.get("obligation")?.str()?, "preservation obligation")?;
                plans.insert(event_id.to_owned());
            }
            "VERIFY" if i > 0 => {
                if payload.entries()?.len() != 2
                    || !plans.contains(payload.get("plan_event_id")?.str()?)
                {
                    return refuse("verification must reference an earlier preservation plan");
                }
                verify_agent_report(repo, c, t, payload.get("report")?)?;
            }
            _ => return refuse("invalid investigation event kind or initial event"),
        }
        prior = Some(event_id.to_owned());
        result.push(event);
    }
    Ok(result)
}

pub fn audit(db: &Db, repo: &Path, id: &str) -> Result<Value> {
    init(db)?;
    db.read_transaction(|db| {
        let events = records(db, repo, id)?;
        let initial = events.first().ok_or("missing initial investigation")?;
        let mut report = obj(&[
            ("schema", Value::string(REPORT_SCHEMA)),
            ("investigation_id", Value::string(id)),
            ("source_commit", initial.get("source_commit")?.clone()),
            ("source_tree", initial.get("source_tree")?.clone()),
            ("question", initial.get("question")?.clone()),
            ("revision", Value::number(events.len())),
            ("events", Value::array(events)),
            ("semantic_verification", Value::string("NOT_ESTABLISHED")),
            ("ready_to_merge", Value::Bool(false)),
        ]);
        let hash = report.sha256();
        report
            .entries_mut()?
            .insert("report_id".into(), Value::string(hash));
        Ok(report)
    })
}

fn append(
    db: &Db,
    repo: &Path,
    id: &str,
    expected_revision: usize,
    kind: &str,
    payload: Value,
    initial: Option<(&str, &str, &str)>,
) -> Result<Value> {
    init(db)?;
    db.transaction(|db| {
        let (prior, question, commit, tree) = if let Some((question, commit, tree)) = initial {
            if expected_revision != 0 || kind != "CREATE" {
                return refuse("investigation creation requires expected revision zero");
            }
            let existing = db.query(
                "SELECT revision FROM investigation_event_v1 WHERE investigation_id=? LIMIT 1",
                &[Param::Text(id)],
            )?;
            if !existing.is_empty() || investigation_id(commit, tree, question)? != id {
                return refuse("investigation already exists or identity differs");
            }
            (
                None,
                question.to_owned(),
                commit.to_owned(),
                tree.to_owned(),
            )
        } else {
            let history = records(db, repo, id)?;
            if history.len() != expected_revision {
                return refuse("stale investigation expected revision");
            }
            let last = history.last().ok_or("investigation has no events")?;
            (
                Some(last.get("event_id")?.str()?.to_owned()),
                last.get("question")?.str()?.to_owned(),
                last.get("source_commit")?.str()?.to_owned(),
                last.get("source_tree")?.str()?.to_owned(),
            )
        };
        let event = obj(&[
            ("schema", Value::string(SCHEMA)),
            ("investigation_id", Value::string(id)),
            ("revision", Value::number(expected_revision + 1)),
            (
                "previous_event_id",
                prior.as_deref().map(Value::string).unwrap_or(Value::Null),
            ),
            ("question", Value::string(question)),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("kind", Value::string(kind)),
            ("payload", payload),
        ])
        .stamp_id("event_id")?;
        let bytes = event.canonical();
        if bytes.len() > MAX_EVENT_BYTES {
            return refuse("investigation event exceeds 4 MiB");
        }
        let event_id = event.get("event_id")?.str()?.to_owned();
        db.execute(
            "INSERT INTO investigation_event_v1
             (investigation_id,revision,event_id,previous_event_id,document)
             VALUES(?,?,?,?,?)",
            &[
                Param::Text(id),
                Param::Integer((expected_revision + 1) as i64),
                Param::Text(&event_id),
                prior.as_deref().map(Param::Text).unwrap_or(Param::Null),
                Param::Text(&bytes),
            ],
        )?;
        // Validate the complete persisted chain, not just the newly supplied payload.
        records(db, repo, id)?;
        Ok(event)
    })
}

pub fn start(db: &Db, repo: &Path, rev: &str, question: &str) -> Result<Value> {
    bounded(question, "question")?;
    let source = discovery::inspect(repo, rev)?;
    let id = investigation_id(&source.commit, &source.tree, question)?;
    append(
        db,
        &source.root,
        &id,
        0,
        "CREATE",
        obj(&[]),
        Some((question, &source.commit, &source.tree)),
    )
}

pub struct Claim<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub path: &'a str,
    pub statement: &'a str,
    pub assumption: &'a str,
    pub assessment: &'a str,
}

pub fn claim(db: &Db, repo: &Path, request: Claim<'_>) -> Result<Value> {
    bounded(request.statement, "claim")?;
    bounded(request.assumption, "claim assumption")?;
    if !matches!(request.assessment, "HYPOTHESIS" | "SOURCE_REVIEW_REPORTED") {
        return refuse("claim assessment must be HYPOTHESIS or SOURCE_REVIEW_REPORTED");
    }
    // Resolve source from the ledger's pinned commit, never the working tree.
    let history = audit(db, repo, request.id)?;
    let commit = history.get("source_commit")?.str()?;
    let source = SourceRef::from_git(repo, commit, request.path)?;
    append(
        db,
        repo,
        request.id,
        request.expected_revision,
        "CLAIM",
        obj(&[
            ("source", source_value(&source)),
            ("statement", Value::string(request.statement)),
            ("assumption", Value::string(request.assumption)),
            ("assessment", Value::string(request.assessment)),
        ]),
        None,
    )
}

pub fn plan(
    db: &Db,
    repo: &Path,
    id: &str,
    expected_revision: usize,
    claim_event_id: &str,
    obligation: &str,
) -> Result<Value> {
    bounded(obligation, "preservation obligation")?;
    append(
        db,
        repo,
        id,
        expected_revision,
        "PLAN",
        obj(&[
            ("claim_event_id", Value::string(claim_event_id)),
            ("obligation", Value::string(obligation)),
        ]),
        None,
    )
}

pub struct Verification<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub plan_event_id: &'a str,
    pub candidate: &'a str,
    pub limit: usize,
    pub roots: Option<&'a str>,
    pub evidence_db: Option<&'a str>,
    pub receipts: Option<&'a Path>,
}

pub fn verify(db: &Db, repo: &Path, request: Verification<'_>) -> Result<Value> {
    let history = audit(db, repo, request.id)?;
    if history.get("revision")?.integer()? != request.expected_revision as i64 {
        return refuse("stale investigation expected revision");
    }
    let base = history.get("source_commit")?.str()?;
    let report = crate::agent::verify(
        repo,
        base,
        request.candidate,
        request.limit,
        request.roots,
        request.evidence_db,
        request.receipts,
    )?;
    verify_agent_report(repo, base, history.get("source_tree")?.str()?, &report)?;
    append(
        db,
        repo,
        request.id,
        request.expected_revision,
        "VERIFY",
        obj(&[
            ("plan_event_id", Value::string(request.plan_event_id)),
            ("report", report),
        ]),
        None,
    )
}
