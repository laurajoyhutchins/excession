//! Repository-scoped MCP stdio adapter. All analysis stays in the existing Rust
//! implementation. The wire protocol never executes project tools or edits Git.
use excession::{
    Result, agent, agent_locate, analyze, discovery, impact_query, native_test_workflow,
};
use serde_json::{Map, Value, json};
use std::io::{self, BufRead, Write};
use std::path::Path;

const PROTOCOL: &str = "2025-11-25";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

fn fault(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name":name,
        "description":description,
        "inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}
    })
}

fn tool_catalog() -> Value {
    json!({"tools":[
        tool("search_code", "Find Git-pinned source candidates for a natural-language coding task. Heuristic results may be partial; check index coverage.",
            json!({"task":{"type":"string","minLength":1,"maxLength":2000},
                   "revision":{"type":"string","description":"Git revision, default HEAD"},
                   "limit":{"type":"integer","minimum":1,"maximum":64},
                   "index_segments":{"type":"integer","minimum":1,"maximum":256,"description":"Changed FTS partitions to refresh, default 8"}}), &["task"]),
        tool("get_context", "Explain authenticated sources, candidate relationships, test suggestions, and evidence gaps for exact tracked paths.",
            json!({"focus":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":16},
                   "revision":{"type":"string","description":"Git revision, default HEAD"},
                   "task":{"type":"string","maxLength":2000},
                   "limit":{"type":"integer","minimum":1,"maximum":64}}), &["focus"]),
        tool("explain_impact", "Recompute possible effects between two pinned Git revisions; no implied behavioral proof.",
            json!({"base":{"type":"string"},"candidate":{"type":"string"}}), &["base","candidate"]),
        tool("verify_change", "Compare two distinct retained commits. Reports unresolved obligations and never authorizes merge.",
            json!({"base":{"type":"string"},"candidate":{"type":"string"},
                   "limit":{"type":"integer","minimum":1,"maximum":64}}), &["base","candidate"]),
        tool("plan_tests", "Plan native test selection without executing project code; may return unsupported or blocked.",
            json!({"base":{"type":"string"},"candidate":{"type":"string"},
                   "provider":{"type":"string","enum":["auto","jest","pytest-testmon"]}}), &[]),
        tool("check_coverage", "Analyze supported Git-pinned source relationships and report coverage gaps. No compiler execution.",
            json!({"revision":{"type":"string","description":"Git revision, default HEAD"}}), &[])
    ]})
}

fn allowed(args: &Map<String, Value>, keys: &[&str]) -> std::result::Result<(), String> {
    if let Some(key) = args.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(format!("unexpected argument {key:?}"));
    }
    Ok(())
}

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> std::result::Result<&'a str, String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a nonempty string"))?;
    if value.is_empty() || value.len() > 2000 {
        return Err(format!("{key} must contain 1..2000 bytes"));
    }
    Ok(value)
}

fn optional<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    default: &'a str,
) -> std::result::Result<&'a str, String> {
    if args.contains_key(key) {
        required(args, key)
    } else {
        Ok(default)
    }
}

fn limit(
    args: &Map<String, Value>,
    key: &str,
    default: usize,
    max: usize,
) -> std::result::Result<usize, String> {
    let n = match args.get(key) {
        Some(n) => n
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{key} must be an integer"))?,
        None => default,
    };
    if !(1..=max).contains(&n) {
        return Err(format!("{key} must be in 1..={max}"));
    }
    Ok(n)
}

fn as_wire(report: excession::json::Value) -> std::result::Result<Value, String> {
    serde_json::from_str(&report.canonical())
        .map_err(|err| format!("serialize authenticated evidence: {err}"))
}

