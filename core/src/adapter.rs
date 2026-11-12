//! Language parsing is delegated to the actual language parsers, never Regex.
//! Rust owns integrity, input OIDs, persistence and impact queries.
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Resolve interpreters without relying on Unix-only `python3` executable names.
/// Overrides are executable paths, not shell commands.
pub fn python_command() -> Command {
    let program = std::env::var_os("EXCESSION_PYTHON")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from(if cfg!(windows) { "python" } else { "python3" }));
    Command::new(program)
}

pub fn node_command() -> Command {
    let program = std::env::var_os("EXCESSION_NODE")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("node"));
    Command::new(program)
}

pub fn python(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/python_ast.py");
    run("python3", code, entries)
}

/// Parse only changed Python blobs against the complete catalog with bounded IO.
pub fn python_subset(entries: &[Value], catalog: &[Value]) -> Result<Value> {
    run_payload(
        "python3",
        include_str!("../adapters/python_ast.py"),
        obj(&[
            ("catalog", Value::array(catalog.iter().cloned())),
            ("entries", Value::array(entries.iter().cloned())),
        ]),
        AdapterLimits::default(),
    )
}

pub fn javascript(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/js_adapter.cjs");
    run("node", code, entries)
}

/// Resolve imports with TypeScript against only pinned source and config blobs.
pub fn javascript_with_config(entries: &[Value], configs: &[Value]) -> Result<Value> {
    run_payload(
        "node",
        include_str!("../adapters/js_adapter.cjs"),
        obj(&[
            ("entries", Value::array(entries.iter().cloned())),
            ("configs", Value::array(configs.iter().cloned())),
        ]),
        AdapterLimits::default(),
    )
}

pub fn tree_sitter(entries: &[Value]) -> Result<Value> {
    crate::native_syntax::parse(entries)
}

/// Load the locked TypeScript parser from a sidecar or an explicitly trusted
/// location. Never resolve it from the repository being indexed.
pub fn typescript_file() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("EXCESSION_TYPESCRIPT_JS") {
        let path = PathBuf::from(value);
        if !path.is_absolute() || !path.is_file() {
            return refuse("EXCESSION_TYPESCRIPT_JS must name an existing absolute file");
        }
        return path.canonicalize().map_err(|e| e.to_string());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let adjacent = executable
        .parent()
        .ok_or("executable has no parent")?
        .join("lib/typescript.js");
    if adjacent.is_file() {
        return adjacent.canonicalize().map_err(|e| e.to_string());
    }
    // Cargo's development build uses its own locked, locally installed parser.
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("crate manifest has no parent")?
        .join("node_modules/typescript/lib/typescript.js");
    if checkout.is_file() {
        return checkout.canonicalize().map_err(|e| e.to_string());
    }
    refuse("TypeScript parser missing: install the portable bundle or set EXCESSION_TYPESCRIPT_JS")
}

pub fn kernel_sources(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/kernel_parse.py");
    run("python3", code, entries)
}

pub fn clang_ast(request: &Value) -> Result<Value> {
    let code = include_str!("../adapters/clang_ast.py");
    let reply = run("python3", code, std::slice::from_ref(request))?;
    let records = reply.array_ref()?;
    if records.len() != 1 {
        return refuse("Clang adapter did not return exactly one result");
    }
    Ok(records[0].clone())
}

// A single large or hanging language adapter must not exhaust the host.
// These are per-invocation caps; oversized repositories need partitioned scans.
#[derive(Clone, Copy)]
struct AdapterLimits {
    timeout: Duration,
    input_bytes: usize,
    stdout_bytes: usize,
    stderr_bytes: usize,
}

impl Default for AdapterLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            input_bytes: 64 * 1024 * 1024,
            stdout_bytes: 64 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
        }
    }
}

fn bounded_reader<R: Read>(
    mut pipe: R,
    limit: usize,
    exceeded: Arc<AtomicBool>,
) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = pipe.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        if output.len().saturating_add(read) > limit {
            exceeded.store(true, Ordering::Relaxed);
            return Ok(Vec::new());
        }
        output.extend_from_slice(&chunk[..read]);
    }
    Ok(output)
}

fn read_pipe<R: Read + Send + 'static>(
    pipe: R,
    limit: usize,
    exceeded: Arc<AtomicBool>,
) -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(bounded_reader(pipe, limit, exceeded));
    });
    receiver
}

fn receive<T>(receiver: mpsc::Receiver<io::Result<T>>, deadline: Instant, name: &str) -> Result<T> {
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| format!("{name} did not finish before adapter deadline"))?
        .map_err(|error| format!("{name} I/O: {error}"))
}

fn run(binary: &str, code: &str, entries: &[Value]) -> Result<Value> {
    run_bounded(binary, code, entries, AdapterLimits::default())
}

