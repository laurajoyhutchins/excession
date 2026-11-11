//! Language parsing is delegated to the actual language parsers, never Regex.
//! Rust owns integrity, input OIDs, persistence and impact queries.
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::ffi::OsString;
use std::io::Write;
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

pub fn javascript(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/js_adapter.cjs");
    run("node", code, entries)
}

pub fn tree_sitter(entries: &[Value]) -> Result<Value> {
    let code = include_str!("../adapters/tree_sitter.py");
    run("python3", code, entries)
}

fn run(binary: &str, code: &str, entries: &[Value]) -> Result<Value> {
    let mut command = if binary == "node" {
        node_command()
    } else {
        python_command()
    };
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
            .write_all(Value::array(entries.iter().cloned()).canonical().as_bytes())
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
