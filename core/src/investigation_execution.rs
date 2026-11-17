//! Explicit trusted-local differential executions for witness-backed plans.
//! Two detached Git worktrees are run with an identical host executable and
//! argument vector. Only exit status and output digests are compared. There
//! is NO sandbox, execution attestation, or universal semantic equivalence.
use crate::json::{Value, obj};
use crate::sqlite::Db;
use crate::{Result, git, investigation, refuse};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FORMAT: &str = "excession.investigation-execution.v1";
const OUTPUT_POLL: Duration = Duration::from_millis(10);

pub struct Request<'a> {
    pub id: &'a str,
    pub expected_revision: usize,
    pub plan_event_id: &'a str,
    pub candidate: &'a str,
    pub program: &'a Path,
    pub argv_json: &'a str,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    pub trusted_local: bool,
}

fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut f = File::open(path).map_err(|e| format!("execution artifact read: {e}"))?;
    let mut sha = Sha256::new();
    let mut bytes = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("execution artifact stream: {e}"))?;
        if n == 0 { break; }
        bytes = bytes.checked_add(n as u64).ok_or("execution artifact size overflow")?;
        sha.update(&buf[..n]);
    }
    Ok((format!("{:x}", sha.finalize()), bytes))
}

fn argv(raw: &str) -> Result<Vec<String>> {
    if raw.len() > 64 * 1024 {
        return refuse("execution argument JSON exceeds 64 KiB");
    }
    let args: Vec<String> = serde_json::from_str(raw)
        .map_err(|e| format!("--argv-json requires an array of strings: {e}"))?;
    if args.len() > 128 || args.iter().any(|x| x.len() > 8192 || x.contains('\0')) {
        return refuse("execution argument count or length exceeds protocol budget");
    }
    Ok(args)
}

fn verified_revision(repo: &Path, commit: &str) -> Result<String> {
    git::oid(commit)?;
    if git::rev(repo, &format!("{commit}^{{commit}}"))? != commit {
        return refuse("execution commit is not a direct Git commit identity");
    }
    git::rev(repo, &format!("{commit}^{{tree}}"))
}

fn planned_witnesses(events: &[Value], plan_id: &str) -> Result<Vec<Value>> {
    let mut matching = None;
    let mut witnesses = Vec::new();
    for event in events {
        let id = event.get("event_id")?.str()?;
        let kind = event.get("kind")?.str()?;
        if id == plan_id {
            if kind != "PLAN" { return refuse("execution references a non-plan event"); }
            let claim = event.get("payload")?.get("claim_event_id")?.str()?;
            witnesses.retain(|item: &Value| {
                item.get("claim_event_id").and_then(Value::str).ok() == Some(claim)
            });
            if witnesses.is_empty() {
                return refuse("execution plan needs an existing source behavior witness");
            }
            matching = Some(witnesses.iter().map(|x| x.get("event_id").cloned()).collect::<Result<Vec<_>>>()?);
            break;
        }
        if kind == "WITNESS" {
            witnesses.push(obj(&[
                ("event_id", event.get("event_id")?.clone()),
                ("claim_event_id", event.get("payload")?.get("claim_event_id")?.clone()),
            ]));
        }
    }
    matching.ok_or_else(|| "execution requires an earlier witness-backed plan".to_owned())
}

struct Worktrees<'a> {
    repo: &'a Path,
    root: PathBuf,
    registered: Vec<PathBuf>,
}

