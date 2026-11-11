//! Source-bound syntax observations from optional, preinstalled Tree-sitter grammars.
//! Syntax facts are candidates for navigation, not verified dependencies or effects.
use crate::adapter;
use crate::git::{self, Entry};
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::path::Path;

const MAX_FILES: usize = 500;
const MAX_SOURCE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 2048;
const BATCH_SIZE: usize = 16;
const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

fn eligible(path: &str) -> bool {
    [
        ".py", ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".rs", ".go", ".c", ".h", ".cc",
        ".cpp", ".hpp", ".java", ".kt", ".kts", ".cs", ".rb", ".php", ".swift", ".scala", ".sh",
        ".ex", ".exs", ".json", ".yaml", ".yml", ".toml", ".sql",
    ]
    .iter()
    .any(|extension| path.ends_with(extension))
}

fn selected(path: &str, roots: &[&str]) -> bool {
    roots
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
}

fn line_count(data: &[u8]) -> usize {
    data.iter().filter(|byte| **byte == b'\n').count() + 1
}

fn positive_line(value: &Value, lines: usize) -> Result<()> {
    let line = value.get("line")?.integer()?;
    if line < 1 || usize::try_from(line).map_err(|_| "invalid line")? > lines {
        return refuse("parser observation line is outside pinned source");
    }
    Ok(())
}

fn validate_structure(value: &Value, data: &[u8], depth: usize, count: &mut usize) -> Result<()> {
    if depth > 64 || *count >= MAX_OBSERVATIONS {
        return refuse("parser structure exceeds depth or observation limit");
    }
    *count += 1;
    if value.entries()?.len() != 6 {
        return refuse("unexpected structure observation shape");
    }
    let kind = value.get("kind")?.str()?;
    let name = value.get("name")?.str()?;
    if kind.is_empty() || kind.len() > 128 || name.len() > 1024 {
        return refuse("invalid parser structure identity");
    }
    let start = usize::try_from(value.get("start_byte")?.integer()?)
        .map_err(|_| "negative parser start offset")?;
    let end = usize::try_from(value.get("end_byte")?.integer()?)
        .map_err(|_| "negative parser end offset")?;
    if start >= end || end > data.len() {
        return refuse("parser structure span is outside pinned source");
    }
    let line = data[..start].iter().filter(|byte| **byte == b'\n').count() + 1;
    positive_line(value, line_count(data))?;
    if usize::try_from(value.get("line")?.integer()?).ok() != Some(line) {
        return refuse("parser line and byte span disagree");
    }
    for child in value.get("children")?.array_ref()? {
        validate_structure(child, data, depth + 1, count)?;
    }
    Ok(())
}

fn validate_observation(observation: &Value, expected: &Entry, data: &[u8]) -> Result<()> {
    if observation.entries()?.len() != 6 || observation.get("path")?.str()? != expected.path {
        return refuse("parser changed source identity or response shape");
    }
    let status = observation.get("status")?.str()?;
    if !matches!(
        status,
        "PARSED"
            | "PARSED_WITH_ERRORS"
            | "UNSUPPORTED_LANGUAGE"
            | "GRAMMAR_NOT_INSTALLED"
            | "RESOURCE_LIMIT"
            | "PARSER_ERROR"
    ) {
        return refuse("unrecognized parser status");
    }
    let language = observation.get("language")?.str()?;
    if language.len() > 64 || (language.is_empty() != (status == "UNSUPPORTED_LANGUAGE")) {
        return refuse("invalid parser language identity");
    }
    let structure = observation.get("structure")?.array_ref()?;
    let imports = observation.get("imports")?.array_ref()?;
    let diagnostics = observation.get("diagnostics")?.array_ref()?;
    if status != "PARSED"
        && status != "PARSED_WITH_ERRORS"
        && (!structure.is_empty() || !imports.is_empty() || !diagnostics.is_empty())
    {
        return refuse("unavailable or failed parser supplied observations");
    }
    let mut count = 0;
    for item in structure {
        validate_structure(item, data, 0, &mut count)?;
    }
    for item in imports {
        count += 1;
        if count > MAX_OBSERVATIONS || item.entries()?.len() != 3 {
            return refuse("invalid parser import shape or count");
        }
        let name = item.get("source")?.str()?;
        if name.len() > 4096 {
            return refuse("parser import name is oversized");
        }
        match item.get("wildcard")? {
            Value::Bool(_) => {}
            _ => return refuse("parser wildcard must be boolean"),
        }
        positive_line(item, line_count(data))?;
    }
    for item in diagnostics {
        count += 1;
        if count > MAX_OBSERVATIONS || item.entries()?.len() != 2 {
            return refuse("invalid parser diagnostic shape or count");
        }
        if item.get("message")?.str()?.len() > 2048 {
            return refuse("parser diagnostic is oversized");
        }
        positive_line(item, line_count(data))?;
    }
    Ok(())
}

