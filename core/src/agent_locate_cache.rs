//! Incremental, per-checkout task lookup cache.
//!
//! This is an *index* of independently authenticated Git blobs, not a new
//! source authority. A transaction replaces the active source snapshot so
//! concurrent readers cannot mix files from different revisions. The cache
//! lives beside Excession's external SQLite evidence store, never in the
//! repository or its working tree.
use crate::{Result, discovery, git, refuse};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const CACHE_VERSION: &str = "agent-locate-fts5-v1";
pub(crate) const MAX_FILES: usize = 6000;
pub(crate) const MAX_FILE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct Hit {
    pub entry: git::Entry,
    pub body: String,
}

pub(crate) struct Search {
    pub hits: Vec<Hit>,
    pub indexed_files: usize,
    pub indexed_bytes: usize,
    pub skipped_files: usize,
    pub truncated: bool,
    pub reused_blobs: usize,
    pub read_blobs: usize,
    pub cache_path: PathBuf,
}

fn sql_error(where_: &str, error: impl std::fmt::Display) -> String {
    format!("agent locate cache {where_}: {error}")
}

/// Reject cache destinations under the checkout, including symlinked parents.
/// The source repository remains read-only even if cache environment variables
/// are pointed at a checkout directory.
fn external_cache(discovered: &discovery::Discovery) -> Result<PathBuf> {
    let path = discovered.index_path.with_file_name("agent-locate.sqlite");
    let parent = path.parent().ok_or("agent locate cache missing parent")?;
    let root = fs::canonicalize(&discovered.root).map_err(|e| sql_error("root canonicalize", e))?;
    // Check *before* mkdir: a user-specified cache directory must never
    // create files inside the checkout, even when its final component does
    // not exist yet or an existing ancestor is a symlink into the checkout.
    if !parent.is_absolute()
        || parent
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return refuse("agent locate cache directory must be absolute without parent traversal");
    }
    let mut ancestor = parent;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or("no existing cache directory ancestor")?;
    }
    let ancestor = fs::canonicalize(ancestor).map_err(|e| sql_error("ancestor canonicalize", e))?;
    if ancestor.starts_with(&root) || parent.starts_with(&root) {
        return refuse("agent locate cache destination is inside the source repository");
    }
    fs::create_dir_all(parent).map_err(|e| sql_error("directory", e))?;
    let parent = fs::canonicalize(parent).map_err(|e| sql_error("directory canonicalize", e))?;
    if parent.starts_with(&root) {
        return refuse("agent locate cache destination is inside the source repository");
    }
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return refuse("agent locate cache destination is a symlink or non-file");
    }
    Ok(path)
}

fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS locator_meta(
             key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS locator_files(
             id INTEGER PRIMARY KEY,
             path TEXT UNIQUE NOT NULL,
             blob TEXT NOT NULL,
             mode TEXT NOT NULL,
             bytes INTEGER NOT NULL,
             status TEXT NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS source_search
             USING fts5(path, body, tokenize='unicode61');",
    )
    .map_err(|e| sql_error("initialize", e))?;
    let version: Option<String> = db
        .query_row(
            "SELECT value FROM locator_meta WHERE key='version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| sql_error("version", e))?;
    match version.as_deref() {
        Some(CACHE_VERSION) => {}
        None => {
            // Newly created cache must have no unversioned source records.
            // This file name is dedicated to this feature, not the evidence DB.
            let count: i64 = db
                .query_row("SELECT COUNT(*) FROM locator_files", [], |row| row.get(0))
                .map_err(|e| sql_error("initialization count", e))?;
            if count != 0 {
                return refuse("unversioned agent locate cache contains source records");
            }
            db.execute(
                "INSERT INTO locator_meta(key,value) VALUES('version',?1)",
                params![CACHE_VERSION],
            )
            .map_err(|e| sql_error("version insert", e))?;
        }
        Some(_) => return refuse("unsupported agent locate cache version"),
    }
    Ok(())
}

struct CachedFile {
    id: i64,
    blob: String,
    mode: String,
    size: usize,
    status: String,
}

fn previous(tx: &Transaction<'_>, path: &str) -> Result<Option<CachedFile>> {
    tx.query_row(
        "SELECT id,blob,mode,bytes,status FROM locator_files WHERE path=?1",
        params![path],
        |row| {
            let size: i64 = row.get(3)?;
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, size, row.get(4)?))
        },
    )
    .optional()
    .map_err(|e| sql_error("select cached path", e))?
    .map(|(id, blob, mode, size, status)| {
        let size = usize::try_from(size).map_err(|_| "negative cached source size")?;
        Ok(CachedFile {
            id,
            blob,
            mode,
            size,
            status,
        })
    })
    .transpose()
}

fn remove(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute("DELETE FROM source_search WHERE rowid=?1", params![id])
        .map_err(|e| sql_error("delete FTS row", e))?;
    tx.execute("DELETE FROM locator_files WHERE id=?1", params![id])
        .map_err(|e| sql_error("delete source row", e))?;
    Ok(())
}

fn insert(
    tx: &Transaction<'_>,
    entry: &git::Entry,
    body: Option<&str>,
    status: &str,
) -> Result<()> {
    let size = body.map_or(0usize, str::len);
    tx.execute(
        "INSERT INTO locator_files(path,blob,mode,bytes,status) VALUES(?1,?2,?3,?4,?5)",
        params![&entry.path, &entry.oid, &entry.mode, size as i64, status],
    )
    .map_err(|e| sql_error("insert source", e))?;
    if let Some(body) = body {
        tx.execute(
            "INSERT INTO source_search(rowid,path,body) VALUES(?1,?2,?3)",
            params![tx.last_insert_rowid(), &entry.path, body],
        )
        .map_err(|e| sql_error("insert FTS row", e))?;
    }
    Ok(())
}