impl<'a> Worktrees<'a> {
    fn new(repo: &'a Path) -> Result<Self> {
        let canonical = repo.canonicalize().map_err(|e| format!("source checkout: {e}"))?;
        for nonce in 0..32u32 {
            let tick = SystemTime::now().duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?.as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "excession-execution-{}-{tick}-{nonce}", std::process::id()
            ));
            if dir.starts_with(&canonical) {
                return refuse("temporary execution worktrees must be outside the source checkout");
            }
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(Self { repo, root: dir, registered: Vec::new() }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(format!("create temporary execution workspace: {e}")),
            }
        }
        refuse("cannot allocate unique temporary execution workspace")
    }

    fn add(&mut self, label: &str, commit: &str) -> Result<PathBuf> {
        let path = self.root.join(label);
        let string = path.to_str().ok_or("non-UTF8 temporary execution path")?;
        git::command(self.repo, &["worktree", "add", "--detach", string, commit])?;
        self.registered.push(path.clone());
        if git::rev(&path, "HEAD")? != commit {
            return refuse("detached execution worktree has incorrect HEAD");
        }
        git::clean(&path)?;
        Ok(path)
    }

    fn cleanup(mut self) -> Result<()> {
        for path in self.registered.iter().rev() {
            let string = path.to_str().ok_or("non-UTF8 temporary execution path")?;
            git::command(self.repo, &["worktree", "remove", "--force", string])?;
        }
        self.registered.clear();
        fs::remove_dir_all(&self.root)
            .map_err(|e| format!("remove temporary execution workspace: {e}"))?;
        Ok(())
    }
}

