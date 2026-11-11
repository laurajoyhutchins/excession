//! Reproducible, conservative file candidates from source-bound syntax imports.
//! No compiler, build-system or runtime claim is derived from candidate edges.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse, syntax};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;
const MAX_RELATIONS: usize = 10_000;
const MAX_GO_MODULES: usize = 32;
const MAX_GO_MOD_BYTES: usize = 64 * 1024;

struct Source<'a> {
    path: &'a str,
    blob: &'a str,
    language: &'a str,
    imports: &'a [Value],
}

struct GoModule {
    root: String,
    name: String,
    path: String,
    blob: String,
}

fn paths(report: &Value) -> Result<BTreeMap<String, Source<'_>>> {
    let mut out = BTreeMap::new();
    for source in report.get("sources")?.array_ref()? {
        let path = source.get("source_path")?.str()?;
        let blob = source.get("source_blob")?.str()?;
        let status = source.get("status")?.str()?;
        if !matches!(status, "PARSED" | "PARSED_WITH_ERRORS") {
            continue; // An unavailable parser supplies no trustworthy imports.
        }
        if out
            .insert(
                path.to_owned(),
                Source {
                    path,
                    blob,
                    language: source.get("language")?.str()?,
                    imports: source.get("imports")?.array_ref()?,
                },
            )
            .is_some()
        {
            return refuse("duplicated source path in syntax report");
        }
    }
    Ok(out)
}

fn module_name(contents: &str) -> Option<&str> {
    let mut found = None;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix("module ")
            .or_else(|| line.strip_prefix("module\t"))
        {
            let name = name.trim().trim_matches('"');
            if found.is_some()
                || name.is_empty()
                || name.contains(char::is_whitespace)
                || name.starts_with('/')
                || name.contains('\\')
                || name
                    .split('/')
                    .any(|part| part == "." || part == ".." || part.is_empty())
            {
                return None;
            }
            found = Some(name);
        }
    }
    found
}

fn go_modules(root: &Path, revision: &str) -> Result<Vec<GoModule>> {
    let mut selected = git::entries(root, revision)?
        .into_iter()
        .filter(|e| {
            e.kind == "blob"
                && e.mode == "100644"
                && (e.path == "go.mod" || e.path.ends_with("/go.mod"))
        })
        .collect::<Vec<_>>();
    if selected.len() > MAX_GO_MODULES {
        return refuse("more than 32 go.mod files; narrow the analyzed repository");
    }
    selected.sort_by(|a, b| a.path.cmp(&b.path));
    let mut reader = git::BlobReader::open(root)?;
    let mut result = Vec::new();
    for entry in selected {
        let body = reader.read_capped(&entry.oid, MAX_GO_MOD_BYTES)?;
        let contents = std::str::from_utf8(&body).map_err(|_| "non-UTF-8 go.mod")?;
        if let Some(name) = module_name(contents) {
            result.push(GoModule {
                root: entry
                    .path
                    .strip_suffix("go.mod")
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .into(),
                name: name.into(),
                path: entry.path,
                blob: entry.oid,
            });
        }
    }
    reader.finish()?;
    Ok(result)
}

fn join_relative(base: &str, reference: &str) -> Option<String> {
    if !(reference.starts_with("./") || reference.starts_with("../")) {
        return None;
    }
    if reference.contains('\\') || reference.contains('\0') {
        return None;
    }
    let mut segments = base
        .rsplit_once('/')
        .map(|x| x.0)
        .unwrap_or("")
        .split('/')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>();
    for part in reference.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                segments.pop()?;
            }
            v => segments.push(v),
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments.join("/"))
    }
}

