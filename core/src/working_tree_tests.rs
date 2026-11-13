//! Fail-closed uncommitted test snapshots. The source checkout is read only.
//! Only a rootless, seccomp-enabled OCI runtime may execute native project code.
//! The disposable Git commit is ephemeral evidence, never a durable attestation.

use crate::json::{self, Value, obj};
use crate::{Result, digest, discovery, git, native_test_workflow, refuse};
use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const MAX_FILES: usize = 4096;
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_GIT_DB_KIB: u64 = 256 * 1024;
const MAX_SECONDS: u64 = 600;

pub struct Request<'a> {
    pub repo: &'a Path,
    pub base: Option<&'a str>,
    pub candidate: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub tool: Option<&'a str>,
    pub focus: Option<&'a str>,
    pub run: bool,
    pub verify: bool,
    pub sandbox_image: Option<&'a str>,
    pub expected_snapshot: Option<&'a str>,
    pub timeout_seconds: u64,
}

struct Temporary {
    path: PathBuf,
}
impl Temporary {
    #[cfg(target_os = "linux")]
    fn new() -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        let parent = std::env::temp_dir();
        let template = parent.join("excession-uncommitted-XXXXXX");
        let mut raw = template.as_os_str().as_bytes().to_vec();
        raw.push(0);
        let made = unsafe { libc::mkdtemp(raw.as_mut_ptr().cast()) };
        if made.is_null() {
            return refuse(format!("mkdtemp: {}", io::Error::last_os_error()));
        }
        Ok(Self {
            path: PathBuf::from(std::ffi::OsStr::from_bytes(&raw[..raw.len() - 1])),
        })
    }
    #[cfg(not(target_os = "linux"))]
    fn new() -> Result<Self> {
        refuse("uncommitted test isolation currently requires Linux with rootless Podman")
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Input {
    path: String,
    bytes: Option<Vec<u8>>,
    executable: bool,
    untracked: bool,
}
struct Capture {
    head: String,
    files: Vec<Input>,
    baseline: Option<Vec<u8>>,
    fingerprint: String,
}

fn nul_paths(data: &[u8]) -> Result<Vec<String>> {
    if !data.is_empty() && !data.ends_with(&[0]) {
        return refuse("unterminated Git path inventory");
    }
    let mut out = Vec::new();
    for raw in data.split(|b| *b == 0).filter(|x| !x.is_empty()) {
        let path = std::str::from_utf8(raw)
            .map_err(|_| "working-tree snapshot refuses non-UTF-8 paths")?;
        git::validate_path(path)?;
        if path.starts_with(".git/")
            || path == ".git"
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return refuse("snapshot path contains a Git metadata or non-normal component");
        }
        out.push(path.to_owned());
    }
    Ok(out)
}