fn run_bounded(
    binary: &str,
    code: &str,
    entries: &[Value],
    limits: AdapterLimits,
) -> Result<Value> {
    run_payload(binary, code, Value::array(entries.iter().cloned()), limits)
}

fn run_payload(binary: &str, code: &str, payload: Value, limits: AdapterLimits) -> Result<Value> {
    // Calculate and check the input size *before* launching the child.
    let input = payload.canonical().into_bytes();
    if input.len() > limits.input_bytes {
        return refuse(format!(
            "{binary} adapter input exceeds {} bytes",
            limits.input_bytes
        ));
    }
    let mut command = if binary == "node" {
        node_command()
    } else {
        python_command()
    };
    if binary == "node" {
        command.env("EXCESSION_TYPESCRIPT_JS", typescript_file()?);
    }
    let mut process = command
        .arg(if binary == "node" { "-e" } else { "-c" })
        .arg(code)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{binary} parser adapter unavailable: {e}"))?;
    let stdin = process.stdin.take().ok_or("missing child stdin")?;
    let stdout = process.stdout.take().ok_or("missing child stdout")?;
    let stderr = process.stderr.take().ok_or("missing child stderr")?;

    // Drain both output streams concurrently with input. Writing all stdin
    // before reading stdout can deadlock when both OS pipes fill up.
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout_rx = read_pipe(stdout, limits.stdout_bytes, Arc::clone(&exceeded));
    let stderr_rx = read_pipe(stderr, limits.stderr_bytes, Arc::clone(&exceeded));
    let (input_tx, input_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = stdin;
        let _ = input_tx.send(stdin.write_all(&input));
    });

    let deadline = Instant::now() + limits.timeout;
    let status = loop {
        if exceeded.load(Ordering::Relaxed) {
            let _ = process.kill();
            let _ = process.wait();
            return refuse(format!("{binary} adapter output exceeds byte limit"));
        }
        if let Some(status) = process
            .try_wait()
            .map_err(|e| format!("{binary} wait: {e}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = process.kill();
            let _ = process.wait();
            return refuse(format!("{binary} adapter exceeded its execution deadline"));
        }
        thread::sleep(Duration::from_millis(25));
    };

    let stdout = receive(stdout_rx, deadline, "adapter stdout")?;
    let stderr = receive(stderr_rx, deadline, "adapter stderr")?;
    // The producer can exit before the reader observes the cap. Never turn
    // an oversized response into a misleading JSON parse failure.
    if exceeded.load(Ordering::Relaxed) {
        return refuse(format!("{binary} adapter output exceeds byte limit"));
    }
    if !status.success() {
        return refuse(format!(
            "{binary} adapter failed: {}",
            String::from_utf8_lossy(&stderr)
        ));
    }
    receive(input_rx, deadline, "adapter stdin")?;
    crate::json::parse(&stdout)
}

pub fn entry(name: &str, path: &str, content: &str) -> Value {
    obj(&[
        ("name", Value::string(name)),
        ("path", Value::string(path)),
        ("content", Value::string(content)),
    ])
}

pub fn js_entry(path: &str, content: &str) -> Value {
    obj(&[
        ("path", Value::string(path)),
        ("content", Value::string(content)),
    ])
}

#[cfg(test)]
mod limits_tests {
    use super::*;

    #[test]
    fn adapter_bounded_execution_accepts_an_actual_parser_response() {
        let output = run_bounded(
            "python3",
            "import sys; sys.stdin.read(); print('[]')",
            &[],
            AdapterLimits {
                timeout: Duration::from_secs(5),
                ..AdapterLimits::default()
            },
        )
        .unwrap();
        assert!(output.array_ref().unwrap().is_empty());
    }

    #[test]
    fn adapter_kills_a_nonterminating_parser() {
        let error = run_bounded(
            "python3",
            "import time; time.sleep(30)",
            &[],
            AdapterLimits {
                timeout: Duration::from_millis(150),
                ..AdapterLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("execution deadline"), "{error}");
    }

    #[test]
    fn adapter_refuses_excessive_output() {
        let error = run_bounded(
            "python3",
            "import sys; sys.stdin.read(); sys.stdout.write('x'*16384)",
            &[],
            AdapterLimits {
                timeout: Duration::from_secs(5),
                stdout_bytes: 1024,
                ..AdapterLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("output exceeds"), "{error}");
    }

    #[test]
    fn adapter_refuses_oversized_input_before_launch() {
        let error = run_bounded(
            "python3",
            "import sys; sys.stdin.read(); print('[]')",
            &[Value::string("0123456789")],
            AdapterLimits {
                input_bytes: 4,
                ..AdapterLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("input exceeds"), "{error}");
    }
}
