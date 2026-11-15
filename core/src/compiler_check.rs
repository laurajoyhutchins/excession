//! Explicit, bounded native compiler/typechecker execution receipts.
//! These observations authenticate a Git configuration and a local process
//! outcome. They do not establish artifact provenance, complete source
//! membership, remote attestation, or universal behavioral correctness.
use crate::adapter;
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "excession.compiler-check.v1";
const MAX_BINARY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_REPORT_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_CHARS: usize = 2048;
const CHECK_TIMEOUT: Duration = Duration::from_secs(180);
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        for _ in 0..16 {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?.as_nanos();
            let path = env::temp_dir().join(format!(
                "excession-compiler-check-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return refuse(format!("create private compiler scratch: {e}")),
            }
        }
        refuse("cannot allocate private compiler scratch")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub language: &'a str,
    pub configuration: &'a str,
    pub source_prefix: Option<&'a str>,
    pub executable: Option<&'a str>,
    pub trusted_local: bool,
}

fn spec(language: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match language {
        "rust" => Ok(("cargo", "Cargo.toml", "CARGO_CHECK_FROZEN_ALL_TARGETS")),
        "typescript" => Ok(("tsc", "tsconfig.json", "TSC_NO_EMIT_SINGLE_PROJECT")),
        "python" => Ok(("mypy", "mypy.ini", "MYPY_CONFIGURED_SOURCE_SCOPE")),
        _ => refuse("compiler check supports only rust, typescript, python"),
    }
}

fn source_doc(source: &SourceRef) -> Value {
    obj(&[
        ("source_commit", Value::string(&source.commit)),
        ("source_tree", Value::string(&source.tree)),
        ("path", Value::string(&source.path)),
        ("blob", Value::string(&source.blob)),
        ("mode", Value::string(&source.mode)),
    ])
}

fn tool_path(name: &str, specified: Option<&str>) -> Result<PathBuf> {
    let candidate = match specified {
        Some(path) => {
            let tool = PathBuf::from(path);
            if !tool.is_absolute() {
                return refuse("explicit compiler executable must be an absolute path");
            }
            tool
        }
        None => {
            let paths = env::var_os("PATH").ok_or("PATH unavailable for compiler")?;
            env::split_paths(&paths)
                .map(|dir| dir.join(name))
                .find(|path| path.is_file())
                .ok_or_else(|| format!("{name} missing: install a native checker or pass --tool"))?
        }
    };
    let resolved = candidate.canonicalize()
        .map_err(|e| format!("resolve compiler executable: {e}"))?;
    let metadata = fs::metadata(&resolved).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BINARY_BYTES {
        return refuse("compiler launcher is not a regular file within 64 MiB");
    }
    Ok(resolved)
}

fn toolchain(tool: &Path, name: &str) -> Result<Value> {
    let data = fs::read(tool).map_err(|e| format!("read compiler launcher: {e}"))?;
    if data.is_empty() || data.len() as u64 > MAX_BINARY_BYTES {
        return refuse("compiler launcher changed or exceeds 64 MiB");
    }
    let mut version = Command::new(tool);
    version.arg("--version");
    let response = adapter::run_tool_status(version, "compiler version", Duration::from_secs(10))?;
    if response.exit_code != 0 {
        return refuse("compiler launcher did not provide a successful --version");
    }
    let version = String::from_utf8(response.stdout)
        .map_err(|_| "compiler version is not UTF-8".to_owned())?;
    let version = version.trim();
    if version.is_empty() || version.len() > 300 || version.contains('\0') {
        return refuse("invalid or missing compiler version");
    }
    Ok(obj(&[
        ("name", Value::string(name)),
        ("executable", Value::string(tool.to_string_lossy())),
        ("launcher_sha256", Value::string(digest::sha256(&data))),
        ("version", Value::string(version)),
        ("identity_scope", Value::string("LAUNCHER_AND_VERSION_NOT_TRANSITIVE_TOOLCHAIN")),
    ]))
}

