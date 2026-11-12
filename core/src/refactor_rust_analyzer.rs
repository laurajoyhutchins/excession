//! Apply rust-analyzer's native symbol-aware code actions to pinned Rust source.
//! Excession handles evidence binding, edit validation and isolated publication.
//! The server owns Rust semantics; no second syntax resolver is maintained.
use crate::json::{Value, obj};
use crate::{Result, digest, git, refactor_codegen, refuse, scip_refactor};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_FILES: usize = 8;
const MAX_EDITS: usize = 64;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_ACTIONS: usize = 128;

fn field<'a>(v: &'a Json, k: &str) -> Result<&'a Json> {
    v.get(k)
        .ok_or_else(|| format!("missing rust-analyzer field: {k}"))
}
fn string<'a>(v: &'a Json, k: &str) -> Result<&'a str> {
    field(v, k)?
        .as_str()
        .ok_or_else(|| format!("invalid rust-analyzer string: {k}"))
}
fn number(v: &Json, k: &str) -> Result<usize> {
    usize::try_from(field(v, k)?.as_u64().ok_or("invalid LSP position")?)
        .map_err(|_| "LSP position overflow".to_owned())
}

fn frame(reader: &mut impl BufRead) -> Result<Json> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let size = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if size == 0 {
            return refuse("rust-analyzer LSP stdout reached EOF");
        }
        if header_bytes + size > 8192 {
            return refuse("oversized rust-analyzer LSP header");
        }
        header_bytes += size;
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, content)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            if length.is_some() {
                return refuse("duplicate LSP Content-Length");
            }
            length = Some(
                content
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid LSP Content-Length")?,
            );
        }
    }
    let length = length.ok_or("missing LSP Content-Length")?;
    if length > MAX_FRAME {
        return refuse("rust-analyzer LSP response exceeds 8 MiB");
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid rust-analyzer LSP JSON: {e}"))
}

struct Server {
    process: Child,
    input: ChildStdin,
    incoming: Receiver<Result<Json>>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    next_id: i64,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
impl Server {
    fn start(program: &Path, workspace: &Path) -> Result<Self> {
        let mut process = Command::new(program)
            .arg("lsp-server")
            .current_dir(workspace)
            .env("CARGO_NET_OFFLINE", "true")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("rust-analyzer LSP launch failed: {e}"))?;
        let input = process.stdin.take().ok_or("LSP stdin unavailable")?;
        let stdout = process.stdout.take().ok_or("LSP stdout unavailable")?;
        let mut stderr = process.stderr.take().ok_or("LSP stderr unavailable")?;
        let (sender, incoming) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let message = frame(&mut reader);
                let done = message.is_err();
                if sender.send(message).is_err() || done {
                    break;
                }
            }
        });
        // Always drain stderr. Without this, a chatty language server can
        // fill the OS pipe and deadlock before returning its JSON-RPC result.
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&diagnostics);
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                let count = match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                if let Ok(mut data) = sink.lock() {
                    let available = 8192_usize.saturating_sub(data.len()).min(count);
                    data.extend_from_slice(&buffer[..available]);
                }
            }
        });
        Ok(Self {
            process,
            input,
            incoming,
            diagnostics,
            next_id: 0,
        })
    }
    fn send(&mut self, value: &Json) -> Result<()> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME {
            return refuse("oversized rust-analyzer LSP request");
        }
        write!(self.input, "Content-Length: {}\r\n\r\n", bytes.len())
            .and_then(|()| self.input.write_all(&bytes))
            .and_then(|()| self.input.flush())
            .map_err(|e| format!("write rust-analyzer LSP frame: {e}"))
    }
    fn notify(&mut self, method: &str, params: Json) -> Result<()> {
        self.send(&json!({"jsonrpc":"2.0","method":method,"params":params}))
    }
    fn diagnostics(&self) -> String {
        self.diagnostics.lock().map_or_else(
            |_| "<unavailable>".to_owned(),
            |bytes| String::from_utf8_lossy(&bytes).to_string(),
        )
    }
    fn request(&mut self, method: &str, params: Json) -> Result<Json> {
        self.next_id += 1;
        let wanted = self.next_id;
        self.send(&json!({"jsonrpc":"2.0","id":wanted,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        for _ in 0..2048 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = match self.incoming.recv_timeout(remaining) {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    return refuse(format!(
                        "rust-analyzer {method} protocol error: {error}; {}",
                        self.diagnostics(),
                    ));
                }
                Err(error) => {
                    return refuse(format!(
                        "rust-analyzer {method} deadline/connection failed: {error}; {}",
                        self.diagnostics(),
                    ));
                }
            };
            if message.get("id").and_then(Json::as_i64) == Some(wanted)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    return refuse(format!("rust-analyzer {method} refused: {error}"));
                }
                return Ok(message.get("result").cloned().unwrap_or(Json::Null));
            }
            if let (Some(id), Some(event)) = (message.get("id"), message.get("method")) {
                let method = event.as_str().unwrap_or("");
                let response = match method {
                    "workspace/configuration" => {
                        let count = message
                            .pointer("/params/items")
                            .and_then(Json::as_array)
                            .map_or(0, Vec::len);
                        json!({"jsonrpc":"2.0","id":id,"result":vec![Json::Null; count]})
                    }
                    "workspace/workspaceFolders" => json!({"jsonrpc":"2.0","id":id,"result":[]}),
                    "window/workDoneProgress/create" | "client/registerCapability" => {
                        json!({"jsonrpc":"2.0","id":id,"result":null})
                    }
                    "workspace/applyEdit" => {
                        json!({"jsonrpc":"2.0","id":id,"result":{"applied":false}})
                    }
                    _ => json!({"jsonrpc":"2.0","id":id,
                        "error":{"code":-32601,"message":"unsupported LSP client request"}}),
                };
                self.send(&response)?;
            }
        }
        refuse("rust-analyzer LSP response count exceeded bound")
    }
}

