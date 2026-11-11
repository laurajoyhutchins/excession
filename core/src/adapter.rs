//! Language parsing is delegated to the actual language parsers, never Regex.
//! Rust owns integrity, input OIDs, persistence and impact queries.
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::io::Write;
use std::process::{Command, Stdio};

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
    let mut process = Command::new(binary)
        .arg(if binary == "node" { "-e" } else { "-c" })
        .arg(code)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{binary} parser adapter unavailable: {e}"))?;
    let input = Value::array(entries.iter().cloned()).canonical();
    let input_result = {
        let mut stdin = process.stdin.take().ok_or("missing child stdin")?;
        stdin.write_all(input.as_bytes())
    };
    // Always collect the child status and stderr, including when it exits
    // before consuming stdin. An early EPIPE would otherwise hide the real
    // parser error and leave orphaned subprocess handles until drop.
    let output = process
        .wait_with_output()
        .map_err(|e| format!("{binary} adapter wait: {e}"))?;
    if let Err(error) = input_result {
        return refuse(format!(
            "{binary} adapter input: {error}; child status: {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
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