fn validate_configuration(root: &Path, commit: &str, path: &str) -> Result<(SourceRef, PathBuf)> {
    git::validate_path(path)?;
    let source = SourceRef::from_git(root, commit, path)?;
    let physical = root.join(path).canonicalize()
        .map_err(|e| format!("compiler config missing in checkout: {e}"))?;
    if !physical.starts_with(root) || !physical.is_file() {
        return refuse("compiler config resolves outside the selected Git checkout");
    }
    let bytes = fs::read(&physical).map_err(|e| format!("read compiler config: {e}"))?;
    if bytes.len() > MAX_CONFIG_BYTES || digest::git_blob(&bytes) != source.blob {
        return refuse("compiler config bytes differ from the pinned Git blob or exceed 1 MiB");
    }
    Ok((source, physical))
}

fn scope(root: &Path, language: &str, selected: Option<&str>) -> Result<(String, PathBuf)> {
    if language != "python" && selected.is_some() {
        return refuse("--source-prefix applies only to the Python compiler-check scope");
    }
    let prefix = selected.unwrap_or(".");
    if prefix != "." {
        git::validate_path(prefix)?;
    }
    let path = if prefix == "." {root.to_owned()} else {root.join(prefix)};
    let canonical = path.canonicalize().map_err(|e| format!("source scope missing: {e}"))?;
    if !canonical.starts_with(root) {
        return refuse("source scope resolves outside the Git checkout");
    }
    if !canonical.is_dir() && !canonical.is_file() {
        return refuse("source scope is neither a directory nor a source file");
    }
    Ok((prefix.to_owned(), canonical))
}

fn preview(bytes: &[u8]) -> Value {
    let display = String::from_utf8_lossy(bytes);
    let part = display.chars().take(MAX_PREVIEW_CHARS).collect::<String>();
    Value::string(part)
}

fn report_from_execution(
    source: &SourceRef,
    language: &str,
    profile: &str,
    prefix: &str,
    tool: Value,
    output: &adapter::ToolOutput,
) -> Result<Value> {
    let status = if output.exit_code == 0 { "PASS" } else { "FAIL" };
    obj(&[
        ("schema", Value::string(SCHEMA)),
        ("status", Value::string(status)),
        ("language", Value::string(language)),
        ("profile", Value::string(profile)),
        ("source_commit", Value::string(&source.commit)),
        ("source_tree", Value::string(&source.tree)),
        ("configuration", source_doc(source)),
        ("source_scope", Value::string(prefix)),
        ("toolchain", tool),
        ("execution", obj(&[
            ("exit_code", Value::number(output.exit_code)),
            ("stdout_sha256", Value::string(digest::sha256(&output.stdout))),
            ("stderr_sha256", Value::string(digest::sha256(&output.stderr))),
            ("stdout_bytes", Value::number(output.stdout.len())),
            ("stderr_bytes", Value::number(output.stderr.len())),
            ("stdout_preview", preview(&output.stdout)),
            ("stderr_preview", preview(&output.stderr)),
            ("output_preview_truncated", Value::Bool(
                output.stdout.len() > MAX_PREVIEW_CHARS || output.stderr.len() > MAX_PREVIEW_CHARS
            )),
        ])),
        ("coverage", obj(&[
            ("configuration", Value::string("ONE_SELECTED_GIT_CONFIGURATION")),
            ("source_membership", Value::string("COMPILER_SELECTED_UNAUTHENTICATED")),
            ("dependency_closure", Value::string("NOT_ESTABLISHED")),
            ("other_configurations", Value::string("NOT_CHECKED")),
        ])),
        ("execution_authority", Value::string("TRUSTED_LOCAL_NOT_ATTESTED")),
        ("artifact_build_provenance", Value::string("NOT_ESTABLISHED")),
        ("rea", Value::string("NOT_ESTABLISHED")),
    ]).stamp_id("report_id")
}

