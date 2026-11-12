//! Bounded reuse of revision-scoped Python observations in the existing index.
//! All observations remain in the original SQL tables; Git owns source identity.
//! Runs outside the writer transaction except for atomic row carry/publication.

use crate::adapter;
use crate::digest;
use crate::git::{self, Entry};
use crate::indexed_tree;
use crate::module_identity::module_name_with_import_base;
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone)]
struct Snapshot {
    producer: String,
    catalog: String,
    rows: String,
}

pub(crate) struct Plan {
    producer: String,
    catalog: String,
    parent: Option<String>,
    pub(crate) reused: BTreeSet<String>,
    pub(crate) total: usize,
}

fn producer() -> Result<String> {
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
    bytes.extend_from_slice(b"excession-python-incremental-v2");
    bytes.extend_from_slice(include_str!("incremental_python.rs").as_bytes());
    bytes.extend_from_slice(include_str!("semantic.rs").as_bytes());
    // Module-name semantics affect the reuse catalog and must invalidate
    // authenticated snapshots when they change.
    bytes.extend_from_slice(include_str!("module_identity.rs").as_bytes());
    bytes.extend_from_slice(include_str!("../adapters/python_ast.py").as_bytes());
    bytes.extend_from_slice(version);
    Ok(digest::sha256(&bytes))
}

fn catalog(
    files: &BTreeMap<String, (String, String, String)>,
    roots: &[&str],
    import_base: Option<&str>,
) -> String {
    let modules = files
        .iter()
        .map(|(path, (name, _, _))| {
            obj(&[("name", Value::string(name)), ("path", Value::string(path))])
        })
        .collect::<Vec<_>>();
    let mut roots = roots
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    let document = obj(&[
        ("modules", Value::array(modules)),
        ("roots", Value::array(roots.iter().map(Value::string))),
        ("import_base", Value::string(import_base.unwrap_or(""))),
    ]);
    digest::sha256(document.canonical().as_bytes())
}

fn row_digest(db: &Db, repo: i64, commit: &str) -> Result<String> {
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
    for (table, columns, order) in tables {
        let statement = format!(
            "SELECT {columns} FROM {table} WHERE repo_id=? AND commit_oid=? ORDER BY {order}"
        );
        let rows = db.query(&statement, &[Param::Integer(repo), Param::Text(commit)])?;
        let values = rows
            .into_iter()
            .map(|row| {
                Value::array(row.into_iter().map(|field| match field {
                    Some(value) => Value::string(value),
                    None => Value::Null,
                }))
            })
            .collect::<Vec<_>>();
        inventory.push(obj(&[
            ("table", Value::string(table)),
            ("rows", Value::array(values)),
        ]));
    }
    Ok(digest::sha256(
        Value::array(inventory).canonical().as_bytes(),
    ))
}

fn snapshot(db: &Db, repo: i64, commit: &str) -> Result<Option<Snapshot>> {
    let rows = db.query(
        "SELECT producer_sha256,catalog_sha256,rows_sha256
         FROM python_index_snapshot WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo), Param::Text(commit)],
    )?;
    if rows.len() > 1 {
        return refuse("ambiguous completed Python snapshot");
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    Ok(Some(Snapshot {
        producer: row[0].clone().ok_or("null index producer")?,
        catalog: row[1].clone().ok_or("null module catalog")?,
        rows: row[2].clone().ok_or("null completed row digest")?,
    }))
}

fn authenticate(db: &Db, root: &Path, repo: i64, parent: &str, proof: &Snapshot) -> Result<()> {
    let actual = git::rev(root, &format!("{parent}^{{tree}}"))?;
    if indexed_tree::reconstruct_tree(db, repo, parent)? != actual {
        return refuse("parent Git tree does not match stored source index");
    }
    if row_digest(db, repo, parent)? != proof.rows {
        return refuse("stored Python index rows disagree with completed snapshot");
    }
    Ok(())
}

fn parent(root: &Path, commit: &str) -> Result<Option<String>> {
    let result = git::text(root, &["rev-list", "--parents", "-n", "1", commit])?;
    let tokens = result.split_whitespace().collect::<Vec<_>>();
    if tokens.len() == 2 && tokens[0] == commit {
        return Ok(Some(git::oid(tokens[1])?));
    }
    // Root and merge commits use a full scan.
    Ok(None)
}

