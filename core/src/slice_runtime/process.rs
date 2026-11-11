//! Process supervision, timeouts, bounded pipes, and namespace startup.
use super::{Result, refuse};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// This is an isolated *local* observer, not an authenticated remote executor.
// Namespace setup failure must never be converted to an observed test failure.
pub(super) const LINUX_WORKER: &str = r#"set -eu
jail="$1"
source_dir="$2"
entry="$3"
cpu="$4"
mkdir -p "$jail/usr" "$jail/work" "$jail/tmp" "$jail/dev"
ln -s usr/lib "$jail/lib"
ln -s usr/lib64 "$jail/lib64"
ln -s usr/bin "$jail/bin"
touch "$jail/dev/null"
mount --make-rprivate /
mount --bind /usr "$jail/usr"
mount -o remount,bind,ro "$jail/usr"
mount --bind "$source_dir" "$jail/work"
mount -o remount,bind,ro "$jail/work"
mount --bind /dev/null "$jail/dev/null"
mount -t tmpfs -o size=32m,nosuid,nodev,noexec tmpfs "$jail/tmp"
exec prlimit --cpu="$cpu" --as=1073741824 --nofile=128 -- \
  /usr/sbin/chroot "$jail" /usr/bin/setpriv --bounding-set=-all --no-new-privs \
  /usr/bin/python3 -I -B -c \
  "import os,runpy,sys;os.write(2,b'EXCESSION_WORKER_READY\\n');sys.path.insert(0,'/work');runpy.run_path('/work/'+sys.argv[1],run_name='__main__')" \
  "$entry"
"#;
pub(super) const WORKER_READY: &[u8] = b"EXCESSION_WORKER_READY\n";

#[derive(Debug)]
pub(super) struct Outcome {
    pub(super) code: i32,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) timed_out: bool,
}

/// Bound wall time and output of an external process. Threads drain both pipes
/// concurrently; a writer thread prevents pipe deadlock on large parser input.
pub(super) fn process(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    cap: usize,
) -> Result<Outcome> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Isolate the process group so a timeout cannot leave descendant workers
    // holding our output pipes open indefinitely.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|e| format!("subprocess launch refused: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("missing subprocess stdout pipe")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("missing subprocess stderr pipe")?;
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((cap + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take((cap + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let input_thread = input.map(|bytes| {
        let mut stdin = child.stdin.take().expect("stdin requested");
        thread::spawn(move || stdin.write_all(&bytes))
    });
    let start = Instant::now();
    let (status, timed_out) = loop {
        match child
            .try_wait()
            .map_err(|e| format!("subprocess wait refused: {e}"))?
        {
            Some(s) => break (s, false),
            None if start.elapsed() >= timeout => {
                #[cfg(unix)]
                let _ = Command::new("/usr/bin/kill")
                    .args(["-s", "KILL", "--", &format!("-{}", child.id())])
                    .status();
                let _ = child.kill();
                let s = child
                    .wait()
                    .map_err(|e| format!("subprocess kill failed: {e}"))?;
                break (s, true);
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    // Terminate orphaned grandchildren even when the direct child exited.
    // Otherwise pipe readers could wait forever for inherited descriptors.
    #[cfg(unix)]
    let _ = Command::new("/usr/bin/kill")
        .args(["-s", "KILL", "--", &format!("-{}", child.id())])
        .status();
    if let Some(writer) = input_thread {
        let _ = writer
            .join()
            .map_err(|_| "subprocess input writer panicked")?;
    }
    let out = stdout_thread
        .join()
        .map_err(|_| "stdout reader panicked")?
        .map_err(|e| format!("stdout read failed: {e}"))?;
    let err = stderr_thread
        .join()
        .map_err(|_| "stderr reader panicked")?
        .map_err(|e| format!("stderr read failed: {e}"))?;
    if out.len() > cap || err.len() > cap {
        return refuse("subprocess output exceeds permitted byte limit");
    }
    #[cfg(unix)]
    let code = {
        use std::os::unix::process::ExitStatusExt;
        status
            .code()
            .unwrap_or_else(|| -status.signal().unwrap_or(255))
    };
    #[cfg(not(unix))]
    let code = status.code().unwrap_or(255);
    Ok(Outcome {
        code,
        stdout: out,
        stderr: err,
        timed_out,
    })
}