fn uri(path: &Path) -> Result<String> {
    let value = path.to_str().ok_or("non-UTF-8 source path")?;
    if value
        .bytes()
        .any(|c| matches!(c, b'%' | b'#' | b'?' | b' '))
    {
        return refuse("LSP source paths with URI escapes are unsupported");
    }
    Ok(format!("file://{value}"))
}

fn action_allowed(action: &Json) -> bool {
    let title = action.get("title").and_then(Json::as_str).unwrap_or("");
    let kind = action.get("kind").and_then(Json::as_str).unwrap_or("");
    !title.is_empty()
        && !action.get("command").is_some_and(|value| !value.is_null())
        && (kind.starts_with("refactor") || (kind == "quickfix" && title.starts_with("Import ")))
}

fn request_actions(
    analyzer: &Path,
    workspace: &Path,
    relative: &Path,
    source: &str,
    selection: [usize; 4],
    title: Option<&str>,
) -> Result<Vec<Json>> {
    let canonical = workspace.canonicalize().map_err(|e| e.to_string())?;
    let file = canonical.join(relative);
    if file.canonicalize().map_err(|e| e.to_string())? != file {
        return refuse("LSP source must be a normal file in the pinned checkout");
    }
    let root_uri = uri(&canonical)?;
    let file_uri = uri(&file)?;
    let mut server = Server::start(analyzer, &canonical)?;
    server.request(
        "initialize",
        json!({
            "processId":null,
            "rootUri":root_uri,
            "workspaceFolders":[{"uri":root_uri,"name":"source"}],
            "initializationOptions":{
            "cargo":{"buildScripts":{"enable":false}},
            "procMacro":{"enable":false},
            "check":{"enable":false}
        },
        "capabilities":{
                "workspace":{"workspaceEdit":{"documentChanges":true,
                    "resourceOperations":["create"]}},
                "textDocument":{"codeAction":{"codeActionLiteralSupport":{
                    "codeActionKind":{"valueSet":["","quickfix","refactor","refactor.extract",
                        "refactor.rewrite","refactor.inline"]}
                }}}
            }
        }),
    )?;
    server.notify("initialized", json!({}))?;
    server.notify(
        "textDocument/didOpen",
        json!({
            "textDocument":{"uri":file_uri,"languageId":"rust","version":1,"text":source}
        }),
    )?;
    let range = json!({
        "start":{"line":selection[0],"character":selection[1]},
        "end":{"line":selection[2],"character":selection[3]}
    });
    let result = server.request(
        "textDocument/codeAction",
        json!({
            "textDocument":{"uri":file_uri},"range":range,"context":{"diagnostics":[]}
        }),
    )?;
    let values = if result.is_null() {
        Vec::new()
    } else {
        result
            .as_array()
            .ok_or("rust-analyzer codeAction is not an array")?
            .clone()
    };
    if values.len() > MAX_ACTIONS {
        return refuse("too many rust-analyzer code actions");
    }
    let mut resolved = Vec::new();
    for candidate in values {
        if !action_allowed(&candidate) {
            continue;
        }
        // Some rust-analyzer versions advertise lazy edits. Ask the same
        // server to resolve the action, never fabricate text modifications.
        let candidate = if title == candidate.get("title").and_then(Json::as_str)
            && candidate.get("edit").is_none()
            && candidate.get("data").is_some()
        {
            let original_title = string(&candidate, "title")?.to_owned();
            let updated = server.request("codeAction/resolve", candidate)?;
            if string(&updated, "title")? != original_title {
                return refuse("resolved rust-analyzer code action changed identity");
            }
            updated
        } else {
            candidate
        };
        if action_allowed(&candidate) {
            resolved.push(candidate);
        }
    }
    Ok(resolved)
}