fn call(
    name: &str,
    args: &Map<String, Value>,
    repo: &Path,
    roots: Option<&str>,
    db: Option<&str>,
) -> std::result::Result<Value, String> {
    let report = match name {
        "search_code" => {
            allowed(args, &["task", "revision", "limit", "index_segments"])?;
            agent_locate::locate_with_budget(
                repo,
                optional(args, "revision", "HEAD")?,
                required(args, "task")?,
                limit(args, "limit", 20, 64)?,
                roots,
                db,
                limit(args, "index_segments", 8, 256)?,
            )?
        }
        "get_context" => {
            allowed(args, &["focus", "revision", "task", "limit"])?;
            let values = args
                .get("focus")
                .and_then(Value::as_array)
                .ok_or("focus must be an array of tracked paths")?;
            if values.is_empty() || values.len() > 16 {
                return Err("focus must contain 1..16 paths".into());
            }
            let mut paths = Vec::new();
            for value in values {
                let path = value.as_str().ok_or("focus paths must be strings")?;
                if path.is_empty() || path.contains(',') || path.contains('\n') {
                    return Err("focus paths cannot be empty or contain commas/newlines".into());
                }
                paths.push(path);
            }
            let task = if args.contains_key("task") {
                Some(required(args, "task")?)
            } else {
                None
            };
            agent::context(
                repo,
                optional(args, "revision", "HEAD")?,
                &paths.join(","),
                task,
                limit(args, "limit", 32, 64)?,
                roots,
                db,
            )?
        }
        "explain_impact" => {
            allowed(args, &["base", "candidate"])?;
            impact_query::run_auto(
                repo,
                required(args, "base")?,
                required(args, "candidate")?,
                roots,
                db,
            )?
        }
        "verify_change" => {
            allowed(args, &["base", "candidate", "limit"])?;
            agent::verify(
                repo,
                required(args, "base")?,
                required(args, "candidate")?,
                limit(args, "limit", 32, 64)?,
                roots,
                db,
                None,
            )?
        }
        "plan_tests" => {
            allowed(args, &["base", "candidate", "provider"])?;
            let base = if args.contains_key("base") {
                Some(required(args, "base")?)
            } else {
                None
            };
            let candidate = if args.contains_key("candidate") {
                Some(required(args, "candidate")?)
            } else {
                None
            };
            let provider = if args.contains_key("provider") {
                Some(required(args, "provider")?)
            } else {
                None
            };
            native_test_workflow::execute(native_test_workflow::Request {
                repo,
                base,
                candidate,
                provider,
                tool: None,
                focus: None,
                run: false,
                verify: false,
            })?
        }
        "check_coverage" => {
            allowed(args, &["revision"])?;
            let (report, _) =
                analyze::run_default(repo, optional(args, "revision", "HEAD")?, roots, db)?;
            report
        }
        _ => return Err(format!("unknown tool {name:?}")),
    };
    as_wire(report)
}

fn handle(
    message: Value,
    repo: &Path,
    roots: Option<&str>,
    db: Option<&str>,
    initialized: &mut bool,
    ready: &mut bool,
) -> Option<Value> {
    let id = message.get("id")?.clone();
    if !(id.is_null() || id.is_string() || id.is_number()) {
        return Some(fault(Value::Null, -32600, "invalid JSON-RPC request id"));
    }
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(fault(id, -32600, "expected jsonrpc 2.0"));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Some(fault(id, -32600, "missing JSON-RPC method"));
    };
    match method {
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL);
            let version = if matches!(requested, "2025-11-25" | "2025-06-18" | "2025-03-26") {
                requested
            } else {
                PROTOCOL
            };
            *initialized = true;
            *ready = false;
            Some(reply(
                id,
                json!({
                    "protocolVersion":version,
                    "capabilities":{"tools":{"listChanged":false}},
                    "serverInfo":{"name":"excession","version":env!("CARGO_PKG_VERSION")},
                    "instructions":"Use search_code to locate relevant Git sources and get_context for exact paths. Use check_coverage to inspect unsupported or partial analysis, plan_tests before execution, and verify_change after committing. Candidate relationships are not correctness proofs. No MCP tool executes tests, edits source, or authorizes merge. This server is confined to its configured repository."
                }),
            ))
        }
        "ping" => Some(reply(id, json!({}))),
        _ if !*initialized || !*ready => Some(fault(
            id,
            -32000,
            "initialize and notifications/initialized are required first",
        )),
        "tools/list" => Some(reply(id, tool_catalog())),
        "tools/call" => {
            let params = match message.get("params").and_then(Value::as_object) {
                Some(p) => p,
                None => return Some(fault(id, -32602, "tools/call requires params object")),
            };
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(fault(id, -32602, "tools/call requires name"));
            };
            if ![
                "search_code",
                "get_context",
                "explain_impact",
                "verify_change",
                "plan_tests",
                "check_coverage",
            ]
            .contains(&name)
            {
                return Some(fault(id, -32602, &format!("unknown tool: {name}")));
            }
            let empty = Map::new();
            let args = match params.get("arguments") {
                None => &empty,
                Some(Value::Object(a)) => a,
                _ => return Some(fault(id, -32602, "arguments must be an object")),
            };
            let result = match call(name, args, repo, roots, db) {
                Ok(value) => json!({
                    "content":[{"type":"text","text":value.to_string()}],
                    "structuredContent":value,"isError":false
                }),
                Err(error) => json!({
                    "content":[{"type":"text","text":error}],"isError":true
                }),
            };
            Some(reply(id, result))
        }
        _ => Some(fault(id, -32601, &format!("unknown method: {method}"))),
    }
}

