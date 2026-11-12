//! Small, local MCP tool surface for an LLM authoring Excession ontologies.
//!
//! JSON-RPC over standard input/output; no network service, credentials,
//! command execution, mutable source code, or general-purpose SQL. All tools
//! are bound to the same startup repo/revision/official SCIP JSON index.
use crate::concepts_author;
use crate::concepts_query;
use crate::concepts_store;
use crate::concepts_value;
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::Db;
use crate::{Result, refuse};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

const MAX_FRAME_BYTES: usize = 512 * 1024;
const DRAFT_SCHEMA: &str =
    include_str!("../../contracts/concepts/ontology-proposal.v1.schema.json");
const QUERY_SCHEMA: &str = include_str!("../../contracts/concepts/query.v1.schema.json");

pub struct Config {
    pub repo: PathBuf,
    pub revision: String,
    pub index: PathBuf,
    pub source_prefix: Option<String>,
    pub db: PathBuf,
    pub allow_publish: bool,
}

fn native(v: &serde_json::Value) -> Result<Value> {
    let bytes = serde_json::to_vec(v).map_err(|e| format!("serialize MCP tool schema: {e}"))?;
    json::parse(&bytes)
}

fn specs(allow_publish: bool) -> Result<Value> {
    let draft = serde_json::json!({
        "type":"object","additionalProperties":false,"required":["proposal"],
        "properties":{"proposal":{"type":"object"}}
    });
    let tools = vec![
        serde_json::json!({
            "name":"search_code_sources",
            "description":"Discover tracked source paths present in this SCIP index before inspecting them. Path substring search and deterministic pages; missing paths do not prove missing code.",
            "inputSchema":{"type":"object","additionalProperties":false,"properties":{
                "search":{"type":"string","maxLength":128},
                "offset":{"type":"integer","minimum":0,"maximum":1000000}
            }}
        }),
        serde_json::json!({
            "name":"inspect_code_evidence",
            "description":"Read bounded pinned Git source text and exact SCIP observation IDs for a single indexed path. Use before proposing memberships. Unknown native coverage stays explicit.",
            "inputSchema":{"type":"object","additionalProperties":false,
               "required":["path"],"properties":{
               "path":{"type":"string","description":"Exact Git path in the source index"},
               "first_line":{"type":"integer","minimum":1,"description":"First Git source line, 1-based; default 1"}}}
        }),
        serde_json::json!({
            "name":"ontology_schema",
            "description":"Get the strict proposal JSON schema and provenance/review semantics. Read before drafting; names and rationale are untrusted interpretations.",
            "inputSchema":{"type":"object","additionalProperties":false,"properties":{}}
        }),
        serde_json::json!({
            "name":"draft_ontology",
            "description":"Validate an LLM-authored proposal against the pinned native SCIP observations and preview immutable UNREVIEWED semantic claims. Does not store or approve.",
            "inputSchema":draft
        }),
        serde_json::json!({
            "name":"audit_ontology",
            "description":"Recompute and audit a published UNREVIEWED ontology from the exact source index, proposal and SQLite snapshot. Does not certify its meaning.",
            "inputSchema":{"type":"object","additionalProperties":false,
                "required":["proposal","snapshot_id"],
                "properties":{"proposal":{"type":"object"},
                "snapshot_id":{"type":"string","pattern":"^[0-9a-f]{64}$"}}}
        }),
        serde_json::json!({
            "name":"query_ontology",
            "description":"Run a bounded read-only concept query against the exact stored snapshot, returning answerability and provenance gaps. Query schema via ontology_schema.",
            "inputSchema":{"type":"object","additionalProperties":false,
                "required":["query"],"properties":{"query":{"type":"object"}}}
        }),
        serde_json::json!({
            "name":"compare_native_baseline",
            "description":"Compare deterministic SCIP reciprocal-reference concepts to original refactor candidates. A diagnostic, not a validation of a proposed ontology.",
            "inputSchema":{"type":"object","additionalProperties":false,"properties":{}}
        }),
    ];
    let mut tools = tools;
    if allow_publish {
        tools.push(serde_json::json!({
            "name":"publish_ontology",
            "description":"Persist only an UNREVIEWED LLM-authored proposal into the existing SQLite database, after full Git/SCIP evidence checks. Requires explicit server startup permission; never approves a review.",
            "inputSchema":{"type":"object","additionalProperties":false,
                "required":["proposal"],"properties":{"proposal":{"type":"object"}}}
        }));
    }
    native(&serde_json::Value::Array(tools))
}

fn tool_result(value: Value) -> Value {
    let text = value.canonical();
    obj(&[
        (
            "content",
            Value::array([obj(&[
                ("type", Value::string("text")),
                ("text", Value::string(text)),
            ])]),
        ),
        ("structuredContent", value),
        ("isError", Value::Bool(false)),
    ])
}