fn trusted_git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("/usr/bin/git")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.sparseCheckout=false")
        .arg("-C")
        .arg(root)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| format!("Git inventory: {error}"))?;
    if !output.status.success() {
        return refuse(format!(
            "Git inventory failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(output.stdout)
}

fn source_paths(root: &Path) -> Result<(Vec<String>, BTreeSet<String>)> {
    let unmerged = trusted_git(root, &["ls-files", "--unmerged", "-z"])?;
    if !unmerged.is_empty() {
        return refuse("cannot snapshot unresolved merge conflicts");
    }
    let changed = nul_paths(&trusted_git(
        root,
        &[
            "diff",
            "HEAD",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "-z",
            "--",
        ],
    )?)?;
    let untracked = nul_paths(&trusted_git(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )?)?;
    let untracked = untracked
        .into_iter()
        .filter(|p| p != ".testmondata")
        .collect::<BTreeSet<_>>();
    let mut all = changed;
    all.extend(untracked.iter().cloned());
    all.sort();
    all.dedup();
    if all.len() > MAX_FILES {
        return refuse("snapshot contains over 4096 modified/untracked paths");
    }
    Ok((all, untracked))
}

fn forbidden_secret(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name,
            ".npmrc"
                | ".pypirc"
                | "id_rsa"
                | "id_ed25519"
                | ".netrc"
                | "credentials.json"
                | "service-account.json"
        )
        || path.starts_with(".ssh/")
        || path.contains("/.ssh/")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}

#[cfg(target_os = "linux")]
fn open_source(root: &Path, relative: &str) -> io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let dir = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let name = CString::new(relative).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let dirfd = unsafe {
        libc::open(
            dir.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if dirfd < 0 {
        return Err(io::Error::last_os_error());
    }
    let directory = unsafe { fs::File::from_raw_fd(dirfd) };
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    // RESOLVE_NO_XDEV is not needed; prohibit symlinks, magic links and escape.
    let how = OpenHow {
        flags: (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64,
        mode: 0,
        resolve: 0x02 | 0x04 | 0x08,
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            directory.as_raw_fd(),
            name.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        ) as i32
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}
#[cfg(not(target_os = "linux"))]
fn open_source(_root: &Path, _relative: &str) -> io::Result<fs::File> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

fn read_file(root: &Path, path: &str) -> Result<(Vec<u8>, bool)> {
    git::validate_path(path)?;
    let file = open_source(root, path)
        .map_err(|error| format!("confined, no-symlink source open {path:?}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect {path:?}: {error}"))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return refuse(format!("source is not a regular file <=16 MiB: {path:?}"));
    }
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {path:?}: {error}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return refuse(format!("source grew beyond 16 MiB: {path:?}"));
    }
    Ok((bytes, executable))
}

fn capture(root: &Path, expected_head: &str) -> Result<Capture> {
    if git::rev(root, "HEAD")? != expected_head {
        return refuse("HEAD changed during snapshot capture");
    }
    for entry in git::entries(root, expected_head)? {
        if entry.kind == "commit" || entry.mode == "120000" {
            return refuse(format!(
                "snapshot refuses Git submodules and symlinks: {}",
                entry.path
            ));
        }
    }
    // Refuse new symlinks and non-stage-zero entries even before inspecting bytes.
    for entry in trusted_git(root, &["ls-files", "--stage", "-z"])?
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
    {
        let header = entry.split(|b| *b == b'\t').next().unwrap_or(&[]);
        let text = std::str::from_utf8(header).map_err(|_| "invalid staged entry")?;
        if text.starts_with("120000 ") || text.starts_with("160000 ") || !text.ends_with(" 0") {
            return refuse("snapshot refuses staged links, submodules and unmerged entries");
        }
    }
    let (paths, untracked) = source_paths(root)?;
    let mut files = Vec::new();
    let mut total = 0usize;
    let mut fingerprint_rows = Vec::new();
    for path in paths {
        if forbidden_secret(&path) && untracked.contains(&path) {
            return refuse(format!(
                "refusing untracked potential credential {path:?}; ignore it and retry"
            ));
        }
        let is_untracked = untracked.contains(&path);
        let value = match read_file(root, &path) {
            Ok((bytes, executable)) => {
                total = total
                    .checked_add(bytes.len())
                    .ok_or("snapshot size overflow")?;
                if total > MAX_TOTAL_BYTES {
                    return refuse("modified/untracked source exceeds 128 MiB");
                }
                fingerprint_rows.push(obj(&[
                    ("path", Value::string(&path)),
                    ("sha256", Value::string(digest::sha256(&bytes))),
                    ("executable", Value::Bool(executable)),
                    ("untracked", Value::Bool(is_untracked)),
                ]));
                Some((bytes, executable))
            }
            Err(error) if !is_untracked && error.contains("No such file or directory") => {
                fingerprint_rows.push(obj(&[
                    ("path", Value::string(&path)),
                    ("deleted", Value::Bool(true)),
                ]));
                None
            }
            Err(error) => return Err(error),
        };
        files.push(Input {
            path,
            bytes: value.as_ref().map(|v| v.0.clone()),
            executable: value.as_ref().is_some_and(|v| v.1),
            untracked: is_untracked,
        });
    }
    let baseline = match open_source(root, ".testmondata") {
        Ok(_) => Some(read_file(root, ".testmondata")?.0),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return refuse(format!("unsafe testmon baseline: {error}"));
        }
    };
    let id = obj(&[
        ("head", Value::string(expected_head)),
        ("files", Value::array(fingerprint_rows)),
        (
            "baseline_sha256",
            baseline
                .as_ref()
                .map(|b| Value::string(digest::sha256(b)))
                .unwrap_or(Value::Null),
        ),
    ])
    .sha256();
    if git::rev(root, "HEAD")? != expected_head {
        return refuse("HEAD moved during source capture");
    }
    Ok(Capture {
        head: expected_head.to_owned(),
        files,
        baseline,
        fingerprint: id,
    })
}

