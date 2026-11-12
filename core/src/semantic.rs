//! Conservative language-specific indexing. Parsing is delegated to CPython AST
//! and TypeScript (for JS/TS). These are candidate relationships, never proof
//! that omitted tests are safe to skip.
use crate::adapter;
use crate::digest;
use crate::git::{self, Entry};
use crate::index::module_name_with_import_base;
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone)]
struct PythonSnapshot {
    producer: String,
    catalog: String,
    rows: String,
}

/// The compiled Rust semantic policy, exact CPython adapter and interpreter
/// version are part of the reuse identity. A changed producer forces parsing.
fn python_producer() -> Result<String> {
    let result = adapter::python_command()
        .arg("--version")
        .output()
        .map_err(|e| format!("Python version probe failed: {e}"))?;
    if !result.status.success() {
        return refuse("Python interpreter version probe failed");
    }
    let version = if result.stdout.is_empty() {
        &result.stderr
    } else {
        &result.stdout
    };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"excession-python-incremental-v1\\0");
    bytes.extend_from_slice(include_str!("semantic.rs").as_bytes());
    bytes.extend_from_slice(include_str!("../adapters/python_ast.py").as_bytes());
    bytes.extend_from_slice(version);
    Ok(digest::sha256(&bytes))
}

fn python_catalog(
    files: &BTreeMap<String, (String, String)>,
    roots: &[&str],
    import_base: Option<&str>,
) -> String {
    let modules = files
        .iter()
        .filter(|(path, _)| path.ends_with(".py"))
        .map(|(path, (name, _))| {
            obj(&[("name", Value::string(name)), ("path", Value::string(path))])
        })
        .collect::<Vec<_>>();
    let mut roots = roots.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    let catalog = obj(&[
        ("modules", Value::array(modules)),
        ("roots", Value::array(roots.iter().map(Value::string))),
        ("import_base", Value::string(import_base.unwrap_or(""))),
    ]);
    digest::sha256(catalog.canonical().as_bytes())
}

/// Hash every persisted semantic row as a closed, canonical table inventory.
/// An omitted row is detectable; source paths/blobs are authenticated by the
/// Git tree reconstruction, not by trusting these hashes alone.
fn python_rows_digest(db: &Db, repo: i64, commit: &str) -> Result<String> {
    let tables = [
        ("module", "name,path,blob_oid,is_test", "name,path"),
        (
            "dependency",
            "source_name,target_name,line,syntax",
            "source_name,target_name,line,syntax",
        ),
        (
            "symbol",
            "module_name,qualname,kind,signature,body_fingerprint,line",
            "module_name,qualname",
        ),
        (
            "api_binding",
            "module_name,public_name,target,explicitly_exported",
            "module_name,public_name",
        ),
        (
            "hazard",
            "module_name,line,reason",
            "module_name,line,reason",
        ),
    ];
    let mut inventory = Vec::new();
    for (table, fields, order) in tables {
        let sql = format!(
            "SELECT {fields} FROM {table} WHERE repo_id=? AND commit_oid=? ORDER BY {order}"
        );
        let rows = db.query(&sql, &[Param::Integer(repo), Param::Text(commit)])?;
        let records = rows
            .into_iter()
            .map(|columns| {
                Value::array(columns.into_iter().map(|field| match field {
                    Some(value) => Value::string(value),
                    None => Value::Null,
                }))
            })
            .collect::<Vec<_>>();
        inventory.push(obj(&[
            ("table", Value::string(table)),
            ("rows", Value::array(records)),
        ]));
    }
    Ok(digest::sha256(
        Value::array(inventory).canonical().as_bytes(),
    ))
}

fn snapshot(db: &Db, repo: i64, commit: &str) -> Result<Option<PythonSnapshot>> {
    let rows = db.query(
        "SELECT producer_sha256,catalog_sha256,rows_sha256
         FROM python_index_snapshot WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo), Param::Text(commit)],
    )?;
    if rows.len() > 1 {
        return refuse("ambiguous Python index snapshot");
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(PythonSnapshot {
        producer: row[0].clone().ok_or("missing index producer")?,
        catalog: row[1].clone().ok_or("missing index catalog")?,
        rows: row[2].clone().ok_or("missing index row receipt")?,
    }))
}

