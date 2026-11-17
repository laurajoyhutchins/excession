//! Demand-driven investigation work over the existing Git and SQLite ledger.
//! Retrieval supplies inspection candidates, not semantic claims. Synthesis
//! retains author attribution, and refresh detects direct source changes
//! without assuming transitive behavioral equivalence.

use crate::evidence::identity::SourceRef;
use crate::json::{Value, obj};
use crate::{Result, agent_locate, change_inventory, digest, investigation, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const EXPLANATION_SCHEMA: &str = "excession.investigation-explanation.v1";
const TEXT_LIMIT: usize = 8192;

fn text(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > TEXT_LIMIT || value.contains('\0') {
        return refuse(format!("invalid investigation {name}"));
    }
    Ok(())
}

fn hash(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return refuse("invalid investigation event reference digest");
    }
    Ok(())
}

fn source_candidate(repo: &Path, commit: &str, tree: &str, value: &Value) -> Result<()> {
    if value.entries()?.len() != 5
        || value.get("source_commit")?.str()? != commit
        || value.get("source_tree")?.str()? != tree
    {
        return refuse("discovery candidate is not pinned to the investigation source");
    }
    let path = value.get("path")?.str()?;
    let actual = SourceRef::from_git(repo, commit, path)?;
    if actual.tree != tree
        || actual.blob != value.get("blob")?.str()?
        || actual.mode != value.get("mode")?.str()?
    {
        return refuse("discovery candidate Git blob or mode differs");
    }
    Ok(())
}

fn check_discovery(
    repo: &Path,
    commit: &str,
    tree: &str,
    question: &str,
    report: &Value,
) -> Result<()> {
    if report.get("schema")?.str()? != "excession.agent-locate.v1"
        || report.get("source_commit")?.str()? != commit
        || report.get("source_tree")?.str()? != tree
        || report.get("task")?.str()? != question
        || report.get("coverage")?.get("status")?.str()? != "PARTIAL"
        || report.get("safe_to_skip_unlisted_tests")?.bool()?
        || report
            .get("reading_plan")?
            .get("complete_behavior_trace")?
            .bool()?
        || !report.content_id_matches("report_id")?
        || report.entries()?.contains_key("cache")
    {
        return refuse("investigation discovery report source, scope or digest mismatch");
    }
    // Authenticate the source boundaries that will be presented to the user.
    // Nested context is original provider evidence, not independent proof.
    for collection in ["candidates", "relationship_candidates"] {
        for item in report.get(collection)?.array_ref()? {
            source_candidate(repo, commit, tree, item.get("source")?)?;
        }
    }
    for step in report.get("reading_plan")?.get("steps")?.array_ref()? {
        if step.get("behavior_verified")?.bool()? {
            return refuse("source reading suggestion cannot be marked verified");
        }
        source_candidate(repo, commit, tree, step.get("source")?)?;
    }
    Ok(())
}

fn prior_reference(prior: &[Value], event_id: &str, expected: &str) -> Result<()> {
    hash(event_id)?;
    if !prior.iter().any(|event| {
        event.get("event_id").and_then(Value::str).ok() == Some(event_id)
            && event.get("kind").and_then(Value::str).ok() == Some(expected)
    }) {
        return refuse(format!(
            "investigation {expected} reference does not precede event"
        ));
    }
    Ok(())
}