/// Stdio consumes one line-delimited JSON-RPC message per request. No logs on stdout.
pub(super) fn serve(repo: &Path, roots: Option<&str>, db: Option<&str>) -> Result<()> {
    let repo = discovery::inspect(repo, "HEAD")?.root;
    let input = io::stdin();
    let mut input = input.lock();
    let output = io::stdout();
    let mut output = output.lock();
    let mut initialized = false;
    let mut ready = false;
    loop {
        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .map_err(|e| format!("read MCP stdin: {e}"))?;
        if bytes == 0 {
            return Ok(());
        }
        let response = if bytes > MAX_REQUEST_BYTES {
            Some(fault(Value::Null, -32600, "MCP request exceeds 1 MiB"))
        } else {
            match serde_json::from_str::<Value>(&line) {
                Ok(message) => {
                    if message.get("id").is_none() {
                        if message.get("method").and_then(Value::as_str)
                            == Some("notifications/initialized")
                            && initialized
                        {
                            ready = true;
                        }
                        None
                    } else {
                        handle(message, &repo, roots, db, &mut initialized, &mut ready)
                    }
                }
                Err(_) => Some(fault(Value::Null, -32700, "invalid JSON message")),
            }
        };
        if let Some(response) = response {
            writeln!(output, "{response}").map_err(|e| format!("write MCP stdout: {e}"))?;
            output
                .flush()
                .map_err(|e| format!("flush MCP stdout: {e}"))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_catalog_do_not_need_project_execution() {
        let mut initialized = false;
        let mut ready = false;
        let init = handle(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-03-26"}}),
            Path::new("."),
            None,
            None,
            &mut initialized,
            &mut ready,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        ready = true;
        let list = handle(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            Path::new("."),
            None,
            None,
            &mut initialized,
            &mut ready,
        )
        .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 6);
        assert!(
            tools
                .iter()
                .all(|v| v["annotations"]["readOnlyHint"] == true)
        );
    }

    #[test]
    fn refusal_is_machine_readable_and_not_silent_success() {
        let mut initialized = true;
        let mut ready = true;
        let result = handle(
            json!({"jsonrpc":"2.0","id":"req","method":"tools/call",
            "params":{"name":"get_context","arguments":{"focus":[]}}}),
            Path::new("."),
            None,
            None,
            &mut initialized,
            &mut ready,
        )
        .unwrap();
        assert_eq!(result["id"], "req");
        assert_eq!(result["result"]["isError"], true);
        let unknown = handle(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"run_shell","arguments":{}}}),
            Path::new("."),
            None,
            None,
            &mut initialized,
            &mut ready,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }
}
