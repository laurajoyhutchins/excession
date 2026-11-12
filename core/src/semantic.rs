//! Conservative language-specific indexing. Parsing is delegated to CPython AST
//! and TypeScript (for JS/TS). These are candidate relationships, never proof
//! that omitted tests are safe to skip.
use crate::adapter;
use crate::digest;
use crate::git::{self, Entry};
use crate::index::module_name_with_import_base;
use crate::json::Value;
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeMap;
use std::path::Path;

fn put_dependency(
    db: &Db,
    id: i64,
    commit: &str,
    module: &str,
    target: &str,
    line: i64,
    syntax: &str,
) -> Result<()> {
    if line < 1 {
        return refuse("invalid dependency line");
    }
    db.execute(
        "INSERT OR IGNORE INTO dependency VALUES(?,?,?,?,?,?)",
        &[
            Param::Integer(id),
            Param::Text(commit),
            Param::Text(module),
            Param::Text(target),
            Param::Integer(line),
            Param::Text(syntax),
        ],
    )
}
fn put_hazard(db: &Db, id: i64, commit: &str, module: &str, line: i64, reason: &str) -> Result<()> {
    if line < 1 {
        return refuse("invalid hazard location");
    }
    db.execute(
        "INSERT OR IGNORE INTO hazard VALUES(?,?,?,?,?)",
        &[
            Param::Integer(id),
            Param::Text(commit),
            Param::Text(module),
            Param::Integer(line),
            Param::Text(reason),
        ],
    )
}
fn put_symbol(db: &Db, id: i64, commit: &str, module: &str, info: &Value) -> Result<()> {
    let name = info.get("name")?.str()?;
    let kind = info.get("kind")?.str()?;
    let signature = info.get("signature")?.str()?;
    let body = info.get("body_fingerprint")?.str()?;
    let line = info.get("line")?.integer()?;
    if line < 1
        || !matches!(kind, "class" | "function" | "async_function" | "variable")
        || digest::unhex(body)?.len() != 32
    {
        return refuse("invalid symbol metadata");
    }
    db.execute(
        "INSERT OR IGNORE INTO symbol VALUES(?,?,?,?,?,?,?,?)",
        &[
            Param::Integer(id),
            Param::Text(commit),
            Param::Text(module),
            Param::Text(name),
            Param::Text(kind),
            Param::Text(signature),
            Param::Text(body),
            Param::Integer(line),
        ],
    )
}

