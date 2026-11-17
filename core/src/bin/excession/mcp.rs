//! Repository-scoped stdio server built on the official Model Context Protocol Rust SDK.
//! rmcp handles JSON-RPC framing, lifecycle, protocol negotiation and tool routing.
//! Excession remains the sole Git/SQLite authority for every exposed computation.
use excession::{agent, agent_locate, analyze, discovery, impact_query, native_test_workflow};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct ExcessionMcp {
    repo: PathBuf,
    roots: Option<String>,
    db: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchBudgets {
    /// Maximum bytes read from a Git blob in one allocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_file_bytes: Option<usize>,
    /// Maximum new source bytes indexed per segment refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_segment_bytes: Option<usize>,
    /// Maximum new source files indexed per segment refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_segment_files: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// Task description for Git-pinned lexical source discovery.
    task: String,
    /// Git revision; defaults to HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    /// Maximum source candidates; defaults to 20 (1..64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    /// Lexical index partitions refreshed; defaults to 8 (1..256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index_segments: Option<usize>,
    /// Authorized, caller-selected indexing policy; defaults apply per field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budgets: Option<SearchBudgets>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContextArgs {
    /// One to sixteen exact tracked Git paths.
    focus: Vec<String>,
    /// Git revision; defaults to HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    /// Optional task description for context selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    /// Maximum entries; defaults to 32 (1..64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImpactArgs {
    /// Retained base Git commit.
    base: String,
    /// Retained candidate Git commit.
    candidate: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct VerifyArgs {
    /// Retained base Git commit.
    base: String,
    /// Retained candidate Git commit.
    candidate: String,
    /// Maximum evidence entries; defaults to 32 (1..64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanTestsArgs {
    /// Optional base Git commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<String>,
    /// Optional candidate Git commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate: Option<String>,
    /// Test planner: auto, jest or pytest-testmon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CoverageArgs {
    /// Git revision; defaults to HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
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
            allowed(args, &["task", "revision", "limit", "index_segments", "budgets"])?;
            let defaults = agent_locate::IndexLimits::default();
            let chosen = match args.get("budgets") {
                Some(value) => serde_json::from_value::<SearchBudgets>(value.clone())
                    .map_err(|err| format!("invalid indexing budgets: {err}"))?,
                None => SearchBudgets {
                    max_file_bytes: None,
                    max_segment_bytes: None,
                    max_segment_files: None,
                },
            };
            agent_locate::locate_with_limits(
                repo,
                optional(args, "revision", "HEAD")?,
                required(args, "task")?,
                limit(args, "limit", 20, 64)?,
                roots,
                db,
                agent_locate::LocateBudget {
                    index_segments: limit(args, "index_segments", 8, 256)?,
                    index: agent_locate::IndexLimits {
                        max_file_bytes: chosen.max_file_bytes.unwrap_or(defaults.max_file_bytes),
                        max_bytes_per_segment: chosen.max_segment_bytes.unwrap_or(defaults.max_bytes_per_segment),
                        max_files_per_segment: chosen.max_segment_files.unwrap_or(defaults.max_files_per_segment),
                    }.validate()?,
                },
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

impl ExcessionMcp {
    /// CPU/SQLite/Git operations are blocking; leave the MCP I/O runtime responsive.
    async fn invoke<T: Serialize>(
        &self,
        name: &'static str,
        args: T,
    ) -> std::result::Result<CallToolResult, McpError> {
        let serialized = serde_json::to_value(args)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let Value::Object(args) = serialized else {
            return Err(McpError::invalid_params(
                "expected an argument object",
                None,
            ));
        };
        let repo = self.repo.clone();
        let roots = self.roots.clone();
        let db = self.db.clone();
        let computed = tokio::task::spawn_blocking(move || {
            call(name, &args, &repo, roots.as_deref(), db.as_deref())
        })
        .await;
        match computed {
            Ok(Ok(report)) => {
                if report.to_string().len() > MAX_REPORT_BYTES {
                    Ok(CallToolResult::error(vec![ContentBlock::text(
                        "REFUSED: report exceeds 4 MiB; narrow the query",
                    )]))
                } else {
                    Ok(CallToolResult::structured(report))
                }
            }
            Ok(Err(reason)) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "REFUSED: {reason}"
            ))])),
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "REFUSED: analysis task failed: {error}"
            ))])),
        }
    }
}

#[tool_router]
impl ExcessionMcp {
    #[tool(
        description = "Find Git-pinned source candidates for a coding task. Results are heuristic; check coverage.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn search_code(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("search_code", args).await
    }

    #[tool(
        description = "Explain source identities, possible relationships, and gaps for exact tracked paths.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_context(
        &self,
        Parameters(args): Parameters<ContextArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("get_context", args).await
    }

    #[tool(
        description = "Compute candidate source impacts between two retained Git commits; not a behavioral proof.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn explain_impact(
        &self,
        Parameters(args): Parameters<ImpactArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("explain_impact", args).await
    }

    #[tool(
        description = "Recompute change impact and unresolved obligations; never authorizes merge.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn verify_change(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("verify_change", args).await
    }

    #[tool(
        description = "Plan native test selection without executing project code.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn plan_tests(
        &self,
        Parameters(args): Parameters<PlanTestsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("plan_tests", args).await
    }

    #[tool(
        description = "Analyze supported Git source relationships and explicit coverage gaps without compiler execution.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn check_coverage(
        &self,
        Parameters(args): Parameters<CoverageArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        self.invoke("check_coverage", args).await
    }
}

#[tool_handler]
impl ServerHandler for ExcessionMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Repository-scoped, Git-pinned evidence. Search for sources, inspect exact paths and coverage, then verify committed changes. Unknowns remain explicit. No tool edits source, executes tests, or approves merge.".to_string()
            )
    }
}

pub(super) fn serve(repo: &Path, roots: Option<&str>, db: Option<&str>) -> excession::Result<()> {
    let repo = discovery::inspect(repo, "HEAD")?.root;
    let server = ExcessionMcp {
        repo,
        roots: roots.map(str::to_owned),
        db: db.map(str::to_owned),
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("create MCP runtime: {error}"))?
        .block_on(async move {
            let running = server
                .serve(rmcp::transport::stdio())
                .await
                .map_err(|error| format!("initialize MCP stdio server: {error}"))?;
            running
                .waiting()
                .await
                .map_err(|error| format!("run MCP stdio server: {error}"))?;
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_budget_schema_rejects_unknown_or_invalid_values() {
        let budget: SearchBudgets = serde_json::from_str(
            r#"{"max_file_bytes":524288,"max_segment_bytes":1048576,"max_segment_files":2}"#,
        ).unwrap();
        assert_eq!(budget.max_file_bytes, Some(524288));
        assert_eq!(budget.max_segment_files, Some(2));
        assert!(serde_json::from_str::<SearchBudgets>(r#"{"unknown":12}"#).is_err());
        assert!(serde_json::from_str::<SearchBudgets>(r#"{"max_file_bytes":-1}"#).is_err());
        assert!(serde_json::from_str::<SearchBudgets>(r#"{"max_segment_files":"3"}"#).is_err());
    }

    #[test]
    fn tools_have_typed_schemas_and_read_only_annotations() {
        let tools = ExcessionMcp::tool_router();
        assert_eq!(tools.map.len(), 6);
        for entry in tools.map.values() {
            assert_eq!(
                entry
                    .attr
                    .annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint),
                Some(true)
            );
        }
    }
}
