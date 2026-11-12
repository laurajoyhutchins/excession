//! A bounded second native LSP action, derived from compiler diagnostics.
//! Neither compiler errors nor an import quick fix establishes semantic
//! correctness. Repairs remain explicitly unverified until independent gates.
use crate::json::{self, Value, obj};
use crate::{Result, digest, git, refactor_codegen, refactor_rust_analyzer, refuse, scip_refactor};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_PLAN: usize = 2 * 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 4 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_COMPILER_ERRORS: usize = 24;

struct ErrorLocation {
    path: String,
    line: usize,
    column: usize,
    code: String,
    message: String,
}

fn field<'a>(value: &'a Json, name: &str) -> Result<&'a Json> {
    value
        .get(name)
        .ok_or_else(|| format!("missing diagnostic field {name}"))
}

fn cargo_errors(bytes: &[u8], prefix: &str) -> Result<Vec<ErrorLocation>> {
    if bytes.len() > MAX_DIAGNOSTICS {
        return refuse("compiler diagnostics exceed 4 MiB");
    }
    let mut errors = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let event: Json = serde_json::from_slice(line)
            .map_err(|e| format!("expected cargo --message-format=json lines: {e}"))?;
        if event.get("reason").and_then(Json::as_str) != Some("compiler-message") {
            continue;
        }
        let message = field(&event, "message")?;
        if message.get("level").and_then(Json::as_str) != Some("error") {
            continue;
        }
        let code = message
            .pointer("/code/code")
            .and_then(Json::as_str)
            .unwrap_or("");
        let reason = message.get("message").and_then(Json::as_str).unwrap_or("");
        if reason.len() > 1024 {
            return refuse("compiler diagnostic message exceeds bound");
        }
        if !matches!(code, "E0432" | "E0425" | "E0433") {
            continue;
        }
        let spans = field(message, "spans")?
            .as_array()
            .ok_or("compiler spans must be an array")?;
        for span in spans
            .iter()
            .filter(|s| s.get("is_primary").and_then(Json::as_bool) == Some(true))
        {
            let file = field(span, "file_name")?
                .as_str()
                .ok_or("missing compiler file_name")?;
            if file.starts_with('/') || file.contains('\\') {
                return refuse("compiler diagnostic uses an unsupported source path");
            }
            let joined = if prefix.is_empty() {
                file.to_owned()
            } else {
                format!("{prefix}/{file}")
            };
            git::validate_path(&joined)?;
            if !joined.ends_with(".rs") || !joined.split('/').any(|part| part == "src") {
                continue;
            }
            let line = usize::try_from(
                field(span, "line_start")?
                    .as_u64()
                    .ok_or("invalid compiler line")?,
            )
            .map_err(|_| "compiler line overflow")?;
            let column = usize::try_from(
                field(span, "column_start")?
                    .as_u64()
                    .ok_or("invalid compiler column")?,
            )
            .map_err(|_| "compiler column overflow")?;
            if line == 0 || column == 0 {
                return refuse("compiler coordinates must be one-based");
            }
            errors.push(ErrorLocation {
                path: joined,
                line: line - 1,
                column: column - 1,
                code: code.to_owned(),
                message: reason.to_owned(),
            });
            if errors.len() > MAX_COMPILER_ERRORS {
                return refuse("too many compiler errors for bounded native repair");
            }
        }
    }
    errors.sort_by(|a, b| {
        (&a.path, a.line, a.column, &a.code).cmp(&(&b.path, b.line, b.column, &b.code))
    });
    errors.dedup_by(|a, b| a.path == b.path && a.line == b.line && a.column == b.column);
    Ok(errors)
}