fn refresh_report(
    repo: &Path,
    base: &str,
    tree: &str,
    candidate: &str,
    prior: &[Value],
) -> Result<Value> {
    let inventory = change_inventory::inventory(repo, base, candidate)?;
    if inventory.get("base_commit")?.str()? != base
        || inventory.get("base_tree")?.str()? != tree
        || !inventory.content_id_matches("report_id")?
    {
        return refuse("refresh change inventory does not match investigation source");
    }
    let candidate_commit = inventory.get("head_commit")?.str()?;
    let candidate_tree = inventory.get("head_tree")?.str()?;
    if candidate_commit == base {
        return refuse("refresh requires a distinct candidate Git commit");
    }
    let changes = inventory.get("changes")?.array_ref()?;
    // There is no artificial cap on changed paths or claim count. The native
    // inventory is complete and no result is silently omitted.
    let mut changed = BTreeMap::<String, &Value>::new();
    for change in changes {
        let before = change.get("before")?;
        let after = change.get("after")?;
        let source = if before != &Value::Null {
            before
        } else {
            after
        };
        changed.insert(source.get("path")?.str()?.to_owned(), after);
    }
    let mut assessments = Vec::new();
    for event in prior
        .iter()
        .filter(|event| event.get("kind").and_then(Value::str).ok() == Some("CLAIM"))
    {
        let old = event.get("payload")?.get("source")?;
        let path = old.get("path")?.str()?;
        let new = changed.get(path).copied();
        let status = match new {
            None => "DIRECT_SOURCE_UNCHANGED",
            Some(&Value::Null) => "DIRECT_SOURCE_UNAVAILABLE",
            Some(value) if !matches!(value.get("mode")?.str()?, "100644" | "100755") => {
                "DIRECT_SOURCE_UNAVAILABLE"
            }
            Some(value)
                if value.get("blob")?.str()? == old.get("blob")?.str()?
                    && value.get("mode")?.str()? == old.get("mode")?.str()? =>
            {
                "DIRECT_SOURCE_UNCHANGED"
            }
            Some(_) => "DIRECT_SOURCE_CHANGED",
        };
        assessments.push(obj(&[
            ("claim_event_id", event.get("event_id")?.clone()),
            ("path", old.get("path")?.clone()),
            ("direct_source", Value::string(status)),
            ("semantic_impact", Value::string("UNRESOLVED")),
        ]));
    }
    obj(&[
        ("candidate_commit", Value::string(candidate_commit)),
        ("candidate_tree", Value::string(candidate_tree)),
        ("change_inventory_id", inventory.get("report_id")?.clone()),
        ("changed_path_count", Value::number(changes.len())),
        ("claim_assessments", Value::array(assessments)),
        ("dependency_completeness", Value::string("NOT_ESTABLISHED")),
        ("semantic_verification", Value::string("NOT_ESTABLISHED")),
    ])
    .stamp_id("refresh_id")
}

/// Invoked by the ledger replay for each new immutable event. It rechecks
/// Git objects and causal references, rather than trusting persisted rows.
pub(crate) fn validate_event(
    repo: &Path,
    commit: &str,
    tree: &str,
    question: &str,
    kind: &str,
    payload: &Value,
    prior: &[Value],
) -> Result<()> {
    match kind {
        "EXPLORE" => {
            if payload.entries()?.len() != 1 {
                return refuse("investigation exploration payload has unexpected fields");
            }
            check_discovery(repo, commit, tree, question, payload.get("report")?)
        }
        "QUESTION" => {
            if payload.entries()?.len() != 2 {
                return refuse("investigation open question has unexpected fields");
            }
            text(payload.get("question")?.str()?, "open question")?;
            text(payload.get("reason")?.str()?, "open question reason")
        }
        "SYNTHESIS" => {
            if payload.entries()?.len() != 2 {
                return refuse("investigation synthesis has unexpected fields");
            }
            text(payload.get("narrative")?.str()?, "explanation")?;
            let ids = payload.get("claim_event_ids")?.array_ref()?;
            if ids.is_empty() {
                return refuse("explanation needs at least one supporting claim");
            }
            let mut unique = BTreeSet::new();
            for event_id in ids {
                let id = event_id.str()?;
                prior_reference(prior, id, "CLAIM")?;
                if !unique.insert(id) {
                    return refuse("explanation claim references must be unique");
                }
            }
            Ok(())
        }
        "REFRESH" => {
            if payload.entries()?.len() != 8
                || !payload
                    .get("refresh_id")?
                    .str()?
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return refuse("invalid refresh report shape");
            }
            if payload
                != &refresh_report(
                    repo,
                    commit,
                    tree,
                    payload.get("candidate_commit")?.str()?,
                    prior,
                )?
            {
                return refuse("investigation refresh no longer matches Git source");
            }
            Ok(())
        }
        _ => refuse("unsupported investigation workflow event"),
    }
}

pub struct Explore<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub limit: usize,
    pub budget: agent_locate::LocateBudget,
    pub roots: Option<&'a str>,
    pub evidence_db: Option<&'a str>,
}