fn tool_error(message: &str) -> Value {
    obj(&[
        (
            "content",
            Value::array([obj(&[
                ("type", Value::string("text")),
                ("text", Value::string(message)),
            ])]),
        ),
        ("isError", Value::Bool(true)),
    ])
}

fn tool(name: &str, args: &Value, config: &Config) -> Result<Value> {
    let repo = &config.repo;
    let revision = &config.revision;
    let index = &config.index;
    let prefix = config.source_prefix.as_deref();
    let fields = args.entries()?;
    match name {
        "search_code_sources" => {
            if fields.keys().any(|key| !matches!(key.as_str(), "search" | "offset")) {
                return refuse("unexpected source search parameter");
            }
            let search = args.get("search").ok().map(Value::str).transpose()?.unwrap_or("");
            let offset = args.get("offset").ok().map(Value::integer).transpose()?.unwrap_or(0);
            if !(0..=1_000_000).contains(&offset) {
                return refuse("source search offset must be 0..1000000");
            }
            concepts_author::list_sources(
                repo, revision, index, prefix, search,
                usize::try_from(offset).map_err(|e| e.to_string())?,
            )
        }
        "inspect_code_evidence" => {
            if fields
                .keys()
                .any(|key| !matches!(key.as_str(), "path" | "first_line"))
            {
                return refuse("unexpected source context parameter");
            }
            let line = args
                .get("first_line")
                .ok()
                .map(Value::integer)
                .transpose()?
                .unwrap_or(1);
            if !(1..=1_000_000).contains(&line) {
                return refuse("first_line must be 1..1000000");
            }
            concepts_author::context(
                repo,
                revision,
                index,
                prefix,
                args.get("path")?.str()?,
                usize::try_from(line).map_err(|e| e.to_string())?,
            )
        }
        "ontology_schema" => {
            if !fields.is_empty() {
                return refuse("ontology_schema accepts no arguments");
            }
            Ok(obj(&[
                ("proposal", json::parse(DRAFT_SCHEMA.as_bytes())?),
                ("query", json::parse(QUERY_SCHEMA.as_bytes())?),
                (
                    "authorization",
                    Value::string("UNVERIFIED_LLM_AUTHOR; no review authority"),
                ),
                (
                    "workflow",
                    Value::string(
                        "Inspect code evidence, propose a grounded ontology, preview, optionally publish unreviewed, then audit and query.",
                    ),
                ),
            ]))
        }
        "draft_ontology" => {
            if fields.len() != 1 {
                return refuse("draft_ontology requires only proposal");
            }
            concepts_author::preview(repo, revision, index, prefix, args.get("proposal")?)
        }
        "publish_ontology" => {
            if !config.allow_publish {
                return refuse("publishing disabled by MCP server startup policy");
            }
            if fields.len() != 1 {
                return refuse("publish_ontology requires only proposal");
            }
            let db = Db::open(&config.db)?;
            concepts_author::publish(&db, repo, revision, index, prefix, args.get("proposal")?)
        }
        "audit_ontology" => {
            if fields.len() != 2 {
                return refuse("audit_ontology requires proposal and snapshot_id");
            }
            let db = Db::open(&config.db)?;
            concepts_author::audit(
                &db,
                repo,
                revision,
                index,
                prefix,
                args.get("proposal")?,
                args.get("snapshot_id")?.str()?,
            )
        }
        "query_ontology" => {
            if fields.len() != 1 {
                return refuse("query_ontology requires only query");
            }
            let query = args.get("query")?;
            let db = Db::open(&config.db)?;
            // MCP startup fixes the repository revision. A snapshot from a
            // different Git source revision cannot be silently substituted.
            let (model, _) = concepts_store::audit(&db, query.get("snapshot_id")?.str()?)?;
            let current = git::rev(repo, revision)?;
            if model.get("source_commit")?.str()? != current {
                return refuse("MCP query snapshot belongs to a different pinned Git commit");
            }
            concepts_query::run(&db, query)
        }
        "compare_native_baseline" => {
            if !fields.is_empty() {
                return refuse("compare_native_baseline accepts no arguments");
            }
            concepts_value::compare_index(repo, revision, index, prefix)
        }
        _ => refuse("unknown or disabled Excession ontology tool"),
    }
}

fn response(id: Value, value: Value) -> Value {
    obj(&[
        ("jsonrpc", Value::string("2.0")),
        ("id", id),
        ("result", value),
    ])
}

fn protocol_error(id: Value, code: i64, message: &str) -> Value {
    obj(&[
        ("jsonrpc", Value::string("2.0")),
        ("id", id),
        (
            "error",
            obj(&[
                ("code", Value::number(code)),
                ("message", Value::string(message)),
            ]),
        ),
    ])
}

