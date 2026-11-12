//! Conservative language-specific indexing. Parsing is delegated to CPython AST
//! and TypeScript (for JS/TS). These are candidate relationships, never proof
//! that omitted tests are safe to skip.
use crate::adapter;
use crate::digest;
use crate::git::{self, Entry};
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// Compatibility exports for callers of the original semantic module.
pub use crate::module_identity::{module_name, module_name_with_import_base};

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

/// Prepared outside SQLite's write transaction. A failed adapter cannot
/// modify a previous, successfully indexed revision.
pub(crate) struct PreparedSources {
    modules: BTreeMap<String, String>,
    map_js: BTreeMap<String, String>,
    python: Value,
    javascript: Value,
}

/// The only incremental-reuse facts needed by source preparation. The
/// authenticated snapshot, database writes and producer checks remain owned
/// by incremental_python, not the language adapter boundary.
#[derive(Clone, Copy)]
pub(crate) struct ReuseSelection<'a> {
    pub paths: &'a BTreeSet<String>,
    pub total: usize,
}

fn verify_reuse_inventory(
    reuse: Option<ReuseSelection<'_>>,
    parsed: usize,
    catalog: usize,
) -> Result<()> {
    if let Some(selection) = reuse
        && (selection.total != catalog
            || parsed.checked_add(selection.paths.len()) != Some(selection.total))
    {
        return refuse("incomplete Python incremental source inventory");
    }
    Ok(())
}

pub(crate) fn prepare_sources(
    root: &Path,
    entries: &[Entry],
    roots: &[&str],
    import_base: Option<&str>,
    reuse: Option<ReuseSelection<'_>>,
) -> Result<PreparedSources> {
    let mut modules = BTreeMap::new();
    let mut files = BTreeMap::new();
    let mut configs = Vec::new();
    let mut blob_reader = git::BlobReader::open(root)?;
    for e in entries {
        if e.kind != "blob" || !matches!(e.mode.as_str(), "100644" | "100755") {
            continue;
        }
        if ["package.json", "tsconfig.json", "jsconfig.json"]
            .iter()
            .any(|name| e.path == *name || e.path.ends_with(&format!("/{name}")))
        {
            let content = String::from_utf8(blob_reader.read_capped(&e.oid, 1024 * 1024)?)
                .map_err(|_| "Git configuration file is not UTF-8")?;
            configs.push(adapter::js_entry(&e.path, &content));
        }
        if let Some((name, _)) = module_name_with_import_base(&e.path, roots, import_base) {
            if modules.insert(name.clone(), e.path.clone()).is_some() {
                return refuse("duplicate logical module identity");
            }
            files.insert(e.path.clone(), (name, e.oid.clone()));
        }
    }
    let catalog = files
        .iter()
        .filter(|(path, _)| path.ends_with(".py"))
        .map(|(path, (name, _))| {
            obj(&[("name", Value::string(name)), ("path", Value::string(path))])
        })
        .collect::<Vec<_>>();
    let mut py = Vec::new();
    let mut js = Vec::new();
    let mut map_js = BTreeMap::new();
    for (path, (name, oid)) in &files {
        if reuse.is_some_and(|selection| selection.paths.contains(path)) {
            continue;
        }
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
    } else if reuse.is_some_and(|selection| !selection.paths.is_empty()) {
        adapter::python_subset(&py, &catalog)?
    } else {
        adapter::python(&py)?
    };
    verify_reuse_inventory(reuse, python.array_ref()?.len(), catalog.len())?;
    let javascript = if js.is_empty() {
        Value::array([])
    } else {
        adapter::javascript_with_config(&js, &configs)?
    };
    Ok(PreparedSources {
        modules,
        map_js,
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
                let target = imp.get("target_path")?;
                if target != &Value::Null {
                    let target_path = target.str()?;
                    let target_module = map_js
                        .get(target_path)
                        .ok_or("native JS resolver returned unindexed source")?;
                    put_dependency(
                        db,
                        id,
                        commit,
                        module,
                        target_module,
                        line,
                        imp.get("syntax")?.str()?,
                    )?;
                } else if imp.get("resolution_issue")?.str()?.is_empty() {
                    return refuse(format!("native JS resolver lost import: {spec}"));
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

#[cfg(test)]
mod module_name_tests {
    use super::*;

    #[test]
    fn source_selection_refuses_an_incomplete_or_inconsistent_reuse_inventory() {
        let reused = BTreeSet::from(["pkg/left.py".to_owned()]);
        let selection = ReuseSelection {
            paths: &reused,
            total: 2,
        };
        assert!(verify_reuse_inventory(Some(selection), 1, 2).is_ok());
        assert!(verify_reuse_inventory(None, 2, 2).is_ok());
        assert!(verify_reuse_inventory(Some(selection), 0, 2).is_err());
        assert!(verify_reuse_inventory(Some(selection), 1, 3).is_err());
        assert!(verify_reuse_inventory(Some(selection), usize::MAX, 2).is_err());
    }

    #[test]
    fn semantic_module_names_preserve_import_root_and_test_flags() {
        assert_eq!(
            module_name_with_import_base("pkg/features/test_parse.py", &["pkg"], Some("pkg")),
            Some(("features.test_parse".to_owned(), true)),
        );
        assert_eq!(
            module_name("pkg/client.ts", &["pkg"]),
            Some(("pkg.client.ts".to_owned(), false)),
        );
        assert!(module_name("node_modules/pkg/client.js", &["."]).is_none());
        assert!(module_name_with_import_base("other/a.py", &["pkg"], Some("pkg")).is_none());
    }
}
