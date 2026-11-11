//! Local finite execution receipts, with optional fail-closed Linux namespaces.
use super::filesystem::audit;
use super::process::{LINUX_WORKER, WORKER_READY, process};
use super::{MAX_OUTPUT, PrivateDir};
use crate::json::{Value, obj};
use crate::{Result, digest, refuse};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub fn execute(
    repo: &Path,
    manifest: &Value,
    root: &Path,
    timeout: u64,
    isolated: bool,
    trusted: bool,
) -> Result<Value> {
    if !trusted {
        return refuse("native execution requires explicit --trusted-local true");
    }
    if !(1..=60).contains(&timeout) {
        return refuse("execution timeout must be 1..60 seconds");
    }
    audit(repo, manifest, root)?;
    let entry = manifest.get("entry")?.str()?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let (out, python_version, isolation) = if isolated {
        #[cfg(not(target_os = "linux"))]
        {
            return refuse("Linux namespace worker unavailable on this platform");
        }
        #[cfg(target_os = "linux")]
        {
            for name in ["unshare", "mount", "chroot", "setpriv", "prlimit"] {
                let probe = Command::new("which")
                    .arg(name)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !probe.status.success() {
                    return refuse(format!(
                        "Linux worker requires {name}; no unsandboxed fallback"
                    ));
                }
            }
            if !Path::new("/usr/bin/python3").is_file() || !Path::new("/usr/sbin/chroot").is_file()
            {
                return refuse("Linux worker requires /usr/bin/python3 and /usr/sbin/chroot");
            }
            let private = PrivateDir::new(&std::env::temp_dir(), "excession-namespace")?;
            let jail = private.path().join("root");
            fs::create_dir(&jail).map_err(|e| e.to_string())?;
            let mut command = Command::new("unshare");
            command.args([
                "--user",
                "--map-root-user",
                "--mount",
                "--pid",
                "--fork",
                "--ipc",
                "--uts",
                "--net",
                "/bin/sh",
                "-c",
                LINUX_WORKER,
                "excession-sandbox",
                jail.to_str().ok_or("non-UTF8 jail path")?,
                root.to_str().ok_or("non-UTF8 slice root")?,
                entry,
                &format!("{}", timeout + 2),
            ]);
            command
                .current_dir(private.path())
                .env_clear()
                .env("PATH", "/usr/sbin:/usr/bin:/bin")
                .env("LC_ALL", "C")
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("PYTHONNOUSERSITE", "1");
            let out = process(command, None, Duration::from_secs(timeout + 8), MAX_OUTPUT)?;
            if out.timed_out || !out.stderr.starts_with(WORKER_READY) {
                return refuse("isolated worker setup did not establish execution boundary");
            }
            let mut out = out;
            out.stderr.drain(..WORKER_READY.len());
            let mut version_cmd = Command::new("/usr/bin/python3");
            version_cmd.arg("--version");
            let v = process(version_cmd, None, Duration::from_secs(5), 2048)?;
            (
                out,
                String::from_utf8_lossy(&v.stdout)
                    .trim_start_matches("Python ")
                    .trim()
                    .to_owned(),
                "LOCAL_LINUX_NAMESPACES_UNATTESTED",
            )
        }
    } else {
        let program = "import runpy,sys;sys.path.insert(0,sys.argv[1]);runpy.run_path(sys.argv[2],run_name='__main__')";
        let mut command = Command::new("python3");
        command.args([
            "-I",
            "-B",
            "-c",
            program,
            root.to_str().ok_or("non-UTF8 slice root")?,
            root.join(entry).to_str().ok_or("non-UTF8 entry path")?,
        ]);
        command
            .current_dir(&root)
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONNOUSERSITE", "1")
            .env("HOME", &root)
            .env("LC_ALL", "C.UTF-8");
        let out = process(command, None, Duration::from_secs(timeout), MAX_OUTPUT)?;
        let mut version_cmd = Command::new("python3");
        version_cmd.args(["-c", "import sys;print(sys.version.split()[0])"]);
        let v = process(version_cmd, None, Duration::from_secs(5), 2048)?;
        (
            out,
            String::from_utf8_lossy(&v.stdout).trim().to_owned(),
            "NONE",
        )
    };
    // An observed failing test is valid evidence, but mutated source isn't.
    audit(repo, manifest, &root)?;
    let code = if out.timed_out { 124 } else { out.code };
    let stdout = if out.timed_out {
        &b""[..]
    } else {
        out.stdout.as_slice()
    };
    let stderr = if out.timed_out {
        &b"timeout"[..]
    } else {
        out.stderr.as_slice()
    };
    let mut receipt = obj(&[
        ("schema", Value::string("excession.slice-observation.v1")),
        ("view_id", manifest.get("view_id")?.clone()),
        ("commit", manifest.get("commit")?.clone()),
        ("entry", manifest.get("entry")?.clone()),
        ("python", Value::string(python_version)),
        ("exit_code", Value::number(code)),
        ("stdout_sha256", Value::string(digest::sha256(stdout))),
        ("stderr_sha256", Value::string(digest::sha256(stderr))),
        (
            "status",
            Value::string(if code == 0 {
                "OBSERVED_PASS"
            } else {
                "OBSERVED_FAIL"
            }),
        ),
        ("scope", Value::string("FINITE_TRUSTED_LOCAL_EXECUTION")),
        ("isolation", Value::string(isolation)),
    ]);
    let receipt_id = receipt.sha256();
    receipt
        .entries_mut()?
        .insert("receipt_id".into(), Value::string(receipt_id));
    Ok(receipt)
}
