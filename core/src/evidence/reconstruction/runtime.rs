//! Shared, bounded trusted-local execution and artifact primitives for C reconstruction.
//! Ownership is independent of either the manual attempt or automatic search.
//! This module does not grant execution authority; public callers still require consent.
use crate::evidence::identity::Subject;
use crate::json::{Value, obj};
use crate::{Result, digest, refuse};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio, Child};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) const MAX_TEXT: usize = 1024 * 1024;
pub(super) const FLAGS: &[&str] = &[
    "-O2",
    "-fno-stack-protector",
    "-fno-asynchronous-unwind-tables",
    "-fno-ident",
    "-c",
];
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Scratch(PathBuf);
impl Scratch {
    pub(super) fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "excession-reconstruct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).map_err(|e| format!("reconstruction scratch: {e}"))?;
        Ok(Self(dir))
    }
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn run(program: &str, args: &[&str]) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn trusted-local tool {program:?}: {e}"))?;
    wait_for_exit(
        &mut child,
        program,
        Duration::from_secs(15),
        Duration::from_millis(20),
        "trusted-local tool",
    )
}


/// The two reconstruction paths retain distinct deadlines and diagnostics,
/// but share one bounded process lifecycle and child-reaping rule.
pub(super) fn wait_for_exit(
    child: &mut Child,
    program: &str,
    timeout: Duration,
    poll: Duration,
    label: &str,
) -> Result<()> {
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return refuse(format!("{label} {program:?} failed: {status}")),
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return refuse(format!("{label} {program:?} timed out"));
            }
            None => std::thread::sleep(poll),
        }
    }
}

pub(super) fn bounded_file(path: &Path, max: usize) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(|e| format!("input metadata: {e}"))?;
    if !meta.is_file() || meta.len() > max as u64 {
        return refuse("reconstruction input is not a bounded regular file");
    }
    let bytes = fs::read(path).map_err(|e| format!("input read: {e}"))?;
    if bytes.is_empty() || bytes.len() > max {
        return refuse("reconstruction input must be nonempty and bounded");
    }
    Ok(bytes)
}

pub(super) fn code_region(objcopy: &str, source: &Path, destination: &Path) -> Result<Vec<u8>> {
    let arg = format!(".text={}", destination.display());
    run(
        objcopy,
        &[
            "--dump-section",
            &arg,
            source.to_str().ok_or("non-UTF-8 object path")?,
        ],
    )?;
    bounded_file(destination, MAX_TEXT)
}

pub(super) fn subject(bytes: &[u8]) -> Result<String> {
    Subject::Artifact {
        sha256: digest::sha256(bytes),
        size_bytes: bytes.len() as u64,
    }
    .id()
}

pub(super) fn sample(left: &[u8], right: &[u8]) -> (usize, Vec<Value>) {
    let mut mismatches = 0usize;
    let mut examples = Vec::new();
    for offset in 0..left.len().max(right.len()) {
        let l = left.get(offset);
        let r = right.get(offset);
        if l != r {
            mismatches += 1;
            if examples.len() < 16 {
                examples.push(obj(&[
                    ("offset", Value::number(offset)),
                    ("target", l.map_or(Value::Null, |b| Value::number(*b))),
                    ("candidate", r.map_or(Value::Null, |b| Value::number(*b))),
                ]));
            }
        }
    }
    (mismatches, examples)
}