/// A complete module catalog is needed to resolve imports correctly. Reuse
/// requires identical names/paths, but not identical source blobs: changed
/// blobs are reparsed while identical blob+mode observations are carried.
pub(crate) fn plan(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    entries: &[Entry],
    roots: &[&str],
    import_base: Option<&str>,
) -> Result<Option<Plan>> {
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    for entry in entries {
        if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
            continue;
        }
        let Some((module, _)) = module_name_with_import_base(&entry.path, roots, import_base)
        else {
            continue;
        };
        if !entry.path.ends_with(".py") {
            // JS/TS resolver options need separate, independently established
            // invalidation rules. This optimization does not touch them.
            return Ok(None);
        }
        if !names.insert(module.clone()) {
            return refuse("duplicate logical module identity");
        }
        files.insert(
            entry.path.clone(),
            (module, entry.oid.clone(), entry.mode.clone()),
        );
    }
    if files.is_empty() {
        return Ok(None);
    }
    let mut planned = Plan {
        producer: producer()?,
        catalog: catalog(&files, roots, import_base),
        parent: None,
        reused: BTreeSet::new(),
        total: files.len(),
    };
    let Some(parent) = parent(root, commit)? else {
        return Ok(Some(planned));
    };
    let rows = db.query(
        "SELECT id,git_dir FROM repository WHERE name=?",
        &[Param::Text(name)],
    )?;
    if rows.is_empty() {
        return Ok(Some(planned));
    }
    if rows.len() != 1 {
        return refuse("ambiguous repository identity for incremental indexing");
    }
    let repo = rows[0][0]
        .as_ref()
        .ok_or("null repository ID")?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    let stored_root = rows[0][1].as_ref().ok_or("null source checkout")?;
    if Path::new(stored_root)
        .canonicalize()
        .map_err(|e| e.to_string())?
        != root.canonicalize().map_err(|e| e.to_string())?
    {
        return refuse("registered index source differs from requested Git checkout");
    }
    let Some(prior) = snapshot(db, repo, &parent)? else {
        return Ok(Some(planned));
    };
    if prior.producer != planned.producer || prior.catalog != planned.catalog {
        return Ok(Some(planned));
    }
    authenticate(db, root, repo, &parent, &prior)?;
    let older = db.query(
        "SELECT m.name,m.path,m.blob_oid,t.mode FROM module AS m
         JOIN tree_entry AS t ON t.repo_id=m.repo_id
             AND t.commit_oid=m.commit_oid AND t.path=m.path
         WHERE m.repo_id=? AND m.commit_oid=? ORDER BY m.name",
        &[Param::Integer(repo), Param::Text(&parent)],
    )?;
    let mut old = BTreeMap::new();
    for row in older {
        let name = row[0].clone().ok_or("null previous module name")?;
        let path = row[1].clone().ok_or("null previous module path")?;
        let blob = row[2].clone().ok_or("null previous blob")?;
        let mode = row[3].clone().ok_or("null previous mode")?;
        if old.insert(path, (name, blob, mode)).is_some() {
            return refuse("duplicate previous module");
        }
    }
    for (path, identity) in &files {
        if old.get(path) == Some(identity) {
            planned.reused.insert(path.clone());
        }
    }
    planned.parent = Some(parent);
    Ok(Some(planned))
}

const MATCHED: &str = "
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

/// Call inside the same write transaction as target-index publication.
/// Revalidate the parent so a concurrent change between planning and commit
/// cannot silently promote corrupt SQLite rows.
pub(crate) fn carry(db: &Db, root: &Path, repo: i64, commit: &str, planned: &Plan) -> Result<()> {
    let Some(parent) = &planned.parent else {
        return Ok(());
    };
    let prior = snapshot(db, repo, parent)?
        .ok_or("parent Python snapshot disappeared before publication")?;
    if prior.producer != planned.producer || prior.catalog != planned.catalog {
        return refuse("parent Python source/producer context changed");
    }
    authenticate(db, root, repo, parent, &prior)?;
    let matches = db.query(
        MATCHED,
        &[
            Param::Text(parent),
            Param::Integer(repo),
            Param::Text(commit),
        ],
    )?;
    let paths = matches
        .into_iter()
        .map(|row| {
            row[0]
                .clone()
                .ok_or_else(|| "null reusable path".to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if paths != planned.reused {
        return refuse("source identities changed between incremental planning and publication");
    }
    let specs = [
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
    for (table, module_column, columns) in specs {
        let sql = format!(
            "INSERT INTO {table}(repo_id,commit_oid,{columns})
             SELECT ?,?,{columns} FROM {table} AS old_row
             WHERE old_row.repo_id=? AND old_row.commit_oid=?
             AND old_row.{module_column} IN (
                 SELECT newer.name FROM module AS newer
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

pub(crate) fn finish(db: &Db, repo: i64, commit: &str, planned: &Plan) -> Result<()> {
    let reused = planned.reused.len();
    let parsed = planned
        .total
        .checked_sub(reused)
        .ok_or("invalid reuse count")?;
    let rows = row_digest(db, repo, commit)?;
    db.execute(
        "INSERT INTO python_index_snapshot
         (repo_id,commit_oid,producer_sha256,catalog_sha256,rows_sha256,
          parsed_modules,reused_modules) VALUES(?,?,?,?,?,?,?)
         ON CONFLICT(repo_id,commit_oid) DO UPDATE SET
          producer_sha256=excluded.producer_sha256,
          catalog_sha256=excluded.catalog_sha256,
          rows_sha256=excluded.rows_sha256,
          parsed_modules=excluded.parsed_modules,
          reused_modules=excluded.reused_modules",
        &[
            Param::Integer(repo),
            Param::Text(commit),
            Param::Text(&planned.producer),
            Param::Text(&planned.catalog),
            Param::Text(&rows),
            Param::Integer(parsed as i64),
            Param::Integer(reused as i64),
        ],
    )
}