pub fn has_source_changes(repo: &Path) -> Result<bool> {
    let root = &discovery::inspect(repo, "HEAD")?.root;
    Ok(!source_paths(root)?.0.is_empty())
}

fn git_sanitized(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("/usr/bin/git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
        ])
        .arg(root)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|error| format!("spawn isolated Git: {error}"))?;
    if !output.status.success() {
        return refuse(format!(
            "isolated Git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(())
}

fn clone_into(root: &Path, directory: &Path) -> Result<PathBuf> {
    let checkout = directory.join("repo");
    fs::create_dir(directory.join("empty-git-template"))
        .map_err(|e| format!("create empty Git template: {e}"))?;
    // Local copy without hard links avoids upload-pack, fetch helpers, filters
    // and credential access from the original repository's configuration.
    let output = Command::new("/usr/bin/git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "clone",
            "-q",
            "--local",
            "--no-hardlinks",
            "--no-checkout",
        ])
        .arg("--")
        .arg(root)
        .arg(&checkout)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TEMPLATE_DIR", directory.join("empty-git-template"))
        .output()
        .map_err(|error| format!("create independent Git clone: {error}"))?;
    if !output.status.success() {
        return refuse(format!(
            "independent clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(checkout)
}

fn check_git_storage(root: &Path) -> Result<()> {
    let output = trusted_git(root, &["count-objects", "-v"])?;
    let stats = std::str::from_utf8(&output).map_err(|_| "invalid Git object statistics")?;
    let size = stats
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(": ")?;
            if matches!(key, "size" | "size-pack") {
                Some(value.parse::<u64>().ok())
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("invalid Git object size")?;
    if size.len() != 2 || size.iter().sum::<u64>() > MAX_GIT_DB_KIB {
        return refuse("Git object database exceeds 256 MiB safe snapshot limit");
    }
    if root.join(".git/objects/info/alternates").exists() {
        return refuse("Git alternates are unsupported for isolated working-tree snapshots");
    }
    Ok(())
}

fn prepare_snapshot(
    original: &Path,
    state: &Capture,
    directory: &Path,
) -> Result<(PathBuf, String, String)> {
    check_git_storage(original)?;
    let checkout = clone_into(original, directory)?;
    git_sanitized(&checkout, &["checkout", "--force", "--detach", &state.head])?;
    let exclusion = checkout.join(".git/info/exclude");
    fs::create_dir_all(checkout.join(".git/info"))
        .map_err(|e| format!("create isolated Git info directory: {e}"))?;
    fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&exclusion)
        .and_then(|mut file| file.write_all(b"\n/.testmondata\n"))
        .map_err(|error| format!("exclude local coverage database from source commit: {error}"))?;
    for input in &state.files {
        let destination = checkout.join(&input.path);
        if let Some(bytes) = &input.bytes {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("snapshot parent: {e}"))?;
            }
            fs::write(&destination, bytes).map_err(|e| format!("snapshot write: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if input.executable { 0o755 } else { 0o644 };
                fs::set_permissions(&destination, fs::Permissions::from_mode(mode))
                    .map_err(|e| format!("snapshot permissions: {e}"))?;
            }
        } else if destination.exists() {
            fs::remove_file(&destination).map_err(|e| format!("snapshot deletion: {e}"))?;
        }
    }
    git_sanitized(&checkout, &["add", "-A", "--", "."])?;
    for input in &state.files {
        if input.untracked {
            git_sanitized(&checkout, &["add", "-f", "--", &input.path])?;
        }
    }
    let output = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(&checkout)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Excession Ephemeral",
            "-c",
            "user.email=ephemeral@invalid.example",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "Excession working-tree snapshot",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| format!("commit ephemeral snapshot: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "commit snapshot failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    let commit = git::rev(&checkout, "HEAD")?;
    let tree = git::rev(&checkout, &format!("{commit}^{{tree}}"))?;
    if let Some(baseline) = &state.baseline {
        fs::write(checkout.join(".testmondata"), baseline)
            .map_err(|e| format!("restore native coverage baseline: {e}"))?;
    }
    if !git::command(
        &checkout,
        &["status", "--porcelain", "--untracked-files=no"],
    )?
    .is_empty()
    {
        return refuse("temporary snapshot has dirty tracked files");
    }
    Ok((checkout, commit, tree))
}

fn valid_image(value: &str) -> bool {
    let (name, digest) = if let Some(parts) = value.rsplit_once("@sha256:") {
        parts
    } else if let Some(digest) = value.strip_prefix("sha256:") {
        ("sha256", digest)
    } else {
        return false;
    };
    (name == "sha256"
        || (!name.is_empty()
            && !name.starts_with('-')
            && name.bytes().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, b'/' | b'.' | b'_' | b'-' | b':')
            })))
        && digest.len() == 64
        && digest
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn sandbox_preflight(image: Option<&str>) -> Vec<String> {
    let mut blocked = Vec::new();
    let Some(image) = image else {
        blocked.push("PINNED_SANDBOX_IMAGE_REQUIRED".to_owned());
        return blocked;
    };
    if !valid_image(image) {
        blocked.push("SANDBOX_IMAGE_MUST_BE_PINNED_SHA256".to_owned());
        return blocked;
    }
    #[cfg(not(target_os = "linux"))]
    {
        blocked.push("ROOTLESS_PODMAN_LINUX_REQUIRED".to_owned());
        return blocked;
    }
    let info = Command::new("podman")
        .args([
            "info",
            "--format",
            "{{.Host.Security.Rootless}} {{.Host.Security.SeccompEnabled}}",
        ])
        .output();
    if !matches!(info, Ok(ref output) if output.status.success()
        && String::from_utf8_lossy(&output.stdout).trim() == "true true")
    {
        blocked.push("ROOTLESS_PODMAN_WITH_SECCOMP_REQUIRED".to_owned());
    }
    let image_found = Command::new("podman")
        .args(["image", "exists", image])
        .output()
        .is_ok_and(|output| output.status.success());
    if !image_found {
        blocked.push("PREINSTALLED_PINNED_IMAGE_REQUIRED_NO_PULL".to_owned());
    }
    blocked
}

fn tool_in_image(path: Option<&str>, provider: &str) -> Result<String> {
    let chosen = path.unwrap_or(if provider == "jest" {
        "/usr/local/bin/jest"
    } else {
        "/usr/local/bin/python3"
    });
    if !chosen.starts_with('/')
        || chosen.len() > 256
        || chosen.contains('\0')
        || !Path::new(chosen)
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return refuse(
            "--tool in uncommitted mode must be an absolute normalized path inside the sandbox image",
        );
    }
    Ok(chosen.to_owned())
}

fn shell_quote(value: &str) -> String {
    if value.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b':' | b'-' | b'_' | b'=' | b'@')
    }) && !value.is_empty()
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn podman_stop(cidfile: &Path) {
    if let Ok(id) = fs::read_to_string(cidfile) {
        let id = id.trim();
        if id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()) {
            let _ = Command::new("podman")
                .args(["rm", "--force", "--ignore", id])
                .output();
        }
    }
}