fn request(input: &Value, config: &Config) -> Value {
    let id = input.get("id").cloned().unwrap_or(Value::Null);
    let result = (|| -> Result<Value> {
        if input.get("jsonrpc")?.str()? != "2.0" {
            return refuse("MCP request requires JSON-RPC 2.0");
        }
        let method = input.get("method")?.str()?;
        match method {
            "initialize" => Ok(response(
                id.clone(),
                obj(&[
                    ("protocolVersion", Value::string("2025-06-18")),
                    ("capabilities", obj(&[("tools", obj(&[]))])),
                    (
                        "serverInfo",
                        obj(&[
                            ("name", Value::string("excession-ontology")),
                            ("version", Value::string(env!("CARGO_PKG_VERSION"))),
                        ]),
                    ),
                    (
                        "instructions",
                        Value::string(
                            "You author ontology proposals. Only pinned SCIP/Git observations are evidence. All authored semantics are unreviewed and no tool certifies truth.",
                        ),
                    ),
                ]),
            )),
            "ping" => Ok(response(id.clone(), obj(&[]))),
            "tools/list" => Ok(response(
                id.clone(),
                obj(&[("tools", specs(config.allow_publish)?)]),
            )),
            "tools/call" => {
                let params = input.get("params")?;
                let name = params.get("name")?.str()?;
                let arguments = params.get("arguments").cloned().unwrap_or(obj(&[]));
                // Tool errors are ordinary isError results, not JSON-RPC
                // transport failures that disconnect the agent.
                let result = match tool(name, &arguments, config) {
                    Ok(output) => tool_result(output),
                    Err(err) => tool_error(&err),
                };
                Ok(response(id.clone(), result))
            }
            _ => Ok(protocol_error(id.clone(), -32601, "unknown MCP method")),
        }
    })();
    match result {
        Ok(value) => value,
        Err(message) => protocol_error(id, -32602, &message),
    }
}

/// Local stdio endpoint. One JSON-RPC object per line, no other stdout.
/// No background process is created by Excession: the MCP host owns lifetime.
pub fn serve(config: &Config) -> Result<()> {
    git::verify_repo(&config.repo)?;
    git::rev(&config.repo, &config.revision)?;
    if !config.index.is_file() {
        return refuse("MCP SCIP decoded JSON index does not exist");
    }
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut line = String::new();
        let bytes = input
            .read_line(&mut line)
            .map_err(|e| format!("MCP input read: {e}"))?;
        if bytes == 0 {
            return Ok(());
        }
        if bytes > MAX_FRAME_BYTES {
            writeln!(
                output,
                "{}",
                protocol_error(Value::Null, -32600, "MCP input exceeds 512 KiB").canonical()
            )
            .map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
            continue;
        }
        let result = match json::parse(line.as_bytes()) {
            Ok(value) => {
                if value.get("id").is_err() {
                    // MCP notifications/initialized do not receive a reply.
                    continue;
                }
                request(&value, config)
            }
            Err(message) => protocol_error(Value::Null, -32700, &message),
        };
        writeln!(output, "{}", result.canonical()).map_err(|e| format!("MCP output write: {e}"))?;
        output
            .flush()
            .map_err(|e| format!("MCP output flush: {e}"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            repo: PathBuf::from("/unused"),
            revision: "a".repeat(40),
            index: PathBuf::from("/unused"),
            source_prefix: None,
            db: PathBuf::from("/unused"),
            allow_publish: false,
        }
    }
    #[test]
    fn tools_are_curated_and_publish_requires_explicit_host_grant() {
        let list = specs(false).unwrap();
        assert_eq!(list.array_ref().unwrap().len(), 7);
        assert!(!list.canonical().contains("\"publish_ontology\""));
        let allowed = specs(true).unwrap();
        assert_eq!(allowed.array_ref().unwrap().len(), 8);
        assert!(allowed.canonical().contains("\"publish_ontology\""));
    }
    #[test]
    fn mcp_initialize_and_tools_list_are_protocol_responses() {
        let init = request(
            &obj(&[
                ("jsonrpc", Value::string("2.0")),
                ("id", Value::number(1)),
                ("method", Value::string("initialize")),
            ]),
            &config(),
        );
        assert_eq!(
            init.get("result")
                .unwrap()
                .get("protocolVersion")
                .unwrap()
                .str()
                .unwrap(),
            "2025-06-18"
        );
        let list = request(
            &obj(&[
                ("jsonrpc", Value::string("2.0")),
                ("id", Value::number(2)),
                ("method", Value::string("tools/list")),
            ]),
            &config(),
        );
        assert_eq!(
            list.get("result")
                .unwrap()
                .get("tools")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            7
        );
    }
    #[test]
    fn untrusted_lm_cannot_publish_without_host_permission() {
        let call = request(
            &obj(&[
                ("jsonrpc", Value::string("2.0")),
                ("id", Value::number(3)),
                ("method", Value::string("tools/call")),
                (
                    "params",
                    obj(&[
                        ("name", Value::string("publish_ontology")),
                        ("arguments", obj(&[("proposal", obj(&[]))])),
                    ]),
                ),
            ]),
            &config(),
        );
        assert!(
            call.get("result")
                .unwrap()
                .get("isError")
                .unwrap()
                .bool()
                .unwrap()
        );
    }
}
