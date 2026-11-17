//! Best-effort, trusted-local CPython event traces over the canonical
//! investigation execution worktrees. Nothing here is sandboxed, signed,
//! a complete runtime trace or a proof of behavioral preservation.
use crate::evidence::identity::SourceRef;
use crate::investigation_execution as exec;
use crate::json::{self, Value, obj};
use crate::sqlite::Db;
use crate::{Result, digest, git, investigation, investigation_witness, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FORMAT: &str = "excession.investigation-runtime-trace.v1";

pub struct Request<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub plan_event_id: &'a str,
    pub candidate: &'a str,
    pub program: &'a Path,
    pub driver: &'a Path,
    pub script: &'a str,
    pub args_json: &'a str,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    pub max_trace_events: usize,
    pub max_trace_bytes: u64,
    pub trusted_local: bool,
}

fn safe_host_file(repo: &Path, path: &Path) -> Result<(PathBuf, String)> {
    if !path.is_absolute() {
        return refuse("runtime trace requires absolute trusted host paths");
    }
    let real = path.canonicalize().map_err(|e| format!("trace tool resolution: {e}"))?;
    let root = repo.canonicalize().map_err(|e| format!("source checkout: {e}"))?;
    if !real.is_file() || real.starts_with(root) {
        return refuse("trace tool must be a regular host file outside source checkout");
    }
    Ok((real.clone(), exec::sha256_file(&real)?.0))
}

fn args(raw: &str) -> Result<Vec<String>> {
    if raw.len() > 64 * 1024 {
        return refuse("runtime trace argument JSON exceeds 64 KiB");
    }
    let values: Vec<String> = serde_json::from_str(raw)
        .map_err(|e| format!("runtime trace --argv-json must be a string array: {e}"))?;
    if values.len() > 128
        || values.iter().any(|s| s.len() > 8192 || s.contains('\0'))
    {
        return refuse("runtime trace arguments exceed count or size budget");
    }
    Ok(values)
}

fn validate_event_shape(event: &Value) -> Result<(&str, usize)> {
    if event.entries()?.len() != 7 {
        return refuse("runtime trace event has unexpected fields");
    }
    let kind = event.get("kind")?.str()?;
    if !matches!(kind, "CALL" | "LINE" | "RETURN" | "EXCEPTION") {
        return refuse("runtime trace event kind unsupported");
    }
    let path = event.get("path")?.str()?;
    git::validate_path(path)?;
    if !path.ends_with(".py") {
        return refuse("CPython trace event is not attributed to a Python source file");
    }
    let line = usize::try_from(event.get("line")?.integer()?)
        .map_err(|_| "invalid runtime trace line number".to_owned())?;
    if line == 0 {
        return refuse("runtime trace line must be positive");
    }
    let function = event.get("function")?.str()?;
    if function.is_empty() || function.len() > 256 {
        return refuse("invalid runtime trace function name");
    }
    if event.get("values")?.entries()?.len() > 128
        || event.get("changes")?.array_ref()?.len() > 256
        || !matches!(event.get("return_value")?, Value::Null | Value::Object(_))
    {
        return refuse("runtime trace value payload exceeds supported shape");
    }
    Ok((path, line))
}

fn source_refs(
    repo: &Path,
    commit: &str,
    events: &[Value],
) -> Result<Vec<Value>> {
    let mut max_line = BTreeMap::<String, usize>::new();
    for event in events {
        let (path, line) = validate_event_shape(event)?;
        let largest = max_line.entry(path.to_owned()).or_default();
        *largest = (*largest).max(line);
    }
    let mut sources = Vec::new();
    for (path, line) in max_line {
        let source = SourceRef::from_git(repo, commit, &path)?;
        // Validate the maximum observed line in the authenticated original
        // Git blob. This also refuses a fabricated out-of-range trace.
        investigation_witness::exact_span(repo, &source, line, line)?;
        sources.push(investigation::source_value(&source));
    }
    Ok(sources)
}