struct ContainerGuard {
    cidfile: PathBuf,
}
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        podman_stop(&self.cidfile);
    }
}

fn read_capped(mut input: impl Read, limit: usize, exceeded: Arc<AtomicBool>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let Ok(n) = input.read(&mut buffer) else {
            exceeded.store(true, Ordering::Release);
            break;
        };
        if n == 0 {
            break;
        }
        if bytes.len().saturating_add(n) > limit {
            exceeded.store(true, Ordering::Release);
            break;
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    bytes
}

fn execute_isolated(
    checkout: &Path,
    directory: &Path,
    image: &str,
    base: &str,
    candidate: &str,
    provider: &str,
    tool: Option<&str>,
    focus: Option<&str>,
    verify: bool,
    timeout_seconds: u64,
) -> Result<Value> {
    let binary = std::env::current_exe()
        .map_err(|e| format!("current executable: {e}"))?
        .canonicalize()
        .map_err(|e| format!("resolve executable: {e}"))?;
    if !binary.is_file() {
        return refuse("no regular Excession binary to mount");
    }
    let cidfile = directory.join("sandbox.cid");
    let mut command = Command::new("podman");
    command.args([
        "run",
        "--rm",
        "--pull=never",
        "--network=none",
        "--pid=private",
        "--ipc=private",
        "--uts=private",
        "--userns=keep-id",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--read-only",
        "--pids-limit=128",
        "--memory=1g",
        "--cpus=2",
        "--ulimit=nofile=256:256",
        "--ulimit=fsize=104857600:104857600",
        "--tmpfs=/tmp:rw,nosuid,nodev,size=134217728,mode=1777",
        "--tmpfs=/work:rw,nosuid,nodev,size=536870912,mode=1777",
        "--env=HOME=/tmp",
        "--env=XDG_CACHE_HOME=/tmp/cache",
        "--entrypoint=/bin/sh",
    ]);
    command.arg("--cidfile").arg(&cidfile);
    command.arg("--mount").arg(format!(
        "type=bind,src={},dst=/input,readonly",
        checkout.display()
    ));
    command.arg("--mount").arg(format!(
        "type=bind,src={},dst=/usr/local/bin/excession,readonly",
        binary.display()
    ));
    command.arg(image);
    command.args(["-c",
        "mkdir -p /work/repo && cp -a /input/. /work/repo/ && cd /work/repo && exec /usr/local/bin/excession \"$@\"",
        "sh", "tests", "--run", "--json", "--repo", "/work/repo",
        "--base", base, "--candidate", candidate, "--provider", provider,
    ]);
    if let Some(tool) = tool {
        command.args(["--tool", tool]);
    }
    if let Some(focus) = focus {
        command.args(["--focus", focus]);
    }
    if verify {
        command.arg("--verify");
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("start rootless sandbox: {e}"))?;
    let _cleanup = ContainerGuard {
        cidfile: cidfile.clone(),
    };
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take().ok_or("no sandbox stdout")?;
    let stderr = child.stderr.take().ok_or("no sandbox stderr")?;
    let a = Arc::clone(&exceeded);
    let b = Arc::clone(&exceeded);
    let out_reader = thread::spawn(move || read_capped(stdout, MAX_OUTPUT_BYTES, a));
    let err_reader = thread::spawn(move || read_capped(stderr, MAX_OUTPUT_BYTES, b));
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let code = loop {
        if Instant::now() >= deadline || exceeded.load(Ordering::Acquire) {
            podman_stop(&cidfile);
            let _ = child.kill();
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return refuse("sandbox timed out or exceeded bounded output; container terminated");
        }
        match child
            .try_wait()
            .map_err(|e| format!("wait for sandbox: {e}"))?
        {
            Some(status) => break status.code().ok_or("sandbox exited by signal")?,
            None => thread::sleep(Duration::from_millis(100)),
        }
    };
    let stdout = out_reader
        .join()
        .map_err(|_| "sandbox stdout reader failed")?;
    let stderr = err_reader
        .join()
        .map_err(|_| "sandbox stderr reader failed")?;
    if exceeded.load(Ordering::Acquire) {
        return refuse("sandbox produced oversized output");
    }
    if !matches!(code, 0 | 1 | 3) {
        return refuse(format!(
            "isolated test runner refused or crashed (exit {code}): {}",
            String::from_utf8_lossy(&stderr)
                .chars()
                .take(250)
                .collect::<String>()
        ));
    }
    let report = json::parse(&stdout)
        .map_err(|error| format!("sandbox returned invalid structured test evidence: {error}"))?;
    if report.get("schema")?.str()? != "excession.test-workflow.v1"
        || report.get("mode")?.str()? != "RUN"
        || report.get("candidate_commit")?.str()? != candidate
        || report.get("safe_to_skip_unlisted_tests")?.bool()?
    {
        return refuse("sandbox response has mismatched source identity or contract");
    }
    let status = report.get("status")?.str()?;
    if (status == "PASS" && code != 0)
        || (status == "FAIL" && code != 1)
        || (status == "INCONCLUSIVE" && code != 3)
        || !matches!(status, "PASS" | "FAIL" | "INCONCLUSIVE")
    {
        return refuse("sandbox process exit disagrees with its reported test status");
    }
    Ok(report)
}

fn report_id(report: &mut Value) -> Result<()> {
    report.entries_mut()?.remove("report_id");
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(id));
    Ok(())
}

