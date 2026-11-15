//! Trusted-local rust-analyzer SCIP execution with Git-authenticated observations.
//! Rust semantics stay with rust-analyzer; SCIP interpretation stays with scip_refactor.
use crate::json::{self, Value};
use crate::{Result, git, refuse, scip_refactor};
use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(240);
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        for _ in 0..8 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let directory = env::temp_dir().join(format!(
                "excession-rust-scip-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => return Ok(Self(directory)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("create private SCIP directory: {e}")),
            }
        }
        refuse("cannot create private native analysis directory")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Run an explicitly selected native executable without a shell. The output
/// budget applies while the child is running, not merely after its exit.
fn run_bounded(mut command: Command, label: &str, artifact: &Path) -> Result<()> {
    command.stdin(Stdio::null()).stderr(Stdio::null());
    let mut process = command.spawn().map_err(|e| format!("start {label}: {e}"))?;
    let deadline = Instant::now() + DEADLINE;
    loop {
        if artifact.metadata().is_ok_and(|m| m.len() > MAX_INDEX_BYTES) {
            let _ = process.kill();
            let _ = process.wait();
            return refuse(format!("{label} output exceeds 32 MiB"));
        }
        match process
            .try_wait()
            .map_err(|e| format!("wait for {label}: {e}"))?
        {
            Some(status) if status.success() => break,
            Some(status) => return refuse(format!("{label} failed ({status})")),
            None if Instant::now() >= deadline => {
                let _ = process.kill();
                let _ = process.wait();
                return refuse(format!("{label} exceeded 240-second deadline"));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let size = fs::metadata(artifact)
        .map_err(|e| format!("{label} did not produce its index: {e}"))?
        .len();
    if size == 0 || size > MAX_INDEX_BYTES {
        return refuse(format!("{label} output is empty or exceeds 32 MiB"));
    }
    Ok(())
}

/// Only the selected clean HEAD is eligible for local native execution.
/// Historical revisions require an independently materialized clean worktree.
/// No automatic builds, indexing, downloads, or subprocesses occur in ordinary
/// 'excession analyze'; this command is intentionally explicit and trusted.
pub fn scan(
    repo: &Path,
    revision: &str,
    source_prefix: Option<&str>,
    analyzer: &str,
    scip: &str,
    trusted_local: &str,
) -> Result<Value> {
    if trusted_local != "true" {
        return refuse("Rust SCIP execution requires --trusted-local true");
    }
    let root = repo
        .canonicalize()
        .map_err(|e| format!("resolve Git checkout: {e}"))?;
    git::verify_repo(&root)?;
    git::clean(&root)?;
    let commit = git::rev(&root, revision)?;
    if git::text(&root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("Rust SCIP analysis requires a commit revision");
    }
    if git::rev(&root, "HEAD")? != commit {
        return refuse("Rust SCIP analysis requires selected revision at clean HEAD");
    }
    let tracked = git::entries(&root, &commit)?;
    let prefix = match source_prefix {
        Some(prefix) => {
            git::validate_path(prefix)?;
            Some(prefix.to_owned())
        }
        None if tracked.iter().any(|entry| {
            entry.path == "Cargo.toml" && entry.kind == "blob" && entry.mode == "100644"
        }) => None,
        None => {
            let mut candidates = tracked
                .iter()
                .filter(|entry| entry.kind == "blob" && entry.mode == "100644")
                .filter_map(|entry| entry.path.strip_suffix("/Cargo.toml"))
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            candidates.dedup();
            match candidates.as_slice() {
                [only] => Some((*only).to_owned()),
                [] => return refuse("No tracked Cargo.toml. Run 'excession discover .' to inspect source roots."),
                _ => return refuse("Multiple Cargo projects found; choose one with --source-prefix."),
            }
        }
    };
    let manifest = prefix.as_deref().map_or_else(
        || "Cargo.toml".to_owned(),
        |prefix| format!("{prefix}/Cargo.toml"),
    );
    if !tracked
        .iter()
        .any(|entry| entry.path == manifest && entry.kind == "blob" && entry.mode == "100644")
    {
        return refuse("selected Rust workspace requires a tracked Cargo.toml");
    }
    let workspace = prefix.as_deref().map_or_else(|| root.clone(), |prefix| root.join(prefix));
    let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
    if !workspace.starts_with(&root) || !workspace.is_dir() {
        return refuse("Rust workspace resolves outside the Git checkout");
    }

    let scratch = Scratch::new()?;
    let index = scratch.0.join("index.scip");
    let decoded_path = scratch.0.join("index.scip.json");
    let mut native = Command::new(analyzer);
    native
        .arg("scip")
        .arg(&workspace)
        .arg("--output")
        .arg(&index)
        .current_dir(&workspace)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", scratch.0.join("cargo-target"))
        .stdout(Stdio::null());
    run_bounded(native, "rust-analyzer scip", &index)?;

    let decoded_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&decoded_path)
        .map_err(|e| format!("create private decoded index: {e}"))?;
    let mut decoder = Command::new(scip);
    decoder
        .args(["print", "--json"])
        .arg(&index)
        .current_dir(&workspace)
        .stdout(Stdio::from(decoded_file));
    run_bounded(decoder, "scip print", &decoded_path)?;
    git::clean(&root)?;
    if git::rev(&root, "HEAD")? != commit {
        return refuse("Git HEAD changed while rust-analyzer was running");
    }

    let bytes = fs::read(&decoded_path).map_err(|e| format!("read SCIP JSON: {e}"))?;
    let parsed = json::parse(&bytes)?;
    let metadata = parsed.get("metadata")?;
    let project_root = metadata
        .get("project_root")
        .or_else(|_| metadata.get("projectRoot"))?
        .str()?;
    let expected = scip_refactor::scip_project_root_uri(&workspace)?;
    if project_root != expected {
        return refuse("native SCIP project_root differs from the selected Cargo workspace");
    }
    let report = scip_refactor::evidence_from_bytes(&root, &commit, &bytes, prefix.as_deref())?;
    if report.get("indexer")?.get("name")?.str()? != "rust-analyzer" {
        return refuse("Rust native analysis requires a rust-analyzer SCIP producer");
    }
    Ok(report)
}

/// A concise view; --json exposes the complete Git-bound native report.
pub fn human(report: &Value) -> Result<String> {
    let coverage = report.get("coverage")?;
    Ok(format!(
        "Rust SCIP analysis (rust-analyzer)\nRevision: {}\nIndexed files: {}\nCross-file symbol references: {}\nUnresolved occurrences: {}\nCoverage: PARTIAL (references are not necessarily imports or calls)\nUse --json for source-bound evidence.",
        report.get("source_commit")?.str()?,
        coverage.get("indexed_documents")?.integer()?,
        coverage.get("matched_cross_file_references")?.integer()?,
        coverage.get("unresolved_references")?.integer()?,
    ))
}