fn position(source: &str, line: usize, utf16: usize) -> Result<usize> {
    let mut start = 0;
    for _ in 0..line {
        let newline = source[start..].find('\n').ok_or("LSP line beyond source")?;
        start += newline + 1;
    }
    let end = source[start..]
        .find('\n')
        .map_or(source.len(), |n| start + n);
    let mut units = 0;
    for (offset, ch) in source[start..end].char_indices() {
        if units == utf16 {
            return Ok(start + offset);
        }
        units += ch.len_utf16();
        if units > utf16 {
            return refuse("LSP character splits UTF-16 surrogate pair");
        }
    }
    if units == utf16 {
        Ok(end)
    } else {
        refuse("LSP character offset outside pinned source")
    }
}

fn candidate_matches(report: &Value, id: &str, path: &str, blob: &str) -> Result<()> {
    if report.get("indexer")?.get("name")?.str()? != "rust-analyzer" {
        return refuse("code actions require rust-analyzer SCIP evidence");
    }
    let chosen = report
        .get("candidates")?
        .array_ref()?
        .iter()
        .find(|candidate| {
            candidate
                .get("candidate_id")
                .ok()
                .and_then(|v| v.str().ok())
                == Some(id)
        })
        .ok_or("unknown candidate ID")?;
    if chosen.get("kind")?.str()? != "reciprocal_symbol_references" {
        return refuse("code action must be attached to a reciprocal symbol candidate");
    }
    if !chosen.get("sources")?.array_ref()?.iter().any(|source| {
        source.get("path").ok().and_then(|x| x.str().ok()) == Some(path)
            && source.get("blob").ok().and_then(|x| x.str().ok()) == Some(blob)
            && source.get("language").ok().and_then(|x| x.str().ok()) == Some("rust")
    }) {
        return refuse("LSP source is not bound to the audited candidate");
    }
    if report
        .get("triage")?
        .get("likely_structural_module_candidate_ids")?
        .array_ref()?
        .iter()
        .any(|value| value.str().ok() == Some(id))
    {
        return refuse("structural-only SCIP candidate is not an approved refactor target");
    }
    Ok(())
}

struct Changed {
    original: String,
    before: Option<String>,
    changes: Vec<(usize, usize, String)>,
    created: bool,
}

fn path_for_uri(root: &Path, value: &str) -> Result<String> {
    let original = value
        .strip_prefix("file://")
        .ok_or("LSP edit is not a file URI")?;
    if original.contains('%') || original.contains('#') || original.contains('?') {
        return refuse("LSP URI escapes are unsupported");
    }
    let full = Path::new(original);
    let relative = full
        .strip_prefix(root)
        .map_err(|_| "LSP edit outside source checkout")?;
    let result = relative.to_str().ok_or("non-UTF-8 LSP source path")?;
    git::validate_path(result)?;
    if !result.ends_with(".rs") || !result.split('/').any(|part| part == "src") {
        return refuse("rust-analyzer edits must affect only Rust source under src/");
    }
    Ok(result.to_owned())
}

