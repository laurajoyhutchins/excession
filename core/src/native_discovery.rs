//! Git-authoritative staging and input contracts for native language resolvers.
//!
//! A language adapter reports *candidate* source/configuration paths only.
//! Rust authenticates their Git identities and owns revision selection, staging,
//! byte limits, toolchain provenance and the declaration schema. No claims
//! about runtime effect completeness or target behavioral equivalence follow.
//! Native compiler invocation is explicitly trusted-local, not sandboxed.
use crate::digest;
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_STAGE_FILES: usize = 4096;
const MAX_STAGE_BYTES: usize = 96 * 1024 * 1024;
const MAX_INPUT_FILES: usize = 512;
const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PROCESS_TIME: Duration = Duration::from_secs(55);
const RESOLVER: &str = include_str!("../../tools/native_language_adapter.py");
const TYPESCRIPT_HELPER: &str = include_str!("../../tools/excession_ts_resolve.cjs");
static NONCE: AtomicU64 = AtomicU64::new(0);
type PinnedFiles = BTreeMap<String, (usize, String)>;

struct Snapshot {
    directory: PathBuf,
}
impl Snapshot {
    fn new() -> Result<Self> {
        for _ in 0..8 {
            let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let directory = env::temp_dir().join(format!(
                "excession-native-{}-{time}-{nonce}",
                std::process::id()
            ));
            // Private from creation: snapshots can contain repository secrets.
            #[cfg(unix)]
            let created = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                builder.create(&directory)
            };
            #[cfg(not(unix))]
            let created = fs::create_dir(&directory);
            match created {
                Ok(()) => return Ok(Self { directory }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("create native snapshot directory: {e}")),
            }
        }
        refuse("unable to create unique native snapshot directory")
    }
    fn source(&self) -> PathBuf {
        self.directory.join("src")
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn run_bounded(mut command: Command, context: &str) -> Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("{context}: {e}"))?;
    let stdout = child.stdout.take().ok_or("missing stdout")?;
    let stderr = child.stderr.take().ok_or("missing stderr")?;
    let out_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut data = Vec::new();
        stdout.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut data)?;
        Ok(data)
    });
    let err_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut data = Vec::new();
        stderr.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut data)?;
        Ok(data)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("{context}: wait: {e}"))?
        {
            break Ok(status);
        }
        if start.elapsed() > MAX_PROCESS_TIME {
            let _ = child.kill();
            let _ = child.wait();
            break Err(format!("{context}: exceeded 55-second timeout"));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Consume output after process termination; this also closes both pipe handles.
    let out = out_thread
        .join()
        .map_err(|_| format!("{context}: stdout reader panicked"))?
        .map_err(|e| format!("{context}: stdout: {e}"))?;
    let err = err_thread
        .join()
        .map_err(|_| format!("{context}: stderr reader panicked"))?
        .map_err(|e| format!("{context}: stderr: {e}"))?;
    if out.len() as u64 > MAX_OUTPUT_BYTES || err.len() as u64 > MAX_OUTPUT_BYTES {
        return refuse(format!("{context}: exceeded 16 MiB output bound"));
    }
    if !status?.success() {
        return refuse(format!(
            "{context}: {}",
            String::from_utf8_lossy(&err[..err.len().min(700)])
        ));
    }
    Ok(out)
}

fn executable(name: &str) -> Result<PathBuf> {
    let path = env::var_os("PATH").ok_or("native resolver requires PATH")?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            // Preserve argv[0]: rustup dispatches based on the invoked shim
            // name ("rustc"), not its resolved binary name ("rustup").
            // Canonicalizing the path changes compiler behavior.
            return Ok(candidate);
        }
    }
    refuse(format!("native resolver requires installed {name}"))
}
fn toolchain_at(path: &Path, version_flag: &str) -> Result<(String, String)> {
    let mut command = Command::new(path);
    command.arg(version_flag);
    let version = String::from_utf8(run_bounded(command, "toolchain version")?)
        .map_err(|_| "non-UTF8 toolchain version".to_string())?;
    let version = version.trim().chars().take(180).collect::<String>();
    if version.is_empty() {
        return refuse("toolchain version is empty");
    }
    let binary = fs::read(path).map_err(|e| format!("read toolchain: {e}"))?;
    Ok((version, digest::sha256(&binary)))
}
fn toolchain(name: &str, version_flag: &str) -> Result<(String, String)> {
    toolchain_at(&executable(name)?, version_flag)
}