/// Source bytes were committed to an isolated candidate. A repair may only
/// follow a source plan whose exact modified files are present in that commit.
fn validate_chain(repo: &Path, revision: &str, plan: &Value, base: &str) -> Result<String> {
    if plan.get("schema")?.str()? != "excession.rust-assist.v1"
        || plan.get("status")?.str()? != "APPLIED_UNVERIFIED"
        || plan.get("source_commit")?.str()? != base
    {
        return refuse("repair requires an applied rust-assist plan at the declared base");
    }
    let plan_id = plan.get("plan_id")?.str()?;
    if plan_id.len() != 64 || !plan_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return refuse("invalid parent code-action plan identity");
    }
    let canonical = obj(&[
        ("source_commit", Value::string(base)),
        ("candidate_id", plan.get("candidate_id")?.clone()),
        ("scip_report_id", plan.get("scip_report_id")?.clone()),
        ("action_title", plan.get("action_title")?.clone()),
        ("edits", plan.get("edits")?.clone()),
    ]);
    if canonical.sha256() != plan_id {
        return refuse("parent code-action plan ID is not its canonical content digest");
    }
    let entries = git::entries(repo, base)?
        .into_iter()
        .map(|entry| (entry.path, entry.oid))
        .collect::<BTreeMap<_, _>>();
    let changed = git::text(repo, &["diff", "--name-only", base, revision, "--"])?;
    let actual = changed.lines().map(str::to_owned).collect::<BTreeSet<_>>();
    let mut expected = BTreeSet::new();
    let rows = plan.get("changes")?.array_ref()?;
    if rows.is_empty() || rows.len() > 8 {
        return refuse("original refactor plan must modify between 1 and 8 files");
    }
    for row in rows {
        let path = row.get("path")?.str()?;
        git::validate_path(path)?;
        if !path.ends_with(".rs") || !expected.insert(path.to_owned()) {
            return refuse("duplicate or unsupported source in original refactor plan");
        }
        let original = row.get("original_blob")?;
        if let Some(oid) = entries.get(path) {
            if original.str()? != oid {
                return refuse("original refactor plan does not match Git base blob");
            }
        } else if !matches!(original, Value::Null) {
            return refuse("created refactor source must have null original blob");
        }
        let file = repo.join(path);
        if fs::symlink_metadata(&file)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return refuse("candidate Rust source must not be a symlink");
        }
        let body = fs::read(&file).map_err(|e| e.to_string())?;
        if body.len() > MAX_SOURCE_BYTES
            || digest::sha256(&body) != row.get("generated_sha256")?.str()?
        {
            return refuse("candidate source bytes differ from the parent refactor plan");
        }
        let original = if let Some(oid) = entries.get(path) {
            let mut reader = git::BlobReader::open(repo)?;
            let bytes = reader.read_capped(oid, MAX_SOURCE_BYTES)?;
            reader.finish()?;
            String::from_utf8(bytes).map_err(|_| "original source is not UTF-8")?
        } else {
            String::new()
        };
        let replay = refactor_codegen::replay_edits(&original, plan.get("edits")?, path)?;
        if replay.as_bytes() != body {
            return refuse("original Git blobs and parent byte edits do not replay to the candidate");
        }
    }
    if actual != expected {
        return refuse("candidate commit includes unexpected or missing source changes");
    }
    Ok(plan_id.to_owned())
}

fn byte_at_utf16(text: &str, line: usize, col: usize) -> Result<usize> {
    let mut pos = 0;
    for _ in 0..line {
        pos += text[pos..]
            .find('\n')
            .ok_or("diagnostic line outside Rust source")?
            + 1;
    }
    let end = text[pos..].find('\n').map_or(text.len(), |n| pos + n);
    let mut units = 0;
    for (byte, ch) in text[pos..end].char_indices() {
        if units == col {
            return Ok(pos + byte);
        }
        units += ch.len_utf16();
        if units > col {
            return refuse("compiler coordinate splits UTF-16 scalar");
        }
    }
    if units == col {
        Ok(end)
    } else {
        refuse("compiler coordinate outside line")
    }
}

fn candidates(
    repo: &Path,
    prefix: &str,
    analyzer: &Path,
    errors: &[ErrorLocation],
) -> Result<Vec<(String, String, Json)>> {
    let root = repo.canonicalize().map_err(|e| e.to_string())?;
    let workspace = if prefix.is_empty() {
        root.clone()
    } else {
        root.join(prefix)
    };
    if !workspace.join("Cargo.toml").exists() {
        return refuse("missing Rust workspace Cargo.toml");
    }
    let mut actions = Vec::new();
    for error in errors {
        let file = root.join(&error.path);
        if fs::symlink_metadata(&file)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return refuse("compiler diagnostic Rust source is a symlink");
        }
        let bytes = fs::read(&file).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return refuse("compiler source exceeds native action file limit");
        }
        let source = String::from_utf8(bytes).map_err(|_| "compiler source is not UTF-8")?;
        // Rustc's column is measured in display columns, not necessarily
        // UTF-16. Admit only ASCII before the cursor; otherwise fail closed.
        let start = byte_at_utf16(&source, error.line, 0)?;
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |n| start + n);
        if !source[start..end].is_ascii() {
            return refuse("non-ASCII compiler diagnostic requires explicit LSP selection");
        }
        byte_at_utf16(&source, error.line, error.column)?;
        let relative = file
            .strip_prefix(&workspace)
            .map_err(|_| "diagnostic outside Rust workspace")?;
        // Supply the independent rustc diagnostic verbatim as CodeActionContext.
        let diagnostic = json!({
            "range": {
                "start":{"line":error.line,"character":error.column},
                "end":{"line":error.line,"character":error.column+1},
            },
            "severity":1,
            "code":error.code,
            "source":"rustc",
            "message":error.message,
        });
        let offered = refactor_rust_analyzer::request_actions(
            analyzer,
            &workspace,
            relative,
            &source,
            [error.line, error.column, error.line, error.column],
            None,
            &[diagnostic],
        )?;
        for action in offered {
            let title = action.get("title").and_then(Json::as_str).unwrap_or("");
            if action.get("kind").and_then(Json::as_str) == Some("quickfix")
                && title.starts_with("Import ")
                && action.get("edit").is_some()
                && action.get("command").is_none_or(Json::is_null)
            {
                actions.push((error.path.clone(), title.to_owned(), action));
            }
        }
    }
    Ok(actions)
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub base: &'a str,
    pub plan: &'a Path,
    pub index: &'a Path,
    pub report: &'a Path,
    pub diagnostics: &'a Path,
    pub source_prefix: Option<&'a str>,
    pub analyzer: &'a Path,
    pub output: Option<&'a Path>,
}