pub fn execute(request: Request<'_>) -> Result<Value> {
    if request.verify && !request.run {
        return refuse("--verify requires --run");
    }
    if request.timeout_seconds == 0 || request.timeout_seconds > MAX_SECONDS {
        return refuse("--timeout must be an integer from 1 to 600 seconds");
    }
    if !matches!(request.candidate, None | Some("HEAD")) {
        return refuse("uncommitted test mode requires --candidate HEAD");
    }
    if request
        .expected_snapshot
        .is_some_and(|id| id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return refuse("--expected-snapshot must be a SHA-256 hex digest");
    }
    let discovered = discovery::inspect(request.repo, "HEAD")?;
    let root = discovered.root;
    let state = capture(&root, &discovered.commit)?;
    if state.files.is_empty() {
        return refuse("no uncommitted tracked/untracked source changes to snapshot");
    }
    if let Some(expected) = request.expected_snapshot {
        if expected != state.fingerprint {
            return refuse("working-tree fingerprint changed since the approved test plan");
        }
    }
    let image = request.sandbox_image;
    let mut blockers = sandbox_preflight(image)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let temp = Temporary::new()?;
    let (checkout, commit, tree) = prepare_snapshot(&root, &state, &temp.path)?;
    let base = if let Some(spec) = request.base {
        git::rev(&checkout, spec)?
    } else {
        state.head.clone()
    };
    // Python uses the image interpreter on PATH; Jest uses the image-global bin.
    let plan_tool = if request.provider == Some("pytest-testmon") && request.tool.is_none() {
        None
    } else {
        Some(tool_in_image(request.tool, request.provider.unwrap_or("jest"))?)
    };
    let mut plan = native_test_workflow::execute(native_test_workflow::Request {
        repo: &checkout,
        base: Some(&base),
        candidate: Some(&commit),
        provider: request.provider,
        tool: plan_tool.as_deref(),
        focus: request.focus,
        run: false,
        verify: false,
    })?;
    let provider = match plan.get("provider")? {
        Value::Null => {
            blockers.insert("MULTIPLE_OR_UNKNOWN_NATIVE_PROVIDERS".to_owned());
            None
        }
        value => Some(value.str()?.to_owned()),
    };
    for reason in plan.get("blockers")?.array_ref()? {
        let name = reason.str()?;
        if name != "JEST_NOT_INSTALLED" {
            blockers.insert(name.to_owned());
        }
    }
    let tool = match provider.as_deref() {
        Some("pytest-testmon") if request.tool.is_none() => None,
        Some(selected) => Some(tool_in_image(request.tool, selected)?),
        None => plan_tool,
    };
    let mut argv = vec![
        "excession".to_owned(),
        "tests".to_owned(),
        "--working-tree".to_owned(),
        "--run".to_owned(),
        "--json".to_owned(),
        "--repo".to_owned(),
        root.to_string_lossy().into_owned(),
        "--base".to_owned(),
        base.clone(),
        "--candidate".to_owned(),
        "HEAD".to_owned(),
        "--expected-snapshot".to_owned(),
        state.fingerprint.clone(),
        "--timeout".to_owned(),
        request.timeout_seconds.to_string(),
    ];
    if let Some(image) = image {
        argv.extend(["--sandbox-image".to_owned(), image.to_owned()]);
    }
    if let Some(ref selected) = provider {
        argv.extend(["--provider".to_owned(), selected.clone()]);
    }
    if let Some(tool) = request.tool {
        argv.extend(["--tool".to_owned(), tool.to_owned()]);
    }
    if let Some(focus) = request.focus {
        argv.extend(["--focus".to_owned(), focus.to_owned()]);
    }
    let next_command = argv
        .iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    let fingerprint = state.fingerprint.clone();
    let changed = state
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let untracked = state
        .files
        .iter()
        .filter(|file| file.untracked)
        .map(|file| Value::string(&file.path))
        .collect::<Vec<_>>();
    let working_tree = obj(&[
        ("source_head", Value::string(&state.head)),
        ("snapshot_sha256", Value::string(&fingerprint)),
        ("snapshot_commit", Value::string(&commit)),
        ("snapshot_tree", Value::string(&tree)),
        ("untracked_paths", Value::array(untracked)),
        ("ephemeral", Value::Bool(true)),
        ("replayable_after_completion", Value::Bool(false)),
        ("source_checkout_modified", Value::Bool(false)),
        (
            "receipt_trust",
            Value::string("EPHEMERAL_LOCAL_SANDBOX_UNATTESTED"),
        ),
    ]);
    if request.run && !blockers.is_empty() {
        return refuse(format!(
            "refusing uncommitted execution: {}; see excession tests for plan",
            blockers.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if request.run {
        let actual = capture(&root, &state.head)?;
        if actual.fingerprint != fingerprint {
            return refuse("source changed while preparing the isolated snapshot");
        }
        let provider = provider.ok_or("no supported native provider selected")?;
        let mut result = execute_isolated(
            &checkout,
            &temp.path,
            image.ok_or("missing pinned image")?,
            &base,
            &commit,
            &provider,
            tool.as_deref(),
            request.focus,
            request.verify,
            request.timeout_seconds,
        )?;
        result
            .entries_mut()?
            .insert("repo".to_owned(), Value::string(root.to_string_lossy()));
        result
            .entries_mut()?
            .insert("working_tree".to_owned(), working_tree);
        result.entries_mut()?.insert(
            "schema".to_owned(),
            Value::string("excession.test-workflow.v2"),
        );
        result.entries_mut()?.insert(
            "next_argv".to_owned(),
            Value::array(argv.into_iter().map(Value::string)),
        );
        result
            .entries_mut()?
            .insert("next_command".to_owned(), Value::string(next_command));
        result.entries_mut()?.insert(
            "guidance".to_owned(),
            Value::string("Native execution occurred inside rootless Podman, never on the host"),
        );
        report_id(&mut result)?;
        return Ok(result);
    }

    let status = if blockers.is_empty() {
        "READY"
    } else {
        "BLOCKED"
    };
    {
        let entries = plan.entries_mut()?;
        entries.insert(
            "schema".to_owned(),
            Value::string("excession.test-workflow.v2"),
        );
        entries.insert("repo".to_owned(), Value::string(root.to_string_lossy()));
        entries.insert("status".to_owned(), Value::string(status));
        entries.insert(
            "changed_sources".to_owned(),
            Value::array(changed.into_iter().map(Value::string)),
        );
        entries.insert(
            "blockers".to_owned(),
            Value::array(blockers.iter().map(Value::string)),
        );
        entries.insert(
            "next_argv".to_owned(),
            Value::array(argv.into_iter().map(Value::string)),
        );
        entries.insert("next_command".to_owned(), Value::string(next_command));
        entries.insert("guidance".to_owned(), Value::string(if blockers.contains("PINNED_SANDBOX_IMAGE_REQUIRED") {
            "Pass --sandbox-image registry/image@sha256:<digest> with preinstalled tests, Git and Excession-compatible libc"
        } else if blockers.is_empty() {
            "Explicit --run executes only in rootless Podman with no network, no host credentials or writable mounts"
        } else {
            "Repair reported sandbox or source prerequisites; host execution is never a fallback"
        }));
        entries.insert("working_tree".to_owned(), working_tree);
    }
    report_id(&mut plan)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_preserves_mixed_git_changes_without_touching_original() {
        let base = Temporary::new().unwrap();
        let source = base.path.join("source");
        fs::create_dir(&source).unwrap();
        let cmd = |args: &[&str]| {
            let out = Command::new("/usr/bin/git")
                .arg("-C")
                .arg(&source)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "Git {:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        cmd(&["init", "-q"]);
        fs::write(source.join("staged.py"), b"old staged\n").unwrap();
        fs::write(source.join("unstaged.py"), b"old unstaged\n").unwrap();
        fs::write(source.join("deleted.py"), b"to delete\n").unwrap();
        cmd(&["add", "."]);
        cmd(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ]);
        let head = git::rev(&source, "HEAD").unwrap();
        fs::write(source.join("staged.py"), b"new staged\n").unwrap();
        cmd(&["add", "staged.py"]);
        fs::write(source.join("unstaged.py"), b"new unstaged\n").unwrap();
        fs::remove_file(source.join("deleted.py")).unwrap();
        fs::write(source.join("new_test.py"), b"def test_untracked(): pass\n").unwrap();
        fs::write(source.join(".testmondata"), b"coverage baseline\n").unwrap();
        let index_before = fs::read(source.join(".git/index")).unwrap();
        let captured = capture(&source, &head).unwrap();
        assert_eq!(captured.files.len(), 4);
        assert_eq!(
            captured.baseline.as_deref(),
            Some(&b"coverage baseline\n"[..])
        );
        let work = Temporary::new().unwrap();
        let (snapshot, commit, _tree) = prepare_snapshot(&source, &captured, &work.path).unwrap();
        assert_eq!(git::rev(&snapshot, "HEAD^").unwrap(), head);
        assert_ne!(git::rev(&snapshot, "HEAD").unwrap(), head);
        assert_eq!(
            fs::read(snapshot.join("staged.py")).unwrap(),
            b"new staged\n"
        );
        assert_eq!(
            fs::read(snapshot.join("unstaged.py")).unwrap(),
            b"new unstaged\n"
        );
        assert_eq!(
            fs::read(snapshot.join("new_test.py")).unwrap(),
            b"def test_untracked(): pass\n"
        );
        assert!(!snapshot.join("deleted.py").exists());
        assert_eq!(
            fs::read(snapshot.join(".testmondata")).unwrap(),
            b"coverage baseline\n"
        );
        assert_eq!(fs::read(source.join(".git/index")).unwrap(), index_before);
        assert_eq!(git::rev(&source, "HEAD").unwrap(), head);
        assert_eq!(fs::read(source.join("staged.py")).unwrap(), b"new staged\n");
        assert!(
            git::command(
                &snapshot,
                &["status", "--porcelain", "--untracked-files=no"]
            )
            .unwrap()
            .is_empty()
        );
        assert_eq!(commit, git::rev(&snapshot, "HEAD").unwrap());
        fs::write(source.join("new_test.py"), b"changed again\n").unwrap();
        assert_ne!(
            capture(&source, &head).unwrap().fingerprint,
            captured.fingerprint
        );
    }

    #[test]
    fn source_symlinks_cannot_escape_snapshot() {
        let source = Temporary::new().unwrap();
        fs::create_dir(source.path.join("repo")).unwrap();
        let root = source.path.join("repo");
        let call = |args: &[&str]| {
            let output = Command::new("/usr/bin/git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
        };
        call(&["init", "-q"]);
        fs::write(root.join("ok.py"), b"x=1\n").unwrap();
        call(&["add", "."]);
        call(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "init",
        ]);
        let head = git::rev(&root, "HEAD").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("evil.py")).unwrap();
        assert!(capture(&root, &head).is_err());
    }

    #[test]
    fn digest_pins_sandbox_images_and_rejects_tags() {
        assert!(valid_image(&format!(
            "docker.io/library/debian@sha256:{}",
            "a".repeat(64)
        )));
        assert!(!valid_image("docker.io/library/debian:latest"));
        assert!(valid_image(&format!("sha256:{}", "b".repeat(64))));
        assert!(!valid_image(&format!("-image@sha256:{}", "a".repeat(64))));
    }
    #[test]
    fn unsafe_files_and_paths_refuse() {
        assert!(nul_paths(b"a/../b\0").is_err());
        assert!(nul_paths(b"source\0").is_ok());
        assert!(forbidden_secret("new/.env.local"));
        assert!(!forbidden_secret("tests/test_value.py"));
    }
    #[test]
    fn sandbox_tools_are_not_host_paths() {
        assert!(tool_in_image(Some("../jest"), "jest").is_err());
        assert!(tool_in_image(Some("/usr/bin/../bin/sh"), "jest").is_err());
        assert_eq!(tool_in_image(None, "jest").unwrap(), "/usr/local/bin/jest");
    }
    #[test]
    fn any_missing_isolation_prerequisite_blocks() {
        assert!(sandbox_preflight(None).contains(&"PINNED_SANDBOX_IMAGE_REQUIRED".to_owned()));
    }
}