fn no_trace(status: &str) -> Value {
    obj(&[
        ("status", Value::string(status)),
        ("events", Value::array(vec![])),
        ("sources", Value::array(vec![])),
        ("reported_event_count", Value::number(0)),
        ("values_partial", Value::Bool(false)),
        ("raw_sha256", Value::Null),
    ])
}

/// A bounded JSONL reader, independently validating all source identities.
/// The trace content is *reported by a local instrumented process*, not
/// independently attested or guaranteed complete by the interpreter.
fn read_trace(repo: &Path, commit: &str, path: &Path, max_bytes: u64, max_events: usize) -> Result<Value> {
    if !path.exists() {
        return Ok(no_trace("MISSING"));
    }
    let raw_len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if raw_len > max_bytes.saturating_add(512) {
        return refuse("runtime trace sidecar exceeds configured trace byte budget");
    }
    let raw = fs::read(path).map_err(|e| format!("read runtime trace sidecar: {e}"))?;
    let data = std::str::from_utf8(&raw)
        .map_err(|_| "runtime trace sidecar is not valid UTF-8".to_owned())?;
    let mut events = Vec::new();
    let mut end = None;
    for (i, line) in data.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let item = json::parse(line.as_bytes())?;
        if item.get("kind")?.str()? == "END" {
            if item.entries()?.len() != 5 || end.is_some() {
                return refuse("runtime trace has malformed or duplicate END marker");
            }
            end = Some(item);
        } else {
            if end.is_some() || i > max_events || events.len() >= max_events {
                return refuse("runtime trace events exceed configured event budget or follow END");
            }
            validate_event_shape(&item)?;
            events.push(item);
        }
    }
    let Some(end) = end else {
        return Ok(no_trace("MISSING_END"));
    };
    if end.get("events")?.integer()? != events.len() as i64
        || end.get("complete")?.bool()? == end.get("truncated")?.bool()?
    {
        return refuse("runtime trace END marker contradicts event stream");
    }
    let sources = source_refs(repo, commit, &events)?;
    Ok(obj(&[
        ("status", Value::string(if end.get("truncated")?.bool()? {
            "TRUNCATED"
        } else { "REPORTED_COMPLETE" })),
        ("events", Value::array(events)),
        ("sources", Value::array(sources)),
        ("reported_event_count", Value::number(end.get("events")?.integer()?)),
        ("values_partial", end.get("values_partial")?.clone()),
        ("raw_sha256", Value::string(digest::sha256(&raw))),
    ]))
}

fn witness_hits(prior: &[Value], witness_ids: &[Value], base_trace: &Value) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for witness_id in witness_ids {
        let id = witness_id.str()?;
        let witness = prior.iter().find(|event| {
            event.get("kind").and_then(Value::str).ok() == Some("WITNESS")
                && event.get("event_id").and_then(Value::str).ok() == Some(id)
        }).ok_or("runtime trace references absent source witness")?;
        let source = witness.get("payload")?.get("source")?;
        let first = witness.get("payload")?.get("first_line")?.integer()?;
        let last = witness.get("payload")?.get("last_line")?.integer()?;
        let count = base_trace.get("events")?.array_ref()?.iter().filter(|event| {
            event.get("kind").and_then(Value::str).ok() == Some("LINE")
                && event.get("path").and_then(Value::str).ok()
                    == source.get("path").and_then(Value::str).ok()
                && event.get("line").and_then(Value::integer).ok()
                    .is_some_and(|line| line >= first && line <= last)
        }).count();
        result.push(obj(&[
            ("witness_event_id", Value::string(id)),
            ("base_line_event_count", Value::number(count)),
            ("interpretation_verified", Value::Bool(false)),
        ]));
    }
    Ok(result)
}