pub fn repair(request: &Request<'_>) -> Result<Value> {
    let repo = request.repo;
    git::clean(repo)?;
    // Repair only the explicitly committed result of a previous native action.
    // A user's primary checkout is never an eligible repair input.
    if !repo.join(".git").is_file() {
        return refuse("repair requires a clean, linked Git worktree");
    }
    let branch = git::command(repo, &["symbolic-ref", "--quiet", "HEAD"]);
    if branch.is_ok() {
        return refuse("repair requires a detached linked Git worktree");
    }
    let revision = git::rev(repo, "HEAD")?;
    let base = git::rev(repo, request.base)?;
    if revision == base {
        return refuse("repair source must be a committed native code-action candidate");
    }
    let plan_bytes = fs::read(request.plan).map_err(|e| format!("parent plan: {e}"))?;
    if plan_bytes.len() > MAX_PLAN {
        return refuse("parent plan exceeds 2 MiB");
    }
    let plan = json::parse(&plan_bytes)?;
    let prefix = request.source_prefix.unwrap_or("");
    scip_refactor::audit_with_prefix(
        repo, &base, request.index, request.report, request.source_prefix,
    )?;
    let report_bytes = fs::read(request.report).map_err(|e| format!("SCIP report: {e}"))?;
    let report = json::parse(&report_bytes)?;
    if plan.get("scip_report_id")?.str()? != report.get("report_id")?.str()? {
        return refuse("parent plan does not bind to independently replayed SCIP report");
    }
    let parent_id = validate_chain(repo, &revision, &plan, &base)?;
    if !prefix.is_empty() {
        git::validate_path(prefix)?;
    }
    let diagnostic_bytes =
        fs::read(request.diagnostics).map_err(|e| format!("compiler diagnostics: {e}"))?;
    let errors = cargo_errors(&diagnostic_bytes, prefix)?;
    if errors.is_empty() {
        return refuse("no supported unresolved import diagnostics to repair");
    }
    let actions = candidates(repo, prefix, request.analyzer, &errors)?;
    let mut actions = actions.into_iter().collect::<Vec<_>>();
    actions.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let candidate_count = actions.len();
    if candidate_count != 1 {
        return Ok(obj(&[
            ("schema", Value::string("excession.rust-repair.v1")),
            (
                "status",
                Value::string(if candidate_count == 0 {
                    "NO_SUPPORTED_NATIVE_REPAIR"
                } else {
                    "AMBIGUOUS_NATIVE_REPAIRS"
                }),
            ),
            ("source_commit", Value::string(revision)),
            ("parent_plan_id", Value::string(parent_id)),
            ("supported_action_count", Value::number(candidate_count)),
            (
                "verification",
                Value::string(
                    "OPEN_OBLIGATIONS: require explicit review; no source edit was performed.",
                ),
            ),
        ]));
    }
    let (file, title, action) = actions.remove(0);
    let workspace_edit = action.get("edit").ok_or("native quickfix has no edit")?;
    let (edits, outputs) = refactor_rust_analyzer::collect_edits(repo, &revision, workspace_edit)?;
    if !outputs.contains_key(&file) {
        return refuse("native quickfix does not edit the diagnostic source file");
    }
    let plan_identity = obj(&[
        ("parent_plan_id", Value::string(&parent_id)),
        ("source_commit", Value::string(&revision)),
        (
            "compiler_diagnostics_sha256",
            Value::string(digest::sha256(&diagnostic_bytes)),
        ),
        ("action_title", Value::string(&title)),
        ("edits", edits.clone()),
    ]);
    // The language server must never mutate a pinned Git checkout.
    git::clean(repo)?;
    if git::rev(repo, "HEAD")? != revision {
        return refuse("repair source commit moved during native LSP inspection");
    }
    let mut worktree = Value::Null;
    let mut status = "REPAIR_PREVIEW_UNVERIFIED";
    if let Some(path) = request.output {
        if !path.is_absolute() || path.exists() {
            return refuse("repair output must be a new absolute worktree path");
        }
        let parent = path
            .parent()
            .ok_or("missing repair worktree parent")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let source_root = repo.canonicalize().map_err(|e| e.to_string())?;
        if parent.starts_with(&source_root) {
            return refuse("repair output may not be inside the source worktree");
        }
        let target = path.to_str().ok_or("non-UTF-8 repair worktree output")?;
        git::command(
            repo,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
                target,
                &revision,
            ],
        )?;
        let result = (|| -> Result<()> {
            let root = path.canonicalize().map_err(|e| e.to_string())?;
            for (file, body) in &outputs {
                let destination = root.join(file);
                let parent = destination
                    .parent()
                    .ok_or("repair destination has no parent")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if !parent.starts_with(&root) || destination.exists() && !destination.is_file() {
                    return refuse("repair output has unsupported destination");
                }
                if destination.is_symlink() {
                    return refuse("repair output contains a symlink");
                }
                fs::write(&destination, body).map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = git::command(
                repo,
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "worktree",
                    "remove",
                    "--force",
                    target,
                ],
            );
            return Err(error);
        }
        status = "REPAIR_APPLIED_UNVERIFIED";
        worktree = Value::string(target);
    }
    let source = git::entries(repo, &revision)?
        .into_iter()
        .map(|entry| (entry.path, entry.oid))
        .collect::<BTreeMap<_, _>>();
    Ok(obj(&[
        ("schema", Value::string("excession.rust-repair.v1")),
        ("status", Value::string(status)),
        ("source_commit", Value::string(revision)),
        ("parent_plan_id", Value::string(parent_id)),
        (
            "compiler_diagnostics_sha256",
            Value::string(digest::sha256(&diagnostic_bytes)),
        ),
        ("action_title", Value::string(title)),
        ("action_source", Value::string(file)),
        ("plan_id", Value::string(plan_identity.sha256())),
        ("edits", edits),
        (
            "changes",
            Value::array(outputs.iter().map(|(path, bytes)| {
                obj(&[
                    ("path", Value::string(path)),
                    (
                        "original_blob",
                        source.get(path).map(Value::string).unwrap_or(Value::Null),
                    ),
                    (
                        "generated_sha256",
                        Value::string(digest::sha256(bytes.as_bytes())),
                    ),
                ])
            })),
        ),
        ("worktree", worktree),
        (
            "verification",
            Value::string(
                "OPEN_OBLIGATIONS: rerun independent compiler, regression tests, public API compatibility and fresh rust-analyzer SCIP. A native quickfix does not prove equivalence.",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(code: &str, file: &str, line: usize, column: usize) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "reason":"compiler-message",
            "message":{"level":"error","code":{"code":code},
                "spans":[{"is_primary":true,"file_name":file,
                    "line_start":line,"column_start":column}]}
        }))
        .unwrap()
    }
    #[test]
    fn only_unresolved_imports_from_rustc_are_eligible() {
        let errors = cargo_errors(&sample("E0432", "src/index.rs", 10, 40), "core").unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].path, "core/src/index.rs");
        assert_eq!(errors[0].line, 9);
        assert_eq!(errors[0].column, 39);
        assert!(
            cargo_errors(&sample("E0308", "src/index.rs", 10, 40), "core")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn refuse_out_of_workspace_and_malformed_compiler_diagnostics() {
        assert!(cargo_errors(&sample("E0432", "../../secret.rs", 1, 1), "core").is_err());
        assert!(cargo_errors(&sample("E0432", "/tmp/outside.rs", 1, 1), "core").is_err());
        assert!(cargo_errors(b"not json", "core").is_err());
        assert!(cargo_errors(&sample("E0432", "src/index.rs", 0, 0), "core").is_err());
    }
    #[test]
    fn non_ascii_and_surrogate_positions_are_not_guessed() {
        assert_eq!(byte_at_utf16("hi\nok", 1, 1).unwrap(), 4);
        assert!(byte_at_utf16("😀 foo", 0, 1).is_err());
        assert!(byte_at_utf16("hi", 3, 0).is_err());
    }
}