impl Drop for Worktrees<'_> {
    fn drop(&mut self) {
        for path in self.registered.iter().rev() {
            if let Some(s) = path.to_str() {
                let _ = git::command(self.repo, &["worktree", "remove", "--force", s]);
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn observation(
    worktree: &Path,
    revision: &str,
    tree: &str,
    program: &Path,
    args: &[String],
    timeout: Duration,
    max_output: u64,
) -> Result<Value> {
    let stdout = worktree.join(".excession-observed-stdout");
    let stderr = worktree.join(".excession-observed-stderr");
    let output = (|| -> Result<Value> {
        let out = File::create(&stdout).map_err(|e| format!("execution stdout capture: {e}"))?;
        let err = File::create(&stderr).map_err(|e| format!("execution stderr capture: {e}"))?;
        let mut child = Command::new(program)
            .args(args)
            .current_dir(worktree)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("trusted-local execution launch: {e}"))?;
        let started = Instant::now();
        let (state, code) = loop {
            if let Some(status) = child.try_wait().map_err(|e| format!("execution wait: {e}"))? {
                break ("EXITED", status.code());
            }
            let a = fs::metadata(&stdout).map_err(|e| e.to_string())?.len();
            let b = fs::metadata(&stderr).map_err(|e| e.to_string())?.len();
            if a > max_output || b > max_output {
                let _ = child.kill();
                let _ = child.wait();
                break ("OUTPUT_LIMIT", None);
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                break ("TIMEOUT", None);
            }
            thread::sleep(OUTPUT_POLL);
        };
        let a = fs::metadata(&stdout).map_err(|e| e.to_string())?.len();
        let b = fs::metadata(&stderr).map_err(|e| e.to_string())?.len();
        let state = if a > max_output || b > max_output {
            "OUTPUT_LIMIT"
        } else if state == "EXITED" && code.is_none() {
            "TERMINATED_BY_SIGNAL"
        } else {
            state
        };
        let stdout_hash = if a <= max_output {
            Value::string(sha256_file(&stdout)?.0)
        } else { Value::Null };
        let stderr_hash = if b <= max_output {
            Value::string(sha256_file(&stderr)?.0)
        } else { Value::Null };
        // These are observations of one bounded run, not semantic guarantees.
        Ok(obj(&[
            ("commit", Value::string(revision)),
            ("tree", Value::string(tree)),
            ("status", Value::string(state)),
            ("exit_code", code.map(Value::number).unwrap_or(Value::Null)),
            ("stdout_sha256", stdout_hash),
            ("stderr_sha256", stderr_hash),
            ("stdout_bytes", Value::number(a)),
            ("stderr_bytes", Value::number(b)),
        ]))
    })();
    let _ = fs::remove_file(stdout);
    let _ = fs::remove_file(stderr);
    output
}

fn comparison(base: &Value, candidate: &Value) -> Result<&'static str> {
    if base.get("status")?.str()? != "EXITED" || candidate.get("status")?.str()? != "EXITED" {
        return Ok("INCONCLUSIVE");
    }
    for field in ["exit_code", "stdout_sha256", "stderr_sha256", "stdout_bytes", "stderr_bytes"] {
        if base.get(field)? != candidate.get(field)? {
            return Ok("DIFFERENCE_OBSERVED");
        }
    }
    Ok("AGREEMENT_OBSERVED")
}

pub(crate) fn validate_event(
    repo: &Path,
    commit: &str,
    tree: &str,
    payload: &Value,
    prior: &[Value],
) -> Result<()> {
    if payload.entries()?.len() != 2 {
        return refuse("execution event requires plan identity and captured report");
    }
    let plan_id = payload.get("plan_event_id")?.str()?;
    let witness_ids = planned_witnesses(prior, plan_id)?;
    let report = payload.get("report")?;
    if report.entries()?.len() != 17
        || report.get("schema")?.str()? != FORMAT
        || report.get("base_commit")?.str()? != commit
        || report.get("base_tree")?.str()? != tree
        || report.get("plan_event_id")?.str()? != plan_id
        || report.get("witness_event_ids")?.array_ref()? != witness_ids.as_slice()
        || report.get("trust")?.str()? != "TRUSTED_LOCAL_EXECUTION_UNATTESTED"
        || report.get("semantic_verification")?.str()? != "NOT_ESTABLISHED"
        || report.get("ready_to_merge")?.bool()?
        || !report.content_id_matches("report_id")?
    {
        return refuse("execution report provenance or safety contract differs");
    }
    let candidate = report.get("candidate_commit")?.str()?;
    if candidate == commit
        || verified_revision(repo, candidate)? != report.get("candidate_tree")?.str()?
    {
        return refuse("execution candidate revision identity mismatch");
    }
    let tool = report.get("program")?;
    if tool.entries()?.len() != 3
        || !Path::new(tool.get("path")?.str()?).is_absolute()
        || tool.get("sha256")?.str()?.len() != 64
        || tool.get("argv")?.array_ref()?.iter().any(|v| v.str().is_err())
    {
        return refuse("execution host tool identity or argv is malformed");
    }
    if report.get("environment")?.str()? != "FIXED_MINIMAL_PROCESS_ENV_HOST_UNATTESTED" {
        return refuse("execution environment must remain explicitly unattested");
    }
    let limits = report.get("limits")?;
    if limits.entries()?.len() != 2
        || limits.get("timeout_ms")?.integer()? <= 0
        || limits.get("max_output_bytes")?.integer()? <= 0
    {
        return refuse("execution resource limits are malformed");
    }
    for (key, rev, git_tree) in [
        ("base", commit, tree),
        ("candidate", candidate, report.get("candidate_tree")?.str()?),
    ] {
        let sample = report.get(key)?;
        if sample.entries()?.len() != 8
            || sample.get("commit")?.str()? != rev
            || sample.get("tree")?.str()? != git_tree
            || !matches!(sample.get("status")?.str()?,
                "EXITED" | "TIMEOUT" | "OUTPUT_LIMIT" | "TERMINATED_BY_SIGNAL")
        {
            return refuse("execution sample identity or status mismatch");
        }
        for key in ["stdout_bytes", "stderr_bytes"] {
            if sample.get(key)?.integer()? < 0 {
                return refuse("negative execution output byte count");
            }
        }
        if sample.get("status")?.str()? == "EXITED" {
            if sample.get("exit_code")?.integer()? < 0 {
                return refuse("execution exit code must be nonnegative");
            }
            for field in ["stdout_sha256", "stderr_sha256"] {
                if sample.get(field)?.str()?.len() != 64 {
                    return refuse("execution output digest malformed");
                }
            }
        }
    }
    if report.get("comparison")?.str()? != comparison(report.get("base")?, report.get("candidate")?)? {
        return refuse("execution comparison contradicts retained observations");
    }
    Ok(())
}

pub fn compare(db: &Db, repo: &Path, request: Request<'_>) -> Result<Value> {
    if !request.trusted_local {
        return refuse("execution requires explicit --trusted-local true; no sandbox is provided");
    }
    if request.timeout_ms == 0 || request.max_output_bytes == 0
        || request.timeout_ms > i64::MAX as u64
        || request.max_output_bytes > i64::MAX as u64
    {
        return refuse("execution timeout and output budgets must be positive representable values");
    }
    let arguments = argv(request.argv_json)?;
    if !request.program.is_absolute() {
        return refuse("execution requires an absolute trusted host executable path");
    }
    let program = request.program.canonicalize()
        .map_err(|e| format!("trusted host executable: {e}"))?;
    if !program.is_file() {
        return refuse("execution requires a regular absolute host executable");
    }
    let source_root = repo.canonicalize().map_err(|e| format!("source checkout: {e}"))?;
    if program.starts_with(&source_root)
        || arguments.iter().any(|arg| Path::new(arg).is_absolute()
            && Path::new(arg).starts_with(&source_root))
    {
        return refuse("host executable and absolute script paths must not refer to the mutable source checkout");
    }
    let (program_sha, _) = sha256_file(&program)?;
    let history = investigation::audit(db, repo, request.id)?;
    if history.get("revision")?.integer()? != request.expected_revision as i64 {
        return refuse("stale investigation expected revision");
    }
    let base = history.get("source_commit")?.str()?;
    let tree = history.get("source_tree")?.str()?;
    let witness_ids = planned_witnesses(history.get("events")?.array_ref()?, request.plan_event_id)?;
    let candidate = git::rev(repo, &format!("{}^{{commit}}", request.candidate))?;
    let candidate_tree = verified_revision(repo, &candidate)?;
    if base == candidate {
        return refuse("differential execution requires a distinct candidate commit");
    }

    let mut temp = Worktrees::new(repo)?;
    let base_root = temp.add("base", base)?;
    let candidate_root = temp.add("candidate", &candidate)?;
    let timeout = Duration::from_millis(request.timeout_ms);
    let old = observation(&base_root, base, tree, &program, &arguments, timeout, request.max_output_bytes)?;
    let new = observation(&candidate_root, &candidate, &candidate_tree, &program, &arguments, timeout, request.max_output_bytes)?;
    temp.cleanup()?;

    let report = obj(&[
        ("schema", Value::string(FORMAT)),
        ("base_commit", Value::string(base)),
        ("base_tree", Value::string(tree)),
        ("candidate_commit", Value::string(candidate)),
        ("candidate_tree", Value::string(candidate_tree)),
        ("plan_event_id", Value::string(request.plan_event_id)),
        ("witness_event_ids", Value::array(witness_ids)),
        ("program", obj(&[
            ("path", Value::string(program.to_string_lossy())),
            ("sha256", Value::string(program_sha)),
            ("argv", Value::array(arguments.into_iter().map(Value::string))),
        ])),
        ("limits", obj(&[
            ("timeout_ms", Value::number(request.timeout_ms)),
            ("max_output_bytes", Value::number(request.max_output_bytes)),
        ])),
        ("environment", Value::string("FIXED_MINIMAL_PROCESS_ENV_HOST_UNATTESTED")),
        ("base", old.clone()),
        ("candidate", new.clone()),
        ("comparison", Value::string(comparison(&old, &new)?)),
        ("trust", Value::string("TRUSTED_LOCAL_EXECUTION_UNATTESTED")),
        ("semantic_verification", Value::string("NOT_ESTABLISHED")),
        ("ready_to_merge", Value::Bool(false)),
    ]).stamp_id("report_id")?;
    validate_event(repo, base, tree, &obj(&[
        ("plan_event_id", Value::string(request.plan_event_id)),
        ("report", report.clone()),
    ]), history.get("events")?.array_ref()?)?;
    // Recheck expected revision atomically after the external process exits.
    let result = investigation::append(db, repo, request.id, request.expected_revision,
        "EXECUTION", obj(&[
            ("plan_event_id", Value::string(request.plan_event_id)),
            ("report", report.clone()),
        ]), None)?;
    // The original report remains included inside the append-only event.
    Ok(result)
}