fn trace_comparison(old: &Value, new: &Value, a: &Value, b: &Value) -> Result<&'static str> {
    if old.get("status")?.str()? != "REPORTED_COMPLETE"
        || new.get("status")?.str()? != "REPORTED_COMPLETE"
        || a.get("status")?.str()? != "EXITED"
        || b.get("status")?.str()? != "EXITED"
    {
        return Ok("INCONCLUSIVE");
    }
    if old.get("events")? == new.get("events")? {
        Ok("MATCHING_REPORTED_EVENTS")
    } else {
        Ok("DIFFERING_REPORTED_EVENTS")
    }
}

fn validate_trace(
    repo: &Path,
    commit: &str,
    tree: &str,
    trace: &Value,
    max_events: usize,
) -> Result<()> {
    if trace.entries()?.len() != 6
        || !matches!(trace.get("status")?.str()?,
            "MISSING" | "MISSING_END" | "TRUNCATED" | "REPORTED_COMPLETE")
        || trace.get("events")?.array_ref()?.len() > max_events
        || trace.get("reported_event_count")?.integer()?
            != trace.get("events")?.array_ref()?.len() as i64
    {
        return refuse("runtime trace observation has invalid shape or event count");
    }
    let status = trace.get("status")?.str()?;
    if matches!(status, "MISSING" | "MISSING_END") {
        if !trace.get("events")?.array_ref()?.is_empty()
            || !trace.get("sources")?.array_ref()?.is_empty()
            || trace.get("raw_sha256")? != &Value::Null
        {
            return refuse("missing runtime trace cannot assert observations");
        }
    } else if trace.get("raw_sha256")?.str()?.len() != 64 {
        return refuse("runtime trace raw SHA-256 is malformed");
    }
    trace.get("values_partial")?.bool()?;
    if git::rev(repo, &format!("{commit}^{{tree}}"))? != tree
        || trace.get("sources")? != &Value::array(source_refs(
            repo, commit, trace.get("events")?.array_ref()?,
        )?)
    {
        return refuse("runtime trace source identities do not match Git");
    }
    Ok(())
}

pub(crate) fn validate_event(
    repo: &Path,
    commit: &str,
    tree: &str,
    payload: &Value,
    prior: &[Value],
) -> Result<()> {
    if payload.entries()?.len() != 2 {
        return refuse("runtime trace event requires plan and report");
    }
    let plan = payload.get("plan_event_id")?.str()?;
    let expected = exec::planned_witnesses(prior, plan)?;
    let report = payload.get("report")?;
    let keys = report.entries()?.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_keys = BTreeSet::from([
        "schema", "base_commit", "base_tree", "candidate_commit", "candidate_tree",
        "plan_event_id", "witness_event_ids", "program", "driver", "script",
        "argv", "limits", "base_execution", "candidate_execution", "base_trace",
        "candidate_trace", "process_comparison", "trace_comparison",
        "base_witness_line_hits", "trust", "semantic_verification",
        "ready_to_merge", "report_id",
    ]);
    if keys != expected_keys
        || report.get("schema")?.str()? != FORMAT
        || report.get("base_commit")?.str()? != commit
        || report.get("base_tree")?.str()? != tree
        || report.get("plan_event_id")?.str()? != plan
        || report.get("witness_event_ids")?.array_ref()? != expected.as_slice()
        || report.get("trust")?.str()? != "BEST_EFFORT_TRUSTED_LOCAL_CPYTHON_UNATTESTED"
        || report.get("semantic_verification")?.str()? != "NOT_ESTABLISHED"
        || report.get("ready_to_merge")?.bool()?
        || !report.content_id_matches("report_id")?
    {
        return refuse("runtime trace provenance, history or safety contract mismatch");
    }
    let candidate = report.get("candidate_commit")?.str()?;
    if candidate == commit || exec::verified_revision(repo, candidate)?
        != report.get("candidate_tree")?.str()?
    {
        return refuse("runtime trace candidate Git identity mismatch");
    }
    for key in ["program", "driver"] {
        let v = report.get(key)?;
        if v.entries()?.len() != 2
            || !Path::new(v.get("path")?.str()?).is_absolute()
            || v.get("sha256")?.str()?.len() != 64
        {
            return refuse("runtime trace host tool identity malformed");
        }
    }
    git::validate_path(report.get("script")?.str()?)?;
    for v in report.get("argv")?.array_ref()? {
        v.str()?;
    }
    let limits = report.get("limits")?;
    if limits.entries()?.len() != 4
        || limits.get("timeout_ms")?.integer()? <= 0
        || limits.get("max_output_bytes")?.integer()? <= 0
        || limits.get("max_trace_events")?.integer()? <= 0
        || limits.get("max_trace_bytes")?.integer()? < 1024
    {
        return refuse("runtime trace limits malformed");
    }
    let budget = usize::try_from(limits.get("max_trace_events")?.integer()?)
        .map_err(|_| "runtime trace event budget not representable".to_owned())?;
    validate_trace(repo, commit, tree, report.get("base_trace")?, budget)?;
    validate_trace(repo, candidate, report.get("candidate_tree")?.str()?,
        report.get("candidate_trace")?, budget)?;
    if report.get("base_witness_line_hits")?
        != &Value::array(witness_hits(prior, &expected, report.get("base_trace")?)?)
    {
        return refuse("runtime trace witness event coverage contradicts source observations");
    }
    let a = report.get("base_execution")?;
    let b = report.get("candidate_execution")?;
    if a.get("commit")?.str()? != commit || a.get("tree")?.str()? != tree
        || b.get("commit")?.str()? != candidate
        || b.get("tree")?.str()? != report.get("candidate_tree")?.str()?
        || report.get("process_comparison")?.str()? != exec::comparison(a, b)?
        || report.get("trace_comparison")?.str()?
            != trace_comparison(report.get("base_trace")?,
                report.get("candidate_trace")?, a, b)?
    {
        return refuse("runtime trace comparison contradicts retained observations");
    }
    Ok(())
}

