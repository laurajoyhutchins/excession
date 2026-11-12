//! Language parsing is delegated to the actual language parsers, never Regex.
//! Rust owns integrity, input OIDs, persistence and impact queries.
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Parse only changed Python blobs using the complete, pinned module catalog.
/// The original list input remains the full-analysis compatibility path.
pub fn python_subset(entries: &[Value], catalog: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/python_ast.py");
    run_payload(
        "python3",
        code,
        obj(&[
            ("catalog", Value::array(catalog.iter().cloned())),
            ("entries", Value::array(entries.iter().cloned())),
        ]),
    )
}

pub fn javascript(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/js_adapter.cjs");
    run("node", code, entries)
}

pub fn tree_sitter(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/tree_sitter.py");
    run("python3", code, entries)
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

fn run(binary: &str, code: &str, entries: &[Value]) -> Result<Value> {
    run_payload(binary, code, Value::array(entries.iter().cloned()))
}

fn run_payload(binary: &str, code: &str, payload: Value) -> Result<Value> {
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
    {
        let mut stdin = process.stdin.take().ok_or("missing child stdin")?;
        stdin
            .write_all(payload.canonical().as_bytes())
            .map_err(|e| format!("{binary} adapter input: {e}"))?;
    }
    let output = process
        .wait_with_output()
        .map_err(|e| format!("{binary} adapter wait: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "{binary} adapter failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    crate::json::parse(&output.stdout)
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