fn authenticate_snapshot(db: &Db, repo: i64, commit: &str, record: &PythonSnapshot) -> Result<()> {
    let observed = python_rows_digest(db, repo, commit)?;
    if record.rows != observed {
        return refuse("stored Python index rows disagree with completed snapshot");
    }
    Ok(())
}

fn first_parent(root: &Path, commit: &str) -> Result<Option<String>> {
    let line = git::text(root, &["rev-list", "--parents", "-n", "1", commit])?;
    let items = line.split_whitespace().collect::<Vec<_>>();
    if items.len() == 2 && items[0] == commit {
        return Ok(Some(git::oid(items[1])?));
    }
    // Root and merge commits deliberately use the full-index path.
    Ok(None)
}

const MATCHED_PYTHON_MODULES: &str = "
SELECT newer.path
FROM module AS newer
JOIN module AS older
  ON older.repo_id=newer.repo_id AND older.commit_oid=?
 AND older.name=newer.name AND older.path=newer.path
 AND older.blob_oid=newer.blob_oid AND older.is_test=newer.is_test
JOIN tree_entry AS new_entry
  ON new_entry.repo_id=newer.repo_id
 AND new_entry.commit_oid=newer.commit_oid AND new_entry.path=newer.path
JOIN tree_entry AS old_entry
  ON old_entry.repo_id=older.repo_id
 AND old_entry.commit_oid=older.commit_oid AND old_entry.path=older.path
WHERE newer.repo_id=? AND newer.commit_oid=?
  AND newer.path LIKE '%.py' AND new_entry.mode=old_entry.mode
ORDER BY newer.path";

fn reusable_python(
    db: &Db,
    repo: i64,
    root: &Path,
    commit: &str,
    producer: &str,
    catalog: &str,
) -> Result<Option<(String, BTreeSet<String>)>> {
    let Some(parent) = first_parent(root, commit)? else {
        return Ok(None);
    };
    let Some(prior) = snapshot(db, repo, &parent)? else {
        return Ok(None);
    };
    if prior.producer != producer || prior.catalog != catalog {
        return Ok(None);
    }
    // Authenticate the parent against actual Git, not just internally
    // self-consistent SQLite rows. Reuse of an untrusted snapshot is refused.
    let actual = git::rev(root, &format!("{parent}^{{tree}}"))?;
    if crate::index::reconstruct_tree(db, repo, &parent)? != actual {
        return refuse("parent Git tree differs from stored index");
    }
    authenticate_snapshot(db, repo, &parent, &prior)?;
    let rows = db.query(
        MATCHED_PYTHON_MODULES,
        &[
            Param::Text(&parent),
            Param::Integer(repo),
            Param::Text(commit),
        ],
    )?;
    let mut paths = BTreeSet::new();
    for row in rows {
        if !paths.insert(row[0].clone().ok_or("null reusable module path")?) {
            return refuse("duplicate reusable module identity");
        }
    }
    Ok(Some((parent, paths)))
}

/// Copy original observations into the new revision, preserving all semantic
/// fields. This does not upgrade a static candidate into verified behavior.
fn carry_python_rows(db: &Db, repo: i64, parent: &str, commit: &str) -> Result<()> {
    let specifications = [
        (
            "dependency",
            "source_name",
            "source_name,target_name,line,syntax",
        ),
        (
            "symbol",
            "module_name",
            "module_name,qualname,kind,signature,body_fingerprint,line",
        ),
        (
            "api_binding",
            "module_name",
            "module_name,public_name,target,explicitly_exported",
        ),
        ("hazard", "module_name", "module_name,line,reason"),
    ];
    for (table, module_column, fields) in specifications {
        let sql = format!(
            "INSERT INTO {table}(repo_id,commit_oid,{fields})
             SELECT ?,?,{fields}
             FROM {table} AS previous
             WHERE previous.repo_id=? AND previous.commit_oid=?
             AND previous.{module_column} IN (
                 SELECT newer.name
                 FROM module AS newer
                 JOIN module AS older
                   ON older.repo_id=newer.repo_id AND older.commit_oid=?
                  AND older.name=newer.name AND older.path=newer.path
                  AND older.blob_oid=newer.blob_oid AND older.is_test=newer.is_test
                 JOIN tree_entry AS new_entry
                   ON new_entry.repo_id=newer.repo_id
                  AND new_entry.commit_oid=newer.commit_oid AND new_entry.path=newer.path
                 JOIN tree_entry AS old_entry
                   ON old_entry.repo_id=older.repo_id
                  AND old_entry.commit_oid=older.commit_oid AND old_entry.path=older.path
                 WHERE newer.repo_id=? AND newer.commit_oid=?
                   AND newer.path LIKE '%.py' AND new_entry.mode=old_entry.mode
             )"
        );
        db.execute(
            &sql,
            &[
                Param::Integer(repo),
                Param::Text(commit),
                Param::Integer(repo),
                Param::Text(parent),
                Param::Text(parent),
                Param::Integer(repo),
                Param::Text(commit),
            ],
        )?;
    }
    Ok(())
}