fn validate_report(repo: &Path, revision: &str, report: &Value) -> Result<SourceRef> {
    if report.get("schema")?.str()? != SCHEMA
        || !report.content_id_matches("report_id")?
        || report.get("execution_authority")?.str()? != "TRUSTED_LOCAL_NOT_ATTESTED"
        || report.get("artifact_build_provenance")?.str()? != "NOT_ESTABLISHED"
        || report.get("rea")?.str()? != "NOT_ESTABLISHED"
    {
        return refuse("compiler check report schema, trust scope or digest mismatch");
    }
    let language = report.get("language")?.str()?;
    let (_, _, profile) = spec(language)?;
    if report.get("profile")?.str()? != profile {
        return refuse("compiler check language/profile mismatch");
    }
    let status = report.get("status")?.str()?;
    let exit = report.get("execution")?.get("exit_code")?.integer()?;
    if !matches!((status, exit), ("PASS", 0) | ("FAIL", 1..=i64::MAX)) {
        return refuse("compiler check status and process exit code disagree");
    }
    let commit = git::rev(repo, revision)?;
    let source = SourceRef::from_git(repo, &commit, report.get("configuration")?.get("path")?.str()?)?;
    if report.get("source_commit")?.str()? != commit
        || report.get("source_tree")?.str()? != source.tree
        || report.get("configuration")? != &source_doc(&source)
    {
        return refuse("compiler check configuration differs from pinned Git source");
    }
    let tool = report.get("toolchain")?;
    if tool.get("name")?.str()? != spec(language)?.0
        || tool.get("launcher_sha256")?.str()?.len() != 64
        || tool.get("identity_scope")?.str()? != "LAUNCHER_AND_VERSION_NOT_TRANSITIVE_TOOLCHAIN"
    {
        return refuse("compiler check toolchain identity invalid");
    }
    if report.get("coverage")?.get("dependency_closure")?.str()? != "NOT_ESTABLISHED"
        || report.get("coverage")?.get("other_configurations")?.str()? != "NOT_CHECKED"
    {
        return refuse("compiler check cannot claim unobserved configuration coverage");
    }
    if language != "python" && report.get("source_scope")?.str()? != "." {
        return refuse("unexpected non-Python compiler source scope");
    }
    if language == "python" {
        let path = report.get("source_scope")?.str()?;
        if path != "." {
            git::validate_path(path)?;
        }
    }
    Ok(source)
}

fn batch(repo: &Path, revision: &str, report: &Value) -> Result<Batch> {
    let source = validate_report(repo, revision, report)?;
    let raw_json = report.canonical();
    if raw_json.len() > MAX_REPORT_BYTES {
        return refuse("compiler check receipt exceeds 64 KiB");
    }
    let receipt = Receipt {
        producer: format!("compiler-check:{}", report.get("language")?.str()?),
        version: "1".to_owned(),
        native_id: report.get("report_id")?.str()?.to_owned(),
        raw_json,
    };
    let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
    Ok(Batch {
        source_commit: source.commit.clone(),
        receipts: vec![receipt],
        observations: vec![Observation {
            receipt_id,
            subject_id: Subject::Source(source.clone()).id()?,
            native_kind: "CONFIGURED_COMPILER_EXECUTION".to_owned(),
            payload: obj(&[
                ("source", source_doc(&source)),
                ("status", report.get("status")?.clone()),
                ("profile", report.get("profile")?.clone()),
                ("toolchain", report.get("toolchain")?.clone()),
                ("execution", report.get("execution")?.clone()),
                ("coverage", report.get("coverage")?.clone()),
                ("execution_authority", report.get("execution_authority")?.clone()),
                ("artifact_build_provenance", report.get("artifact_build_provenance")?.clone()),
                ("rea", report.get("rea")?.clone()),
            ]),
        }],
    })
}

fn with_snapshot(mut report: Value, snapshot: &str) -> Result<Value> {
    report.entries_mut()?.insert("snapshot_id".into(), Value::string(snapshot));
    Ok(report)
}