fn collect_edits(repo: &Path, rev: &str, edit: &Json) -> Result<(Value, BTreeMap<String, String>)> {
    let root = repo.canonicalize().map_err(|e| e.to_string())?;
    let tracked = git::entries(repo, rev)?
        .into_iter()
        .filter(|entry| entry.kind == "blob" && entry.mode == "100644")
        .map(|entry| (entry.path.clone(), entry.oid))
        .collect::<BTreeMap<_, _>>();
    let mut changed: BTreeMap<String, Changed> = BTreeMap::new();
    let changes = edit.get("changes");
    let document_changes = edit.get("documentChanges");
    if changes.is_some() && document_changes.is_some() {
        return refuse("ambiguous rust-analyzer WorkspaceEdit format");
    }
    if let Some(operations) = document_changes {
        let list = operations.as_array().ok_or("invalid LSP documentChanges")?;
        if list.len() > MAX_EDITS {
            return refuse("too many workspace operations");
        }
        for op in list {
            if op.get("kind").is_some() {
                if string(op, "kind")? != "create" {
                    return refuse("LSP rename/delete file operations are unsupported");
                }
                if let Some(options) = op.get("options")
                    && !options.is_null()
                    && (options
                        .get("overwrite")
                        .is_some_and(|v| v.as_bool() != Some(false))
                        || options
                            .get("ignoreIfExists")
                            .is_some_and(|v| v.as_bool() != Some(false)))
                {
                    return refuse("LSP overwrite/ignore create options are unsupported");
                }
                let path = path_for_uri(&root, string(op, "uri")?)?;
                if tracked.contains_key(&path) || changed.contains_key(&path) {
                    return refuse("LSP create target exists or is duplicated");
                }
                changed.insert(
                    path,
                    Changed {
                        original: String::new(),
                        before: None,
                        changes: vec![],
                        created: true,
                    },
                );
            }
        }
        for op in list {
            if op.get("kind").is_some() {
                continue;
            }
            let doc = field(op, "textDocument")?;
            if !doc
                .get("version")
                .is_none_or(|v| v.is_null() || v.as_i64() == Some(1))
            {
                return refuse("stale LSP text document version");
            }
            let path = path_for_uri(&root, string(doc, "uri")?)?;
            append_edits(repo, &tracked, &mut changed, &path, field(op, "edits")?)?;
        }
    } else if let Some(map) = changes {
        let files = map.as_object().ok_or("invalid LSP changes map")?;
        for (uri, items) in files {
            let path = path_for_uri(&root, uri)?;
            append_edits(repo, &tracked, &mut changed, &path, items)?;
        }
    } else {
        return refuse("rust-analyzer returned no supported WorkspaceEdit");
    }
    if changed.is_empty() || changed.len() > MAX_FILES {
        return refuse("unsupported rust-analyzer workspace edit file count");
    }
    let count: usize = changed.values().map(|c| c.changes.len()).sum();
    if count == 0 || count > MAX_EDITS {
        return refuse("rust-analyzer returned no edits or too many edits");
    }
    let mut operations = Vec::new();
    let mut outputs = BTreeMap::new();
    for (path, file) in &changed {
        if file.created && file.before.is_some() {
            return refuse("invalid created LSP document");
        }
        let mut instructions = Vec::new();
        for (start, end, replacement) in &file.changes {
            instructions.push(refactor_codegen::edit_operation(
                path,
                file.before.as_deref(),
                *start,
                *end,
                replacement,
            ));
        }
        let data = refactor_codegen::replay_edits(
            &file.original,
            &Value::array(instructions.clone()),
            path,
        )?;
        if data.len() > MAX_FILE_BYTES {
            return refuse("generated Rust source exceeds file size bound");
        }
        outputs.insert(path.clone(), data);
        operations.extend(instructions);
    }
    Ok((Value::array(operations), outputs))
}

fn append_edits(
    repo: &Path,
    tracked: &BTreeMap<String, String>,
    changed: &mut BTreeMap<String, Changed>,
    path: &str,
    items: &Json,
) -> Result<()> {
    let list = items.as_array().ok_or("invalid LSP TextEdit list")?;
    if list.is_empty() {
        return refuse("empty LSP text edit");
    }
    if !changed.contains_key(path) {
        let oid = tracked
            .get(path)
            .ok_or("LSP edit targets untracked file without create")?;
        let mut reader = git::BlobReader::open(repo)?;
        let data = reader.read_capped(oid, MAX_FILE_BYTES)?;
        reader.finish()?;
        let original = String::from_utf8(data).map_err(|_| "source is not UTF-8")?;
        changed.insert(
            path.to_owned(),
            Changed {
                original,
                before: Some(oid.clone()),
                changes: Vec::new(),
                created: false,
            },
        );
    }
    let target = changed.get_mut(path).ok_or("missing edited source")?;
    for item in list {
        if item.get("annotationId").is_some() {
            return refuse("annotated LSP source edit is unsupported");
        }
        let range = field(item, "range")?;
        let from = field(range, "start")?;
        let to = field(range, "end")?;
        let start = position(
            &target.original,
            number(from, "line")?,
            number(from, "character")?,
        )?;
        let end = position(
            &target.original,
            number(to, "line")?,
            number(to, "character")?,
        )?;
        if start > end {
            return refuse("reversed LSP source edit range");
        }
        let replacement = string(item, "newText")?;
        if replacement.len() > MAX_FILE_BYTES {
            return refuse("LSP replacement exceeds file size bound");
        }
        target.changes.push((start, end, replacement.to_owned()));
    }
    Ok(())
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub index: &'a Path,
    pub report: &'a Path,
    pub candidate: &'a str,
    pub source: &'a str,
    pub source_prefix: Option<&'a str>,
    pub selection: [usize; 4],
    pub analyzer: &'a Path,
    pub title: Option<&'a str>,
    pub output: Option<&'a Path>,
}