fn observed(
    repo: &Path,
    worktree: &Path,
    revision: &str,
    tree: &str,
    program: &Path,
    driver: &Path,
    script: &str,
    args: &[String],
    request: &Request<'_>,
) -> Result<(Value, Value)> {
    let root = worktree.parent().ok_or("runtime trace worktree missing parent")?;
    let label = worktree.file_name().and_then(|v| v.to_str())
        .ok_or("runtime trace worktree name invalid")?;
    let trace = root.join(format!(".{label}.trace.jsonl"));
    let mut argv = vec![
        "-I".to_owned(),
        driver.to_string_lossy().into_owned(),
        "--root".into(), worktree.to_string_lossy().into_owned(),
        "--out".into(), trace.to_string_lossy().into_owned(),
        "--max-events".into(), request.max_trace_events.to_string(),
        "--max-bytes".into(), request.max_trace_bytes.to_string(),
        script.to_owned(),
    ];
    argv.extend(args.iter().cloned());
    let output = exec::observation(
        worktree, revision, tree, program, &argv,
        Duration::from_millis(request.timeout_ms), request.max_output_bytes,
    )?;
    // Source mutations would make the trace refer to content other than the
    // authenticated commit. Even on failure, the worktree is deleted by Drop.
    git::clean(worktree)?;
    let recorded = read_trace(repo, revision, &trace,
        request.max_trace_bytes, request.max_trace_events)?;
    let _ = fs::remove_file(trace);
    Ok((output, recorded))
}