fn normalize_local(path: &str, reference: &str) -> Option<String> {
    if !reference.starts_with('.') {
        return None;
    }
    let base = path.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
    let mut parts: Vec<&str> = base.split('/').filter(|x| !x.is_empty()).collect();
    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn resolve_js_target(
    source: &str,
    reference: &str,
    available: &BTreeMap<String, String>,
    mains: &BTreeMap<String, String>,
) -> Option<String> {
    let candidate = normalize_local(source, reference)?;
    let mut options = vec![
        candidate.clone(),
        format!("{candidate}.js"),
        format!("{candidate}.mjs"),
        format!("{candidate}.cjs"),
        format!("{candidate}.ts"),
        format!("{candidate}/index.js"),
        format!("{candidate}/index.mjs"),
        format!("{candidate}/index.cjs"),
        format!("{candidate}/index.ts"),
    ];
    if let Some(main) = mains.get(&candidate)
        && let Some(path) = normalize_local(&format!("{candidate}/package.json"), main)
    {
        options.insert(0, path);
    }
    options.iter().find_map(|p| available.get(p).cloned())
}

/// Prepared outside SQLite's write transaction. A failed adapter cannot
/// modify a previous, successfully indexed revision.
pub(crate) struct PreparedSources {
    modules: BTreeMap<String, String>,
    map_js: BTreeMap<String, String>,
    mains: BTreeMap<String, String>,
    python: Value,
    javascript: Value,
}

pub(crate) fn prepare_sources(
    root: &Path,
    entries: &[Entry],
    roots: &[&str],
    import_base: Option<&str>,
) -> Result<PreparedSources> {
    let mut modules = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut mains = BTreeMap::<String, String>::new();
    let mut blob_reader = git::BlobReader::open(root)?;
    for e in entries {
        if e.kind == "blob"
            && e.path.ends_with("/package.json")
            && matches!(e.mode.as_str(), "100644" | "100755")
            && let Ok(v) = crate::json::parse(&blob_reader.read(&e.oid)?)
            && let Ok(main) = v.get("main").and_then(Value::str)
        {
            let parent = e.path.rsplit_once('/').map(|x| x.0).unwrap_or("");
            mains.insert(parent.into(), main.into());
        }
        if e.kind != "blob" || !matches!(e.mode.as_str(), "100644" | "100755") {
            continue;
        }
        if let Some((name, _)) = module_name_with_import_base(&e.path, roots, import_base) {
            if modules.insert(name.clone(), e.path.clone()).is_some() {
                return refuse("duplicate logical module identity");
            }
            files.insert(e.path.clone(), (name, e.oid.clone()));
        }
    }
    let mut py = Vec::new();
    let mut js = Vec::new();
    let mut map_js = BTreeMap::new();
    for (path, (name, oid)) in &files {
        let content = String::from_utf8(blob_reader.read(oid)?)
            .map_err(|_| "source is not UTF-8; unsupported source decoder".to_owned())?;
        if path.ends_with(".py") {
            py.push(adapter::entry(name, path, &content));
        } else {
            js.push(adapter::js_entry(path, &content));
            map_js.insert(path.clone(), name.clone());
        }
    }
    blob_reader.finish()?;
    let python = if py.is_empty() {
        Value::array([])
    } else {
        adapter::python(&py)?
    };
    let javascript = if js.is_empty() {
        Value::array([])
    } else {
        adapter::javascript(&js)?
    };
    Ok(PreparedSources {
        modules,
        map_js,
        mains,
        python,
        javascript,
    })
}

/// Persist fresh observations after old derived rows have been deleted in the
/// same transaction. Git identity rows and independent receipts are untouched.
pub(crate) fn index_prepared(
    db: &Db,
    id: i64,
    commit: &str,
    prepared: &PreparedSources,
) -> Result<()> {
    let PreparedSources {
        modules,
        map_js,
        mains,
        python,
        javascript,
    } = prepared;
    if !python.array_ref()?.is_empty() {
        for item in python.array_ref()? {
            let module = item.get("name")?.str()?;
            let path = item.get("path")?.str()?;
            if modules.get(module).map(String::as_str) != Some(path) {
                return refuse("Python adapter changed module identity");
            }
            for imp in item.get("imports")?.array_ref()? {
                let target = imp.get("target")?.str()?;
                if !modules.contains_key(target) {
                    return refuse("adapter references unknown source module");
                }
                put_dependency(
                    db,
                    id,
                    commit,
                    module,
                    target,
                    imp.get("line")?.integer()?,
                    imp.get("syntax")?.str()?,
                )?;
            }
            for symbol in item.get("symbols")?.array_ref()? {
                put_symbol(db, id, commit, module, symbol)?;
            }
            for hazard in item.get("hazards")?.array_ref()? {
                put_hazard(
                    db,
                    id,
                    commit,
                    module,
                    hazard.get("line")?.integer()?,
                    hazard.get("reason")?.str()?,
                )?;
            }
            for export in item.get("exports")?.array_ref()? {
                let declared = export.get("explicitly_exported")?.integer()?;
                if !matches!(declared, 0 | 1) {
                    return refuse("invalid Python public API metadata");
                }
                db.execute(
                    "INSERT OR IGNORE INTO api_binding VALUES(?,?,?,?,?,?)",
                    &[
                        Param::Integer(id),
                        Param::Text(commit),
                        Param::Text(module),
                        Param::Text(export.get("name")?.str()?),
                        Param::Text(export.get("target")?.str()?),
                        Param::Integer(declared),
                    ],
                )?;
            }
        }
    }
    if !javascript.array_ref()?.is_empty() {
        for item in javascript.array_ref()? {
            let path = item.get("path")?.str()?;
            let module = map_js
                .get(path)
                .ok_or("JS adapter returned unknown source path")?;
            for imp in item.get("imports")?.array_ref()? {
                let spec = imp.get("specifier")?.str()?;
                let line = imp.get("line")?.integer()?;
                if let Some(target) = resolve_js_target(path, spec, map_js, mains) {
                    put_dependency(
                        db,
                        id,
                        commit,
                        module,
                        &target,
                        line,
                        imp.get("syntax")?.str()?,
                    )?;
                } else if spec.starts_with('.') {
                    put_hazard(
                        db,
                        id,
                        commit,
                        module,
                        line,
                        &format!("unresolved local import: {spec}"),
                    )?;
                }
            }
            for symbol in item.get("symbols")?.array_ref()? {
                put_symbol(db, id, commit, module, symbol)?;
            }
            for hazard in item.get("hazards")?.array_ref()? {
                put_hazard(
                    db,
                    id,
                    commit,
                    module,
                    hazard.get("line")?.integer()?,
                    hazard.get("reason")?.str()?,
                )?;
            }
            for err in item.get("parse_errors")?.array_ref()? {
                put_hazard(
                    db,
                    id,
                    commit,
                    module,
                    err.get("line")?.integer()?,
                    &format!("parse error: {}", err.get("message")?.str()?),
                )?;
            }
        }
    }
    // The resulting graph is an over/under-approximation; lack of edges
    // NEVER constitutes permission to skip verification.
    let indexed = db
        .scalar(
            "SELECT COUNT(*) FROM module WHERE repo_id=? AND commit_oid=?",
            &[Param::Integer(id), Param::Text(commit)],
        )?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    if indexed != modules.len() {
        return refuse("module catalog count differs from analyzed source identities");
    }
    Ok(())
}