fn consume_batch(
    batch: &mut Vec<(Entry, Vec<u8>)>,
    sources: &mut Vec<Value>,
    parser_version: &mut Option<String>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut input = Vec::with_capacity(batch.len());
    for (entry, data) in batch.iter() {
        let content =
            std::str::from_utf8(data).map_err(|_| format!("non-UTF-8 source: {}", entry.path))?;
        input.push(adapter::js_entry(&entry.path, content));
    }
    let parsed = adapter::tree_sitter(&input)?;
    if parsed.entries()?.len() != 2 {
        return refuse("invalid parser response envelope");
    }
    let version = parsed.get("parser_version")?.str()?;
    if version.is_empty() || version.len() > 64 {
        return refuse("parser version missing or invalid");
    }
    if let Some(previous) = parser_version {
        if previous.as_str() != version {
            return refuse("parser version changed during scan");
        }
    } else {
        *parser_version = Some(version.to_owned());
    }
    let records = parsed.get("sources")?.array_ref()?;
    if records.len() != batch.len() {
        return refuse("parser omitted or duplicated source records");
    }
    for ((entry, data), observation) in batch.iter().zip(records) {
        validate_observation(observation, entry, data)?;
        sources.push(obj(&[
            ("source_path", Value::string(&entry.path)),
            ("source_blob", Value::string(&entry.oid)),
            ("source_mode", Value::string(&entry.mode)),
            ("language", observation.get("language")?.clone()),
            ("status", observation.get("status")?.clone()),
            ("structure", observation.get("structure")?.clone()),
            ("imports", observation.get("imports")?.clone()),
            ("diagnostics", observation.get("diagnostics")?.clone()),
        ]));
    }
    batch.clear();
    Ok(())
}

pub fn scan(root: &Path, reference: &str, roots: &[&str]) -> Result<Value> {
    if roots.is_empty() || roots.len() > 32 {
        return refuse("syntax scan requires 1 to 32 roots");
    }
    for root_path in roots {
        git::validate_path(root_path)?;
    }
    git::clean(root)?;
    let commit = git::rev(root, reference)?;
    if git::text(root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("syntax scan reference must be a commit");
    }
    let tree = git::rev(root, &format!("{commit}^{{tree}}"))?;
    let mut selected_files = git::entries(root, &commit)?
        .into_iter()
        .filter(|e| {
            e.kind == "blob"
                && matches!(e.mode.as_str(), "100644" | "100755")
                && selected(&e.path, roots)
                && eligible(&e.path)
                && !e
                    .path
                    .split('/')
                    .any(|part| part == "node_modules" || part == "vendor")
        })
        .collect::<Vec<_>>();
    selected_files.sort_by(|a, b| a.path.cmp(&b.path));
    if selected_files.is_empty() {
        return refuse("no supported source paths under selected roots");
    }
    if selected_files.len() > MAX_FILES {
        return refuse("syntax source selection exceeds 500 files; narrow --roots");
    }
    let mut reader = git::BlobReader::open(root)?;
    let mut total = 0usize;
    let mut batch = Vec::new();
    let mut sources = Vec::new();
    let mut parser_version = None;
    for entry in selected_files {
        let data = reader.read_capped(&entry.oid, MAX_SOURCE_BYTES)?;
        total = total
            .checked_add(data.len())
            .ok_or("source byte count overflow")?;
        if total > MAX_TOTAL_BYTES {
            return refuse("syntax selection exceeds 16 MiB; narrow --roots");
        }
        batch.push((entry, data));
        if batch.len() == BATCH_SIZE {
            consume_batch(&mut batch, &mut sources, &mut parser_version)?;
        }
    }
    consume_batch(&mut batch, &mut sources, &mut parser_version)?;
    reader.finish()?;
    let payload = obj(&[
        ("schema", Value::string("excession.syntax-observations.v1")),
        ("parser", Value::string("tree-sitter-language-pack")),
        (
            "parser_version",
            Value::string(parser_version.unwrap_or_default()),
        ),
        ("completeness", Value::string("CONSERVATIVE_SYNTAX_ONLY")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        (
            "roots",
            Value::array(roots.iter().map(|r| Value::string(*r))),
        ),
        ("source_count", Value::number(sources.len())),
        ("sources", Value::array(sources)),
    ]);
    let mut manifest = payload.entries()?.clone();
    manifest.insert("report_id".into(), Value::string(payload.sha256()));
    Ok(Value::Object(manifest))
}

pub fn audit(root: &Path, file: &Path) -> Result<Value> {
    let bytes = std::fs::read(file).map_err(|e| format!("syntax report read: {e}"))?;
    if bytes.len() > MAX_REPORT_BYTES {
        return refuse("syntax report exceeds 32 MiB");
    }
    let stored = json::parse(&bytes)?;
    crate::preflight::syntax(&stored)?;
    let reference = git::oid(stored.get("source_commit")?.str()?)?;
    let roots = stored
        .get("roots")?
        .array_ref()?
        .iter()
        .map(|v| v.str())
        .collect::<Result<Vec<_>>>()?;
    let reproduced = scan(root, &reference, &roots)?;
    if reproduced.canonical() != stored.canonical() {
        return refuse("syntax report differs from re-parsed Git sources");
    }
    Ok(obj(&[
        ("report_id", reproduced.get("report_id")?.clone()),
        ("source_count", reproduced.get("source_count")?.clone()),
        ("status", Value::string("REPARSED_MATCH")),
    ]))
}