/// Resolve the selected Rust toolchain while still in the caller's environment.
/// Running the rustup shim from a staged checkout without a rust-toolchain file
/// can silently select another compiler or fail without a default toolchain.
/// Pin the *real* rustc executable for invocation and SHA-256 provenance.
fn rustc_binary() -> Result<PathBuf> {
    let launcher = executable("rustc")?;
    let mut probe = Command::new(&launcher);
    probe
        .args(["--print", "sysroot"])
        .env("RUSTUP_AUTO_INSTALL", "0");
    let sysroot = String::from_utf8(run_bounded(probe, "native rustc sysroot")?)
        .map_err(|_| "non-UTF8 rustc sysroot")?;
    let compiler = PathBuf::from(sysroot.trim()).join("bin").join("rustc");
    if !compiler.is_file() {
        return refuse("selected Rust sysroot has no actual rustc executable");
    }
    compiler
        .canonicalize()
        .map_err(|e| format!("resolve rustc compiler: {e}"))
}
fn stage(repo: &Path, commit: &str, root: &Path) -> Result<(usize, usize, PinnedFiles)> {
    let entries = git::entries(repo, commit)?;
    let mut staged = BTreeMap::new();
    let mut bytes = 0usize;
    for entry in entries {
        git::validate_path(&entry.path)?;
        if entry.kind == "tree" {
            continue;
        }
        // A generated snapshot cannot safely preserve submodule/linked semantics.
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            return refuse(format!(
                "unsupported linked/submodule input: {}",
                entry.path
            ));
        }
        if staged.len() >= MAX_STAGE_FILES {
            return refuse("historical snapshot exceeds 4096 staged files");
        }
        let size = git::text(repo, &["cat-file", "-s", &entry.oid])?
            .parse::<usize>()
            .map_err(|_| "invalid Git blob size")?;
        bytes = bytes.checked_add(size).ok_or("staging byte overflow")?;
        if bytes > MAX_STAGE_BYTES {
            return refuse("historical snapshot exceeds 96 MiB staging bound");
        }
        let data = git::command(repo, &["cat-file", "blob", &entry.oid])?;
        if data.len() != size {
            return refuse("Git blob size mismatch");
        }
        let path = root.join(&entry.path);
        let parent = path.parent().ok_or("missing parent of staged path")?;
        fs::create_dir_all(parent).map_err(|e| format!("create snapshot parent: {e}"))?;
        fs::write(&path, &data).map_err(|e| format!("stage source: {e}"))?;
        let mut permissions = fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).map_err(|e| e.to_string())?;
        staged.insert(entry.path, (size, digest::sha256(&data)));
    }
    Ok((staged.len(), bytes, staged))
}

/// Refuse compiler-induced source edits or unexpected files in the snapshot.
/// This is a filesystem postcondition, not a sandbox or process attestation.
fn audit_snapshot(root: &Path, pinned: &PinnedFiles) -> Result<()> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in
            fs::read_dir(&directory).map_err(|e| format!("audit snapshot directory: {e}"))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return refuse("native resolver created a symlink");
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() {
                return refuse("native snapshot contains nonregular file");
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "snapshot path outside root")?
                .to_str()
                .ok_or("non-UTF8 native snapshot pathname")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            git::validate_path(&relative)?;
            let (bytes, hash) = pinned
                .get(&relative)
                .ok_or_else(|| format!("native resolver created untracked file: {relative}"))?;
            if !fs::metadata(&path)
                .map_err(|e| e.to_string())?
                .permissions()
                .readonly()
            {
                return refuse(format!(
                    "native resolver made staged source writable: {relative}"
                ));
            }
            let source = fs::read(&path).map_err(|e| format!("audit staged file: {e}"))?;
            if source.len() != *bytes || digest::sha256(&source) != *hash {
                return refuse(format!("native resolver altered source: {relative}"));
            }
            found.insert(relative);
        }
    }
    if found.len() != pinned.len() {
        return refuse("native resolver deleted pinned file");
    }
    Ok(())
}

fn assumptions(value: &Value) -> Result<BTreeMap<String, Value>> {
    let map = value.entries()?.clone();
    if map.len() > 24 {
        return refuse("too many language adapter assumptions");
    }
    for (key, item) in &map {
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return refuse("invalid adapter assumption key");
        }
        let val = item.str()?;
        if val.is_empty() || val.len() > 512 {
            return refuse("invalid adapter assumption value");
        }
    }
    Ok(map)
}