pub fn explore(db: &crate::sqlite::Db, repo: &Path, options: Explore<'_>) -> Result<Value> {
    let history = investigation::audit(db, repo, options.id)?;
    if history.get("revision")?.integer()? != options.expected_revision as i64 {
        return refuse("stale investigation expected revision");
    }
    if !(1..=64).contains(&options.limit) || !(1..=256).contains(&options.budget.index_segments) {
        return refuse("investigation explore limit must be 1..64 and index-segments 1..256");
    }
    let commit = history.get("source_commit")?.str()?;
    let tree = history.get("source_tree")?.str()?;
    let question = history.get("question")?.str()?;
    let mut report = agent_locate::locate_with_limits(
        repo,
        commit,
        question,
        options.limit,
        options.roots,
        options.evidence_db,
        options.budget,
    )?;
    // The original locate report excludes volatile cache stats from its
    // stable report_id. Persist its exact stable portion, not a cache path.
    report.entries_mut()?.remove("cache");
    check_discovery(repo, commit, tree, question, &report)?;
    investigation::append(
        db,
        repo,
        options.id,
        options.expected_revision,
        "EXPLORE",
        obj(&[("report", report)]),
        None,
    )
}

pub fn question(
    db: &crate::sqlite::Db,
    repo: &Path,
    id: &str,
    revision: usize,
    question: &str,
    reason: &str,
) -> Result<Value> {
    text(question, "open question")?;
    text(reason, "open question reason")?;
    investigation::append(
        db,
        repo,
        id,
        revision,
        "QUESTION",
        obj(&[
            ("question", Value::string(question)),
            ("reason", Value::string(reason)),
        ]),
        None,
    )
}

pub fn synthesize(
    db: &crate::sqlite::Db,
    repo: &Path,
    id: &str,
    revision: usize,
    narrative: &str,
    claim_ids: &str,
) -> Result<Value> {
    text(narrative, "explanation")?;
    let mut seen = BTreeSet::new();
    let ids = claim_ids
        .split(',')
        .map(str::trim)
        .map(|id| {
            hash(id)?;
            if !seen.insert(id.to_owned()) {
                return refuse("repeated claim in synthesis");
            }
            Ok(Value::string(id))
        })
        .collect::<Result<Vec<_>>>()?;
    if ids.is_empty() {
        return refuse("explanation needs supporting claim event IDs");
    }
    investigation::append(
        db,
        repo,
        id,
        revision,
        "SYNTHESIS",
        obj(&[
            ("narrative", Value::string(narrative)),
            ("claim_event_ids", Value::array(ids)),
        ]),
        None,
    )
}

pub fn refresh(
    db: &crate::sqlite::Db,
    repo: &Path,
    id: &str,
    revision: usize,
    candidate: &str,
) -> Result<Value> {
    let history = investigation::audit(db, repo, id)?;
    if history.get("revision")?.integer()? != revision as i64 {
        return refuse("stale investigation expected revision");
    }
    let events = history.get("events")?.array_ref()?;
    let report = refresh_report(
        repo,
        history.get("source_commit")?.str()?,
        history.get("source_tree")?.str()?,
        candidate,
        events,
    )?;
    investigation::append(db, repo, id, revision, "REFRESH", report, None)
}