fn clear_derived_rows(db: &Db, repo: i64, commit: &str) -> Result<()> {
    // Older databases might have been indexed before receipts existed. Rebuild
    // instead of allowing INSERT OR IGNORE to preserve stale derived rows.
    for table in ["dependency", "symbol", "api_binding", "hazard"] {
        db.execute(
            &format!("DELETE FROM {table} WHERE repo_id=? AND commit_oid=?"),
            &[Param::Integer(repo), Param::Text(commit)],
        )?;
    }
    Ok(())
}

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

pub fn index_sources(
    db: &Db,
    id: i64,
    root: &Path,
    commit: &str,
    entries: &[Entry],
    roots: &[&str],
    import_base: Option<&str>,
) -> Result<()> {
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
    // Python-only revision reuse is intentionally narrow. JS resolution also
    // depends on package.json/main and TypeScript configuration, so mixed and
    // JS-only repositories retain their existing full-analysis behavior.
    let python_only = !files.is_empty() && files.keys().all(|path| path.ends_with(".py"));
    let mut reusable_paths = BTreeSet::new();
    let mut reuse_parent = None;
    let mut python_identity = None;
    if python_only {
        let producer = python_producer()?;
        let catalog = python_catalog(&files, roots, import_base);
        if let Some(existing) = snapshot(db, id, commit)? {
            if existing.producer != producer || existing.catalog != catalog {
                return refuse("completed Python index used a different parser or module catalog");
            }
            authenticate_snapshot(db, id, commit, &existing)?;
            blob_reader.finish()?;
            return Ok(());
        }
        if let Some((parent, paths)) = reusable_python(db, id, root, commit, &producer, &catalog)? {
            reuse_parent = Some(parent);
            reusable_paths = paths;
        }
        clear_derived_rows(db, id, commit)?;
        if let Some(ref parent) = reuse_parent {
            carry_python_rows(db, id, parent, commit)?;
        }
        python_identity = Some((producer, catalog));
    }

    let python_module_catalog = files
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
        if reusable_paths.contains(path) {
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
    if !py.is_empty() {
        let result = if reusable_paths.is_empty() {
            adapter::python(&py)?
        } else {
            adapter::python_subset(&py, &python_module_catalog)?
        };
        for item in result.array_ref()? {
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
    if !js.is_empty() {
        for item in adapter::javascript(&js)?.array_ref()? {
            let path = item.get("path")?.str()?;
            let module = map_js
                .get(path)
                .ok_or("JS adapter returned unknown source path")?;
            for imp in item.get("imports")?.array_ref()? {
                let spec = imp.get("specifier")?.str()?;
                let line = imp.get("line")?.integer()?;
                if let Some(target) = resolve_js_target(path, spec, &map_js, &mains) {
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
    if let Some((producer, catalog)) = python_identity {
        let parsed = py.len();
        let reused = reusable_paths.len();
        if parsed + reused != files.len() {
            return refuse("incomplete Python module reuse accounting");
        }
        let rows = python_rows_digest(db, id, commit)?;
        db.execute(
            "INSERT INTO python_index_snapshot
             (repo_id,commit_oid,producer_sha256,catalog_sha256,rows_sha256,
              parsed_modules,reused_modules) VALUES(?,?,?,?,?,?,?)",
            &[
                Param::Integer(id),
                Param::Text(commit),
                Param::Text(&producer),
                Param::Text(&catalog),
                Param::Text(&rows),
                Param::Integer(parsed as i64),
                Param::Integer(reused as i64),
            ],
        )?;
    }
    Ok(())
}