/// Produce a language-neutral input declaration from historical Git and the
/// selected installed compiler. The native code parser does not own Git, SQL,
/// source identity, or evidence interpretation.
pub fn discover(
    repo: &Path,
    revision: &str,
    entry: &str,
    language: &str,
    import_roots: Option<&str>,
    trusted: bool,
) -> Result<Value> {
    if !trusted {
        return refuse("native compiler invocation requires --trusted-local true");
    }
    if !matches!(
        language,
        "go" | "javascript" | "typescript" | "rust" | "python"
    ) {
        return refuse("unsupported native language");
    }
    if (language == "python") != import_roots.is_some() {
        return refuse("Python requires --import-roots; other languages do not accept it");
    }
    let roots = import_roots.unwrap_or("");
    if language == "python" {
        if roots.is_empty() || roots.len() > 512 || roots.split(',').count() > 16 {
            return refuse("invalid bounded Python import root list");
        }
        for part in roots.split(',') {
            if part != "." {
                git::validate_path(part)?;
            }
        }
    }
    git::verify_repo(repo)?;
    git::validate_path(entry)?;
    let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
    let t0 = Instant::now();
    let snapshot = Snapshot::new()?;
    let root = snapshot.source();
    fs::create_dir(&root).map_err(|e| format!("native snapshot: {e}"))?;
    let (staged_files, staged_bytes, staged) = stage(repo, &commit, &root)?;
    if !staged.contains_key(entry) {
        return refuse("entrypoint absent from tracked Git snapshot");
    }
    let (tool, version_flag) = match language {
        "go" => ("go", "version"),
        "rust" => ("rustc", "--version"),
        "python" => ("python3", "--version"),
        _ => ("node", "--version"),
    };
    let pinned_rustc = if language == "rust" {
        Some(rustc_binary()?)
    } else {
        None
    };
    let (version, binary_sha256) = if let Some(ref path) = pinned_rustc {
        toolchain_at(path, version_flag)?
    } else {
        toolchain(tool, version_flag)?
    };
    let (adapter_python_version, adapter_python_sha256) = toolchain("python3", "--version")?;
    let helper = snapshot.directory.join("typescript.cjs");
    fs::write(&helper, TYPESCRIPT_HELPER)
        .map_err(|e| format!("write embedded TypeScript helper: {e}"))?;
    let mut cmd = Command::new(executable("python3")?);
    cmd.arg("-I")
        .arg("-S")
        .arg("-B")
        .arg("-c")
        .arg(RESOLVER)
        .arg(&root)
        .arg(entry)
        .arg(language)
        .arg(&helper)
        .arg(roots)
        .current_dir(&root);
    if let Some(ref compiler) = pinned_rustc {
        cmd.env("EXCESSION_NATIVE_RUSTC", compiler);
    }
    let raw = run_bounded(cmd, "language-native resolver")?;
    let found = json::parse(&raw)?;
    if found.entries()?.len() != if language == "python" { 4 } else { 3 } {
        return refuse("unexpected language resolver output fields");
    }
    let mut selected = BTreeMap::<String, &'static str>::new();
    for (field, role) in [("sources", "source"), ("configuration", "configuration")] {
        for path_value in found.get(field)?.array_ref()? {
            let path = path_value.str()?;
            git::validate_path(path)?;
            if !staged.contains_key(path) {
                return refuse(format!("untracked native dependency: {path}"));
            }
            if selected.insert(path.to_string(), role).is_some() {
                return refuse(format!("duplicate role for native dependency: {path}"));
            }
        }
    }
    if !selected.contains_key(entry) || selected.len() > MAX_INPUT_FILES {
        return refuse("native dependency entry missing or exceeds 512 files");
    }
    audit_snapshot(&root, &staged)?;
    let size = selected.keys().map(|p| staged[p].0).sum::<usize>();
    if size > MAX_INPUT_BYTES {
        return refuse("native input set exceeds 32 MiB");
    }
    let observations = if language == "python" {
        let observations = found.get("observations")?.array_ref()?;
        if observations.len() > 2048 {
            return refuse("Python observations exceed bounded site count");
        }
        for observation in observations {
            if observation.entries()?.len() != 6 {
                return refuse("invalid Python observation fields");
            }
            let path = observation.get("source")?.str()?;
            if !selected.contains_key(path) || path.len() > 4096 {
                return refuse("Python observation source absent from authenticated inputs");
            }
            let line = observation.get("line")?.integer()?;
            if line < 1 {
                return refuse("invalid Python observation line");
            }
            let specifier = observation.get("specifier")?.str()?;
            let reason = observation.get("reason")?.str()?;
            if specifier.is_empty() || specifier.len() > 512 || reason.len() > 512 {
                return refuse("invalid Python observation text");
            }
            if !matches!(
                observation.get("status")?.str()?,
                "RESOLVED" | "CANDIDATE" | "UNRESOLVED" | "UNSUPPORTED"
            ) {
                return refuse("invalid Python observation status");
            }
            let targets = observation.get("targets")?.array_ref()?;
            if targets.len() > MAX_INPUT_FILES {
                return refuse("Python observation targets exceed bound");
            }
            let mut previous = "";
            for target in targets {
                let target = target.str()?;
                if target <= previous || !selected.contains_key(target) {
                    return refuse("Python observation target not declared or unsorted");
                }
                previous = target;
            }
        }
        Some(Value::array(observations.to_vec()))
    } else {
        None
    };
    let mut values = assumptions(found.get("assumptions")?)?;
    for (key, value) in [
        (
            "dependency-discovery",
            "excession-toolchain-native-v1".to_string(),
        ),
        ("toolchain-version", version),
        ("toolchain-sha256", binary_sha256),
        ("adapter-python-version", adapter_python_version),
        ("adapter-python-sha256", adapter_python_sha256),
        ("adapter-source-sha256", digest::sha256(RESOLVER.as_bytes())),
        (
            "typescript-helper-sha256",
            digest::sha256(TYPESCRIPT_HELPER.as_bytes()),
        ),
        (
            "platform",
            format!("{}-{}", env::consts::OS, env::consts::ARCH),
        ),
    ] {
        if values.insert(key.into(), Value::string(value)).is_some() {
            return refuse(format!(
                "adapter may not override authority assumption: {key}"
            ));
        }
    }
    let inputs = selected
        .iter()
        .map(|(path, role)| {
            obj(&[
                ("path", Value::string(path)),
                ("role", Value::string(*role)),
            ])
        })
        .collect::<Vec<_>>();
    let declaration = obj(&[
        ("schema", Value::string("excession.input-declaration.v1")),
        ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
        ("language", Value::string(language)),
        ("subject", Value::string(entry)),
        ("inputs", Value::array(inputs)),
        ("assumptions", Value::Object(values)),
    ]);
    let stats = obj(&[
        ("commit", Value::string(&commit)),
        ("staged_files", Value::number(staged_files)),
        ("staged_bytes", Value::number(staged_bytes)),
        ("declared_files", Value::number(selected.len())),
        ("declared_bytes", Value::number(size)),
        ("elapsed_ns", Value::number(t0.elapsed().as_nanos())),
    ]);
    let mut result = obj(&[("declaration", declaration), ("stats", stats)]);
    if let Some(observations) = observations {
        let mut resolution = obj(&[
            ("schema", Value::string("excession.python-resolution.v1")),
            ("source_commit", Value::string(&commit)),
            ("subject", Value::string(entry)),
            ("import_roots", Value::string(roots)),
            ("observations", observations),
        ]);
        let report_id = resolution.sha256();
        resolution
            .entries_mut()?
            .insert("report_id".into(), Value::string(report_id));
        result
            .entries_mut()?
            .insert("resolution".into(), resolution);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_and_untrusted_fail_closed() {
        assert!(discover(Path::new("/tmp"), "HEAD", "main.go", "go", None, false).is_err());
        assert!(discover(Path::new("/tmp"), "HEAD", "main.go", "unsafe", None, true).is_err());
    }
    #[test]
    fn language_assumption_keys_are_bounded() {
        assert!(assumptions(&obj(&[("edition", Value::string("2024"))])).is_ok());
        assert!(assumptions(&obj(&[("WRONG", Value::string("a"))])).is_err());
        assert!(assumptions(&obj(&[("empty", Value::string(""))])).is_err());
    }
    #[test]
    fn local_git_reconstruction_is_exact_and_declared() {
        let root = Snapshot::new().unwrap();
        let repo = root.directory.join("repo");
        fs::create_dir(&repo).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let source = repo.join("main.go");
        fs::write(&source, "package test\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.org",
                    "commit",
                    "-qm",
                    "base"
                ])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let oid = git::rev(&repo, "HEAD^{commit}").unwrap();
        let restored = root.directory.join("restored");
        fs::create_dir(&restored).unwrap();
        let (count, size, entries) = stage(&repo, &oid, &restored).unwrap();
        assert_eq!(count, 1);
        assert_eq!(size, 13);
        assert_eq!(entries["main.go"].0, 13);
        assert_eq!(
            fs::read(restored.join("main.go")).unwrap(),
            b"package test\n"
        );
        audit_snapshot(&restored, &entries).unwrap();
        // Staged files are read-only. Override this protection only in the
        // adversarial test to simulate a hostile compiler process.
        let source = restored.join("main.go");
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o644);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(&source, permissions).unwrap();
        fs::write(&source, "changed").unwrap();
        assert!(audit_snapshot(&restored, &entries).is_err());
    }
}