fn meta(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO locator_meta(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|e| sql_error("snapshot metadata", e))?;
    Ok(())
}

pub(crate) fn search(
    discovered: &discovery::Discovery,
    entries: Vec<git::Entry>,
    limit: usize,
    expression: &str,
    source_path: impl Fn(&str) -> bool,
) -> Result<Search> {
    let path = external_cache(discovered)?;
    let mut db = Connection::open(&path).map_err(|e| sql_error("open", e))?;
    db.busy_timeout(std::time::Duration::from_secs(15))
        .map_err(|e| sql_error("busy timeout", e))?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| sql_error("begin", e))?;
    initialize(&tx)?;
    let mut reader: Option<git::BlobReader> = None;
    let mut seen = BTreeSet::<String>::new();
    let mut indexed_files = 0usize;
    let mut indexed_bytes = 0usize;
    let mut skipped_files = 0usize;
    let mut reused_blobs = 0usize;
    let mut read_blobs = 0usize;
    let mut truncated = false;

    for entry in entries {
        if entry.kind != "blob"
            || !matches!(entry.mode.as_str(), "100644" | "100755")
            || !source_path(&entry.path)
        {
            continue;
        }
        if seen.len() >= MAX_FILES || indexed_bytes >= MAX_TOTAL_BYTES {
            truncated = true;
            break;
        }
        seen.insert(entry.path.clone());
        let prior = previous(&tx, &entry.path)?;
        if let Some(ref cached) = prior
            && cached.blob == entry.oid
            && cached.mode == entry.mode
        {
            match cached.status.as_str() {
                "OK" if cached.size <= MAX_TOTAL_BYTES - indexed_bytes => {
                    indexed_bytes += cached.size;
                    indexed_files += 1;
                    reused_blobs += 1;
                    continue;
                }
                "LARGE" | "NON_UTF8" => {
                    skipped_files += 1;
                    continue;
                }
                _ => {}
            }
        }
        if let Some(cached) = prior {
            remove(&tx, cached.id)?;
        }
        if reader.is_none() {
            reader = Some(git::BlobReader::open(&discovered.root)?);
        }
        let data = match reader
            .as_mut()
            .ok_or("missing Git reader")?
            .read_capped(&entry.oid, MAX_FILE_BYTES)
        {
            Ok(data) => data,
            Err(error) if error == "Git source blob exceeds byte limit" => {
                skipped_files += 1;
                insert(&tx, &entry, None, "LARGE")?;
                continue;
            }
            Err(error) => return Err(error),
        };
        read_blobs += 1;
        let Ok(body) = String::from_utf8(data) else {
            skipped_files += 1;
            insert(&tx, &entry, None, "NON_UTF8")?;
            continue;
        };
        if body.contains('\0') {
            skipped_files += 1;
            insert(&tx, &entry, None, "NON_UTF8")?;
            continue;
        }
        if body.len() > MAX_TOTAL_BYTES - indexed_bytes {
            skipped_files += 1;
            truncated = true;
            // Budget omissions are not memoized: smaller future revisions may
            // make room for this exact blob.
            continue;
        }
        indexed_bytes += body.len();
        indexed_files += 1;
        insert(&tx, &entry, Some(&body), "OK")?;
    }
    if let Some(reader) = reader {
        reader.finish()?;
    }

    // A revision switch must atomically discard stale paths, including paths
    // that were excluded because the memory/source-count budget was reached.
    let mut stale = Vec::new();
    {
        let mut stmt = tx
            .prepare("SELECT id,path FROM locator_files")
            .map_err(|e| sql_error("stale scan", e))?;
        let ids = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| sql_error("stale query", e))?;
        for row in ids {
            let (id, path) = row.map_err(|e| sql_error("stale row", e))?;
            if !seen.contains(&path) {
                stale.push(id);
            }
        }
    }
    for id in stale {
        remove(&tx, id)?;
    }
    meta(&tx, "source_commit", &discovered.commit)?;
    meta(&tx, "source_tree", &discovered.tree)?;
    // Query *inside the write transaction*: a concurrent revision change
    // cannot interleave and produce a mixed-revision result.
    let mut hits = Vec::new();
    {
        let mut stmt = tx
            .prepare(
                "SELECT f.path,f.blob,f.mode,source_search.body
             FROM source_search
             JOIN locator_files f ON f.id=source_search.rowid
             WHERE source_search MATCH ?1
             ORDER BY bm25(source_search,4.0,1.0), source_search.rowid LIMIT ?2",
            )
            .map_err(|e| sql_error("rank prepare", e))?;
        let rows = stmt
            .query_map(params![expression, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| sql_error("rank query", e))?;
        for row in rows {
            let (path, blob, mode, body) = row.map_err(|e| sql_error("rank row", e))?;
            if crate::digest::git_blob(body.as_bytes()) != blob {
                return refuse("cached FTS source failed Git blob identity verification");
            }
            hits.push(Hit {
                entry: git::Entry {
                    path,
                    oid: blob,
                    mode,
                    kind: "blob".into(),
                },
                body,
            });
        }
    }
    tx.commit().map_err(|e| sql_error("commit", e))?;
    Ok(Search {
        hits,
        indexed_files,
        indexed_bytes,
        skipped_files,
        truncated,
        reused_blobs,
        read_blobs,
        cache_path: path,
    })
}