/// Run one configured local check, persist both PASS and FAIL, and replay
/// pinned Git configuration before a status reaches the caller.
pub fn run(db: &Db, request: Request<'_>) -> Result<Value> {
    if !request.trusted_local {
        return refuse("compiler checks require --trusted-local true; no sandbox is provided");
    }
    let (default_tool, expected_name, profile) = spec(request.language)?;
    if !request.configuration.ends_with(expected_name) {
        return refuse(format!("{} requires a {expected_name} configuration", request.language));
    }
    let root = request.repo.canonicalize()
        .map_err(|e| format!("resolve compiler checkout: {e}"))?;
    git::verify_repo(&root)?;
    git::clean(&root)?;
    let commit = git::rev(&root, request.revision)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("compiler check requires the selected clean HEAD");
    }
    let (configuration, physical) = validate_configuration(&root, &commit, request.configuration)?;
    let (prefix, physical_scope) = scope(&root, request.language, request.source_prefix)?;
    let tool = tool_path(default_tool, request.executable)?;
    let metadata = toolchain(&tool, default_tool)?;
    let scratch = Scratch::new()?;
    let mut command = Command::new(&tool);
    command.current_dir(&root);
    match request.language {
        "rust" => {
            command.arg("check")
                .arg("--manifest-path").arg(&physical)
                .arg("--all-targets").arg("--frozen")
                .env("CARGO_TARGET_DIR", scratch.0.join("cargo-target"))
                .env("CARGO_NET_OFFLINE", "true")
                .env("CARGO_INCREMENTAL", "0");
        }
        "typescript" => {
            command.arg("--project").arg(&physical)
                .arg("--noEmit").arg("--pretty").arg("false")
                .arg("--incremental").arg("--tsBuildInfoFile")
                .arg(scratch.0.join("tsbuildinfo"));
        }
        "python" => {
            command.arg("--config-file").arg(&physical)
                .arg("--no-incremental")
                .arg("--cache-dir").arg(scratch.0.join("mypy-cache"))
                .arg(&physical_scope);
        }
        _ => unreachable!("validated language"),
    }
    let result = adapter::run_tool_status(command, "configured compiler", CHECK_TIMEOUT)?;
    git::clean(&root)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("compiler check changed Git HEAD during execution");
    }
    let (after, _) = validate_configuration(&root, &commit, request.configuration)?;
    if after != configuration {
        return refuse("compiler check configuration changed during execution");
    }
    let report = report_from_execution(
        &configuration, request.language, profile, &prefix, metadata, &result,
    )?;
    let batch = batch(&root, &commit, &report)?;
    let snapshot = evidence_store::persist(db, &batch)?;
    with_snapshot(report, &snapshot)
}

/// Audit immutable original bytes and Git configuration without rerunning
/// compiler code. This does not authenticate the machine that ran the tool.
pub fn audit(db: &Db, repo: &Path, revision: &str, snapshot: &str) -> Result<Value> {
    let stored = evidence_store::audit(db, snapshot)?;
    let receipts = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot)],
    )?;
    if receipts.len() != 1 {
        return refuse("compiler check snapshot requires one original receipt");
    }
    let receipt = json::parse(
        receipts[0][0].as_deref().ok_or("missing compiler check receipt")?.as_bytes()
    )?;
    if receipt.get("version")?.str()? != "1" {
        return refuse("unsupported compiler check receipt version");
    }
    let report = json::parse(receipt.get("raw_json")?.str()?.as_bytes())?;
    if receipt.get("producer")?.str()? != format!("compiler-check:{}", report.get("language")?.str()?)
        || receipt.get("native_id")?.str()? != report.get("report_id")?.str()?
        || report.canonical() != receipt.get("raw_json")?.str()?
    {
        return refuse("compiler check producer or raw receipt mismatch");
    }
    let prepared = batch(repo, revision, &report)?;
    if evidence_store::manifest(&prepared)? != stored {
        return refuse("compiler check stored evidence disagrees with Git replay");
    }
    with_snapshot(report, snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_refuses_unknown_compiler_and_bad_tool_overrides() {
        assert!(spec("java").is_err());
        assert!(spec("typescript").is_ok());
        assert!(tool_path("cargo", Some("relative/compiler")).is_err());
    }

    #[test]
    fn statuses_never_promote_failed_or_missing_builds() {
        let config = SourceRef {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
            path: "Cargo.toml".to_owned(),
            blob: "c".repeat(40),
            mode: "100644".to_owned(),
        };
        let sample = adapter::ToolOutput { stdout: vec![], stderr: b"compile error".to_vec(), exit_code: 1 };
        let report = report_from_execution(
            &config, "rust", "CARGO_CHECK_FROZEN_ALL_TARGETS", ".",
            obj(&[("name", Value::string("cargo"))]), &sample,
        ).unwrap();
        assert_eq!(report.get("status").unwrap().str().unwrap(), "FAIL");
        assert_eq!(report.get("artifact_build_provenance").unwrap().str().unwrap(), "NOT_ESTABLISHED");
        assert!(report.content_id_matches("report_id").unwrap());
    }
}