pub fn execute(request: &Request<'_>) -> Result<Value> {
    let repo = request.repo;
    git::clean(repo)?;
    let revision = git::rev(repo, request.revision)?;
    if git::rev(repo, "HEAD")? != revision {
        return refuse("rust-analyzer requires a clean checkout at the selected Git commit");
    }
    git::validate_path(request.source)?;
    let source_entry = git::entry_at(repo, &revision, request.source)?;
    if source_entry.mode != "100644" || !request.source.ends_with(".rs") {
        return refuse("rust-analyzer requires a tracked regular Rust source");
    }
    scip_refactor::audit_with_prefix(
        repo,
        &revision,
        request.index,
        request.report,
        request.source_prefix,
    )?;
    let report = crate::json::parse(&fs::read(request.report).map_err(|e| e.to_string())?)?;
    candidate_matches(
        &report,
        request.candidate,
        request.source,
        &source_entry.oid,
    )?;
    let mut reader = git::BlobReader::open(repo)?;
    let data = reader.read_capped(&source_entry.oid, MAX_FILE_BYTES)?;
    reader.finish()?;
    let source = String::from_utf8(data).map_err(|_| "non-UTF-8 Rust source")?;
    let canonical = repo.canonicalize().map_err(|e| e.to_string())?;
    let file = canonical.join(request.source);
    if fs::read(&file).map_err(|e| e.to_string())? != source.as_bytes() {
        return refuse("working Rust source differs from pinned Git blob");
    }
    let crate_root = if let Some(prefix) = request.source_prefix {
        git::validate_path(prefix)?;
        canonical.join(prefix)
    } else {
        canonical.clone()
    };
    if !crate_root.join("Cargo.toml").is_file() {
        return refuse("rust-analyzer workspace must have Cargo.toml");
    }
    // Validate UTF-16 offsets before asking the provider to interpret them.
    let [sl, sc, el, ec] = request.selection;
    let start = position(&source, sl, sc)?;
    let end = position(&source, el, ec)?;
    if start > end {
        return refuse("reversed rust-analyzer selection");
    }
    let relative = file
        .strip_prefix(&crate_root)
        .map_err(|_| "source outside Cargo workspace")?;
    let actions = request_actions(
        request.analyzer,
        &crate_root,
        relative,
        &source,
        request.selection,
        request.title,
    )?;
    if request.title.is_none() {
        if request.output.is_some() {
            return refuse("cannot apply an unspecified rust-analyzer action");
        }
        let mut names = actions
            .into_iter()
            .map(|action| {
                (
                    string(&action, "title").unwrap_or("").to_owned(),
                    action
                        .get("kind")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        return Ok(obj(&[
            ("schema", Value::string("excession.rust-assist.v1")),
            ("status", Value::string("AVAILABLE_UNVERIFIED_ACTIONS")),
            ("source_commit", Value::string(&revision)),
            ("candidate_id", Value::string(request.candidate)),
            (
                "actions",
                Value::array(names.into_iter().map(|(name, kind)| {
                    obj(&[
                        ("title", Value::string(name)),
                        ("kind", Value::string(kind)),
                    ])
                })),
            ),
        ]));
    }
    let title = request.title.ok_or("missing action title")?;
    let selected = actions
        .into_iter()
        .filter(|action| action.get("title").and_then(Json::as_str) == Some(title))
        .collect::<Vec<_>>();
    if selected.len() != 1 {
        return refuse("rust-analyzer action title unavailable or ambiguous");
    }
    let action = &selected[0];
    if action.get("command").is_some_and(|value| !value.is_null()) {
        return refuse("code actions requiring a command are unsupported");
    }
    let edit = field(action, "edit")?;
    let (operations, outputs) = collect_edits(&canonical, &revision, edit)?;
    if !outputs.contains_key(request.source) {
        return refuse("code action did not modify the evidence-bound source");
    }
    let plan = obj(&[
        ("source_commit", Value::string(&revision)),
        ("candidate_id", Value::string(request.candidate)),
        ("scip_report_id", report.get("report_id")?.clone()),
        ("action_title", Value::string(title)),
        ("edits", operations.clone()),
    ]);
    let plan_id = plan.sha256();
    // Querying the server must not alter the original checkout.
    git::clean(repo)?;
    if git::rev(repo, "HEAD")? != revision {
        return refuse("source revision changed during LSP request");
    }
    let mut worktree = Value::Null;
    let mut status = "UNVERIFIED_CODE_ACTION_PREVIEW";
    if let Some(output) = request.output {
        if !output.is_absolute() || output.exists() {
            return refuse("output must be a new absolute path");
        }
        let parent = output
            .parent()
            .ok_or("output has no parent")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if parent.starts_with(&canonical) {
            return refuse("code action worktree cannot be nested in source checkout");
        }
        let target = output.to_str().ok_or("non-UTF-8 worktree path")?;
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
            let root = output.canonicalize().map_err(|e| e.to_string())?;
            for (path, body) in &outputs {
                let destination = root.join(path);
                let parent = destination
                    .parent()
                    .ok_or("missing generated file parent")?
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if !parent.starts_with(&root) {
                    return refuse("generated source path escapes output worktree");
                }
                if destination.exists() && !destination.is_file() {
                    return refuse("generated source target is not a file");
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
        status = "APPLIED_UNVERIFIED";
        worktree = Value::string(target);
    }
    let tracked = git::entries(repo, &revision)?
        .into_iter()
        .map(|entry| (entry.path, entry.oid))
        .collect::<BTreeMap<_, _>>();
    let changes = outputs
        .iter()
        .map(|(path, body)| {
            obj(&[
                ("path", Value::string(path)),
                (
                    "original_blob",
                    tracked.get(path).map(Value::string).unwrap_or(Value::Null),
                ),
                (
                    "generated_sha256",
                    Value::string(digest::sha256(body.as_bytes())),
                ),
            ])
        })
        .collect::<Vec<_>>();
    Ok(obj(&[
        ("schema", Value::string("excession.rust-assist.v1")),
        ("status", Value::string(status)),
        ("source_commit", Value::string(revision)),
        ("candidate_id", Value::string(request.candidate)),
        ("scip_report_id", report.get("report_id")?.clone()),
        ("action_title", Value::string(title)),
        ("action_kind", Value::string(string(action, "kind")?)),
        ("plan_id", Value::string(plan_id)),
        ("edits", operations),
        ("changes", Value::array(changes)),
        ("worktree", worktree),
        (
            "verification",
            Value::string(
                "UNRESOLVED: verify generated Rust with compiler, tests, public API checks and a fresh SCIP index; LSP edits are not a behavioral proof.",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_lsp_positions_are_utf16_not_utf8() {
        let text = "α😀z\nhello\n";
        assert_eq!(position(text, 0, 0).unwrap(), 0);
        assert_eq!(position(text, 0, 1).unwrap(), 2);
        assert_eq!(position(text, 0, 3).unwrap(), 6);
        assert!(position(text, 0, 2).is_err());
        assert_eq!(position(text, 1, 5).unwrap(), 13);
        assert!(position(text, 4, 0).is_err());
    }
    #[test]
    fn refuse_edits_outside_src_or_with_escaped_uris() {
        let root = Path::new("/tmp/project");
        assert!(path_for_uri(root, "file:///tmp/project/src/main.rs").is_ok());
        assert!(path_for_uri(root, "file:///tmp/elsewhere/src/main.rs").is_err());
        assert!(path_for_uri(root, "file:///tmp/project/Cargo.toml").is_err());
        assert!(path_for_uri(root, "file:///tmp/project/src/space%20x.rs").is_err());
    }
    #[test]
    fn only_explicit_refactors_and_import_quickfixes_are_selectable() {
        assert!(action_allowed(
            &json!({"title":"Extract module","kind":"refactor.extract"})
        ));
        assert!(action_allowed(
            &json!({"title":"Import foo","kind":"quickfix"})
        ));
        assert!(!action_allowed(
            &json!({"title":"Run build","kind":"quickfix"})
        ));
        assert!(!action_allowed(
            &json!({"title":"Run shell","kind":"refactor","command":"x"})
        ));
    }
}