pub fn run(db: &Db, repo: &Path, request: Request<'_>) -> Result<Value> {
    if !request.trusted_local {
        return refuse("runtime instrumentation requires --trusted-local true; no sandbox is provided");
    }
    if request.timeout_ms == 0 || request.max_output_bytes == 0
        || request.max_trace_events == 0 || request.max_trace_bytes < 1024
        || request.timeout_ms > i64::MAX as u64
        || request.max_output_bytes > i64::MAX as u64
        || request.max_trace_events > i64::MAX as usize
        || request.max_trace_bytes > i64::MAX as u64
    {
        return refuse("runtime tracing requires positive, representable resource budgets");
    }
    let (program, program_sha) = safe_host_file(repo, request.program)?;
    let (driver, driver_sha) = safe_host_file(repo, request.driver)?;
    git::validate_path(request.script)?;
    if !request.script.ends_with(".py") {
        return refuse("runtime tracing currently supports Git-pinned Python scripts");
    }
    let arguments = args(request.args_json)?;
    let history = investigation::audit(db, repo, request.id)?;
    if history.get("revision")?.integer()? != request.expected_revision as i64 {
        return refuse("stale investigation expected revision");
    }
    let base = history.get("source_commit")?.str()?;
    let tree = history.get("source_tree")?.str()?;
    let prior = history.get("events")?.array_ref()?;
    let witnesses = exec::planned_witnesses(prior, request.plan_event_id)?;
    let candidate = git::rev(repo, &format!("{}^{{commit}}", request.candidate))?;
    let candidate_tree = exec::verified_revision(repo, &candidate)?;
    if base == candidate {
        return refuse("runtime tracing needs two distinct Git commits");
    }
    for revision in [base, candidate.as_str()] {
        SourceRef::from_git(repo, revision, request.script)?;
    }
    let mut temp = exec::Worktrees::new(repo)?;
    let base_root = temp.add("base", base)?;
    let candidate_root = temp.add("candidate", &candidate)?;
    let (base_execution, base_trace) = observed(
        repo, &base_root, base, tree, &program, &driver,
        request.script, &arguments, &request,
    )?;
    let (candidate_execution, candidate_trace) = observed(
        repo, &candidate_root, &candidate, &candidate_tree,
        &program, &driver, request.script, &arguments, &request,
    )?;
    temp.cleanup()?;
    let report = obj(&[
        ("schema", Value::string(FORMAT)),
        ("base_commit", Value::string(base)),
        ("base_tree", Value::string(tree)),
        ("candidate_commit", Value::string(&candidate)),
        ("candidate_tree", Value::string(&candidate_tree)),
        ("plan_event_id", Value::string(request.plan_event_id)),
        ("witness_event_ids", Value::array(witnesses.clone())),
        ("program", obj(&[
            ("path", Value::string(program.to_string_lossy())),
            ("sha256", Value::string(program_sha)),
        ])),
        ("driver", obj(&[
            ("path", Value::string(driver.to_string_lossy())),
            ("sha256", Value::string(driver_sha)),
        ])),
        ("script", Value::string(request.script)),
        ("argv", Value::array(arguments.into_iter().map(Value::string))),
        ("limits", obj(&[
            ("timeout_ms", Value::number(request.timeout_ms)),
            ("max_output_bytes", Value::number(request.max_output_bytes)),
            ("max_trace_events", Value::number(request.max_trace_events)),
            ("max_trace_bytes", Value::number(request.max_trace_bytes)),
        ])),
        ("base_execution", base_execution.clone()),
        ("candidate_execution", candidate_execution.clone()),
        ("base_trace", base_trace.clone()),
        ("candidate_trace", candidate_trace.clone()),
        ("process_comparison", Value::string(exec::comparison(
            &base_execution, &candidate_execution,
        )?)),
        ("trace_comparison", Value::string(trace_comparison(
            &base_trace, &candidate_trace,
            &base_execution, &candidate_execution,
        )?)),
        ("base_witness_line_hits", Value::array(witness_hits(
            prior, &witnesses, &base_trace,
        )?)),
        ("trust", Value::string("BEST_EFFORT_TRUSTED_LOCAL_CPYTHON_UNATTESTED")),
        ("semantic_verification", Value::string("NOT_ESTABLISHED")),
        ("ready_to_merge", Value::Bool(false)),
    ]).stamp_id("report_id")?;
    let payload = obj(&[
        ("plan_event_id", Value::string(request.plan_event_id)),
        ("report", report),
    ]);
    validate_event(repo, base, tree, &payload, prior)?;
    investigation::append(db, repo, request.id, request.expected_revision,
        "TRACE", payload, None)
}