/// Build a reviewable explanation projection from authenticated event history.
/// Neither the narrative nor the source review is promoted to a proof.
pub fn explain(db: &crate::sqlite::Db, repo: &Path, id: &str) -> Result<Value> {
    let history = investigation::audit(db, repo, id)?;
    let mut sources = BTreeMap::<String, Value>::new();
    let mut claims = Vec::new();
    let mut narratives = Vec::new();
    let mut open_questions = Vec::new();
    let mut retrievals = Vec::new();
    let mut evidence_gaps = Vec::new();
    let mut preservation_plans = Vec::new();
    let mut behavior_witnesses = Vec::new();
    let mut prior_witnesses = BTreeMap::<String, Vec<Value>>::new();
    let mut verification_receipts = Vec::new();
    let mut last_refresh = Value::Null;
    for event in history.get("events")?.array_ref()? {
        let payload = event.get("payload")?;
        match event.get("kind")?.str()? {
            "CLAIM" => {
                let source = payload.get("source")?;
                sources.insert(source.get("path")?.str()?.to_owned(), source.clone());
                claims.push(obj(&[
                    ("event_id", event.get("event_id")?.clone()),
                    ("source", source.clone()),
                    ("statement", payload.get("statement")?.clone()),
                    ("assumption", payload.get("assumption")?.clone()),
                    ("assessment", payload.get("assessment")?.clone()),
                ]));
            }
            "EXPLORE" => {
                let report = payload.get("report")?;
                retrievals.push(obj(&[
                    ("event_id", event.get("event_id")?.clone()),
                    ("report_id", report.get("report_id")?.clone()),
                    ("reading_plan", report.get("reading_plan")?.clone()),
                    (
                        "index_complete",
                        report.get("index")?.get("complete")?.clone(),
                    ),
                    ("coverage", report.get("coverage")?.clone()),
                ]));
                for gap in report.get("coverage")?.get("gaps")?.array_ref()? {
                    evidence_gaps.push(obj(&[
                        ("exploration_event_id", event.get("event_id")?.clone()),
                        ("reason", gap.get("reason")?.clone()),
                        ("state", Value::string("UNRESOLVED")),
                    ]));
                }
            }
            "QUESTION" => open_questions.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("question", payload.get("question")?.clone()),
                ("reason", payload.get("reason")?.clone()),
                ("state", Value::string("OPEN")),
            ])),
            "SYNTHESIS" => narratives.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("narrative", payload.get("narrative")?.clone()),
                ("claim_event_ids", payload.get("claim_event_ids")?.clone()),
                (
                    "authorship",
                    Value::string("REPORTED_NOT_INDEPENDENTLY_VALIDATED"),
                ),
            ])),
            "WITNESS" => {
                prior_witnesses.entry(payload.get("claim_event_id")?.str()?.to_owned())
                    .or_default().push(event.get("event_id")?.clone());
                behavior_witnesses.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("claim_event_id", payload.get("claim_event_id")?.clone()),
                ("source", payload.get("source")?.clone()),
                ("first_line", payload.get("first_line")?.clone()),
                ("last_line", payload.get("last_line")?.clone()),
                ("source_text", payload.get("source_text")?.clone()),
                ("source_text_sha256", payload.get("source_text_sha256")?.clone()),
                ("kind", payload.get("kind")?.clone()),
                ("interpretation", payload.get("interpretation")?.clone()),
                ("assessment", payload.get("assessment")?.clone()),
                ("runtime_verified", Value::Bool(false)),
                ]));
            }
            "REFRESH" => last_refresh = payload.clone(),
            "PLAN" => preservation_plans.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("claim_event_id", payload.get("claim_event_id")?.clone()),
                ("obligation", payload.get("obligation")?.clone()),
                ("witness_event_ids", Value::array(prior_witnesses
                    .get(payload.get("claim_event_id")?.str()?)
                    .cloned().unwrap_or_default())),
                ("state", Value::string("OPEN")),
            ])),
            "VERIFY" => verification_receipts.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("plan_event_id", payload.get("plan_event_id")?.clone()),
                (
                    "report_id",
                    payload.get("report")?.get("report_id")?.clone(),
                ),
                ("status", payload.get("report")?.get("status")?.clone()),
                ("ready_to_merge", Value::Bool(false)),
            ])),
            _ => {}
        }
    }
    let mut output = obj(&[
        ("schema", Value::string(EXPLANATION_SCHEMA)),
        ("investigation_id", Value::string(id)),
        ("source_commit", history.get("source_commit")?.clone()),
        ("source_tree", history.get("source_tree")?.clone()),
        ("question", history.get("question")?.clone()),
        ("revision", history.get("revision")?.clone()),
        ("implementation_map", Value::array(sources.into_values())),
        ("claims", Value::array(claims)),
        ("narratives", Value::array(narratives)),
        ("open_questions", Value::array(open_questions)),
        ("retrievals", Value::array(retrievals)),
        ("evidence_gaps", Value::array(evidence_gaps)),
        ("behavior_witnesses", Value::array(behavior_witnesses)),
        ("preservation_plans", Value::array(preservation_plans)),
        ("verification_receipts", Value::array(verification_receipts)),
        ("latest_refresh", last_refresh),
        (
            "completeness",
            Value::string("INCOMPLETE_BEHAVIORAL_COVERAGE"),
        ),
        ("semantic_verification", Value::string("NOT_ESTABLISHED")),
        ("ready_to_merge", Value::Bool(false)),
    ]);
    let report_id = digest::sha256(output.canonical().as_bytes());
    output
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok(output)
}