fn relative_candidates(
    specifier: &str,
    source: &str,
    files: &BTreeMap<String, Source<'_>>,
) -> Vec<String> {
    let Some(base) = join_relative(source, specifier) else {
        return Vec::new();
    };
    let extensions = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json"];
    let explicit = extensions.iter().any(|ext| base.ends_with(ext));
    let mut paths = Vec::new();
    if files.contains_key(&base) {
        paths.push(base.clone());
    }
    if !explicit {
        for ext in extensions {
            let candidate = format!("{base}{ext}");
            if files.contains_key(&candidate) {
                paths.push(candidate);
            }
            let index = format!("{base}/index{ext}");
            if files.contains_key(&index) {
                paths.push(index);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn python_candidates(
    specifier: &str,
    source: &str,
    files: &BTreeMap<String, Source<'_>>,
) -> Vec<String> {
    let (module, relative) = if specifier.starts_with('.') {
        let dots = specifier.bytes().take_while(|ch| *ch == b'.').count();
        let tail = &specifier[dots..];
        let mut parts = source.split('/').collect::<Vec<_>>();
        parts.pop();
        if dots > parts.len() + 1 {
            return Vec::new();
        }
        for _ in 1..dots {
            parts.pop();
        }
        if !tail.is_empty() {
            parts.extend(tail.split('.'));
        }
        (parts.join("/"), true)
    } else {
        (specifier.replace('.', "/"), false)
    };
    if module.is_empty()
        || module.contains('\\')
        || module.contains('\0')
        || module
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Vec::new();
    }
    let suffixes = [format!("{module}.py"), format!("{module}/__init__.py")];
    let mut matches = Vec::new();
    for path in files.keys() {
        if !path.ends_with(".py") {
            continue;
        }
        if suffixes
            .iter()
            .any(|suffix| path == suffix || (!relative && path.ends_with(&format!("/{suffix}"))))
        {
            matches.push(path.clone());
        }
    }
    matches
}

fn rust_candidates(
    specifier: &str,
    source: &str,
    files: &BTreeMap<String, Source<'_>>,
) -> Vec<String> {
    let Some(rest) = specifier.strip_prefix("crate::") else {
        return Vec::new();
    };
    let module = rest.split("::").next().unwrap_or("");
    if module.is_empty()
        || !module
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Vec::new();
    }
    let Some((prefix, _)) = source.split_once("src/") else {
        return Vec::new();
    };
    if !(files.contains_key(&format!("{prefix}src/lib.rs"))
        || files.contains_key(&format!("{prefix}src/main.rs")))
    {
        return Vec::new();
    }
    let choices = [
        format!("{prefix}src/{module}.rs"),
        format!("{prefix}src/{module}/mod.rs"),
    ];
    choices
        .into_iter()
        .filter(|path| files.contains_key(path))
        .collect()
}

fn go_candidates<'a>(
    specifier: &str,
    source: &str,
    modules: &'a [GoModule],
    files: &BTreeMap<String, Source<'_>>,
) -> (Vec<String>, Option<&'a GoModule>) {
    let module = modules
        .iter()
        .filter(|m| m.root.is_empty() || source.starts_with(&format!("{}/", m.root)))
        .max_by_key(|m| m.root.len());
    let Some(module) = module else {
        return (Vec::new(), None);
    };
    let local = if specifier == module.name {
        ""
    } else if let Some(path) = specifier.strip_prefix(&format!("{}/", module.name)) {
        path
    } else {
        return (Vec::new(), Some(module));
    };
    if local.contains('\\')
        || local.contains('\0')
        || local
            .split('/')
            .any(|piece| piece == ".." || piece == "." || (piece.is_empty() && !local.is_empty()))
    {
        return (Vec::new(), Some(module));
    }
    let directory = if module.root.is_empty() {
        local.to_owned()
    } else if local.is_empty() {
        module.root.clone()
    } else {
        format!("{}/{local}", module.root)
    };
    let candidate = files
        .iter()
        .filter(|(path, item)| {
            item.language == "go"
                && path.rsplit_once('/').map(|p| p.0).unwrap_or("") == directory
                && path.ends_with(".go")
                && !path.ends_with("_test.go")
        })
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    (candidate, Some(module))
}

/// The grammar-pack extractor returns statement text for some languages
/// instead of a normalized import path. Only accept small explicit shapes.
fn quoted_literal(statement: &str) -> Option<String> {
    let mut quote = None;
    let mut begin = 0;
    let mut values = Vec::new();
    for (at, ch) in statement.char_indices() {
        if let Some(q) = quote {
            if ch == '\\' {
                return None; // Escapes require language-native literal decoding.
            }
            if ch == q {
                let value = &statement[begin..at];
                if value.is_empty() || value.contains('\n') || value.contains('\r') {
                    return None;
                }
                values.push(value.to_owned());
                quote = None;
            }
        } else if ch == '\'' || ch == '"' || ch == char::from(96) {
            quote = Some(ch);
            begin = at + ch.len_utf8();
        }
    }
    if quote.is_some() || values.len() != 1 {
        return None;
    }
    values.pop()
}

fn import_target(language: &str, statement: &str) -> Option<String> {
    let text = statement.trim();
    match language {
        "javascript" | "typescript" | "tsx" => {
            if text.starts_with("import ") || text.starts_with("export ")
                || text.starts_with("import\n") || text.starts_with("export\n")
            {
                quoted_literal(text)
            } else if text.starts_with("./") || text.starts_with("../") {
                Some(text.into())
            } else {
                None
            }
        }
        "go" => {
            let value = if let Some(body) = text.strip_prefix("import ") {
                body.trim()
            } else {
                text
            };
            if value.starts_with('"') || value.starts_with(char::from(96)) {
                quoted_literal(value)
            } else if !value.contains(char::is_whitespace) && !value.contains('"') {
                Some(value.into())
            } else {
                None
            }
        }
        "rust" => {
            let value = text.strip_prefix("pub use ")
                .or_else(|| text.strip_prefix("use "))
                .unwrap_or(text)
                .trim_end_matches(';').trim();
            if value.starts_with("crate::")
                && value.split("::").all(|segment| {
                    !segment.is_empty() &&
                    segment.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                })
            {
                Some(value.to_owned())
            } else {
                None
            }
        }
        "python" => {
            if let Some(rest) = text.strip_prefix("from ") {
                let (module, _) = rest.split_once(" import ")?;
                if !module.is_empty() && !module.contains(char::is_whitespace) {
                    Some(module.into())
                } else {
                    None
                }
            } else {
                let rest = text.strip_prefix("import ").unwrap_or(text).trim();
                let module = rest.split_whitespace().next()?;
                if module.contains(',') || rest.contains(',') {
                    None
                } else {
                    Some(module.into())
                }
            }
        }
        _ => None,
    }
}

fn edge(source: &Source<'_>, target: &Source<'_>, spec: &str, line: i64, rule: &str) -> Value {
    obj(&[
        ("source_path", Value::string(source.path)),
        ("source_blob", Value::string(source.blob)),
        ("target_path", Value::string(target.path)),
        ("target_blob", Value::string(target.blob)),
        ("language", Value::string(source.language)),
        ("specifier", Value::string(spec)),
        ("line", Value::number(line)),
        ("resolution", Value::string(rule)),
    ])
}

fn unresolved(source: &Source<'_>, spec: &str, line: i64, reason: &str) -> Value {
    obj(&[
        ("source_path", Value::string(source.path)),
        ("source_blob", Value::string(source.blob)),
        ("language", Value::string(source.language)),
        ("specifier", Value::string(spec)),
        ("line", Value::number(line)),
        ("reason", Value::string(reason)),
    ])
}

pub fn candidates(root: &Path, syntax_report: &Path) -> Result<Value> {
    // Replay the exact parser and Git sources before resolving anything.
    syntax::audit(root, syntax_report)?;
    let raw = std::fs::read(syntax_report).map_err(|e| format!("syntax read: {e}"))?;
    if raw.len() > MAX_REPORT_BYTES {
        return refuse("syntax input exceeds 32 MiB");
    }
    let report = json::parse(&raw)?;
    let revision = git::oid(report.get("source_commit")?.str()?)?;
    let files = paths(&report)?;
    let modules = go_modules(root, &revision)?;
    let mut candidate_edges = Vec::new();
    let mut unresolved_imports = Vec::new();
    let mut skipped_sources = Vec::new();
    for source in report.get("sources")?.array_ref()? {
        let status = source.get("status")?.str()?;
        let language = source.get("language")?.str()?;
        if status != "PARSED" && status != "PARSED_WITH_ERRORS" {
            skipped_sources.push(obj(&[
                ("source_path", source.get("source_path")?.clone()),
                ("status", Value::string(status)),
            ]));
        } else if !matches!(
            language,
            "python" | "javascript" | "typescript" | "tsx" | "go" | "rust"
        ) {
            skipped_sources.push(obj(&[
                ("source_path", source.get("source_path")?.clone()),
                ("status", Value::string("NO_RESOLVER_FOR_LANGUAGE")),
            ]));
        }
    }
    let mut seen_edges = BTreeSet::new();
    for source in files.values() {
        for import in source.imports {
            if candidate_edges.len() + unresolved_imports.len() >= MAX_RELATIONS {
                return refuse("resolution exceeds 10,000 observations; narrow --roots");
            }
            let raw_spec = import.get("source")?.str()?;
            let line = import.get("line")?.integer()?;
            let Some(spec) = import_target(source.language, raw_spec) else {
                unresolved_imports.push(unresolved(
                    source, raw_spec, line, "UNSUPPORTED_IMPORT_SHAPE",
                ));
                continue;
            };
            let spec = spec.as_str();
            let (targets, rule, fallback) = match source.language {
                "javascript" | "typescript" | "tsx" => (
                    relative_candidates(spec, source.path, &files),
                    "RELATIVE_PATH_CANDIDATE",
                    if spec.starts_with('.') {
                        "NO_UNIQUE_TRACKED_PATH"
                    } else {
                        "BARE_OR_ALIAS_IMPORT"
                    },
                ),
                "python" => (
                    python_candidates(spec, source.path, &files),
                    "PYTHON_MODULE_CANDIDATE",
                    "NO_UNIQUE_TRACKED_MODULE",
                ),
                "rust" => (
                    rust_candidates(spec, source.path, &files),
                    "RUST_CRATE_PATH_CANDIDATE",
                    "UNRESOLVED_OR_NONLOCAL_RUST_IMPORT",
                ),
                "go" => {
                    let (targets, module) = go_candidates(spec, source.path, &modules, &files);
                    (
                        targets,
                        "GO_MODULE_PACKAGE_CANDIDATE",
                        if module.is_some() {
                            "EXTERNAL_OR_UNTRACKED_GO_PACKAGE"
                        } else {
                            "GO_MODULE_NOT_FOUND"
                        },
                    )
                }
                _ => {
                    // Do not substitute a generic path guess for a language resolver.
                    continue;
                }
            };
            if targets.len() == 1 || (source.language == "go" && !targets.is_empty()) {
                for path in targets {
                    if path == source.path {
                        continue;
                    }
                    if seen_edges.insert((
                        source.path.to_owned(),
                        path.clone(),
                        line,
                        spec.to_owned(),
                    )) {
                        let target = files
                            .get(&path)
                            .ok_or("missing tracked resolution target")?;
                        candidate_edges.push(edge(source, target, spec, line, rule));
                        if candidate_edges.len() > MAX_RELATIONS {
                            return refuse("too many resolution candidates");
                        }
                    }
                }
            } else {
                unresolved_imports.push(unresolved(
                    source,
                    spec,
                    line,
                    if targets.len() > 1 {
                        "AMBIGUOUS_TRACKED_TARGETS"
                    } else {
                        fallback
                    },
                ));
            }
        }
    }
    candidate_edges.sort_by_key(Value::canonical);
    unresolved_imports.sort_by_key(Value::canonical);
    skipped_sources.sort_by_key(Value::canonical);
    let metadata = modules
        .iter()
        .map(|module| {
            obj(&[
                ("source_path", Value::string(&module.path)),
                ("source_blob", Value::string(&module.blob)),
                ("module", Value::string(&module.name)),
            ])
        })
        .collect::<Vec<_>>();
    let payload = obj(&[
        ("schema", Value::string("excession.syntax-dependencies.v1")),
        ("source_commit", report.get("source_commit")?.clone()),
        ("source_tree", report.get("source_tree")?.clone()),
        ("syntax_report_id", report.get("report_id")?.clone()),
        (
            "completeness",
            Value::string("CONSERVATIVE_CANDIDATES_ONLY"),
        ),
        ("candidate_count", Value::number(candidate_edges.len())),
        ("unresolved_count", Value::number(unresolved_imports.len())),
        ("candidate_edges", Value::array(candidate_edges)),
        ("unresolved_imports", Value::array(unresolved_imports)),
        ("skipped_sources", Value::array(skipped_sources)),
        ("metadata", Value::array(metadata)),
    ]);
    let mut body = payload.entries()?.clone();
    body.insert("resolution_id".into(), Value::string(payload.sha256()));
    Ok(Value::Object(body))
}

pub fn audit(root: &Path, syntax_report: &Path, resolution_report: &Path) -> Result<Value> {
    let bytes =
        std::fs::read(resolution_report).map_err(|e| format!("resolution report read: {e}"))?;
    if bytes.len() > MAX_REPORT_BYTES {
        return refuse("resolution report exceeds 32 MiB");
    }
    let stored = json::parse(&bytes)?;
    let regenerated = candidates(root, syntax_report)?;
    if regenerated.canonical() != stored.canonical() {
        return refuse("dependency candidates differ from independently regenerated report");
    }
    Ok(obj(&[
        ("resolution_id", regenerated.get("resolution_id")?.clone()),
        (
            "candidate_count",
            regenerated.get("candidate_count")?.clone(),
        ),
        ("status", Value::string("REGENERATED_MATCH")),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_never_escape_repository() {
        assert_eq!(join_relative("src/a.ts", "../b"), Some("b".into()));
        assert_eq!(join_relative("src/a.ts", "../../b"), None);
        assert_eq!(join_relative("src/a.ts", r".\b"), None);
        assert_eq!(join_relative("src/a.ts", "/etc/passwd"), None);
    }

    #[test]
    fn grammar_pack_import_statements_are_normalized_only_when_unambiguous() {
        assert_eq!(import_target("typescript", "import {x} from './util';"), Some("./util".into()));
        assert_eq!(import_target("typescript", "import './side_effect';"), Some("./side_effect".into()));
        assert_eq!(import_target("typescript", "import x from 'a' with {type: 'json'};"), None);
        assert_eq!(import_target("go", "\"example.org/pkg\""), Some("example.org/pkg".into()));
        assert_eq!(import_target("rust", "use crate::widget::answer;"), Some("crate::widget::answer".into()));
        assert_eq!(import_target("rust", "use crate::{a,b};"), None);
        assert_eq!(import_target("python", "import pkg.util"), Some("pkg.util".into()));
        assert_eq!(import_target("python", "from .util import answer"), Some(".util".into()));
    }

    #[test]
    fn go_module_declaration_is_conservative() {
        assert_eq!(
            module_name("go 1.22\nmodule example.org/m\n"),
            Some("example.org/m")
        );
        assert_eq!(module_name("module ../bad\n"), None);
        assert_eq!(
            module_name("module example.org/m\nmodule example.org/n\n"),
            None
        );
    }
}
