//! Incremental, segmented FTS5 index over authenticated Git source blobs.
//!
//! Each logical segment is the first byte of SHA-256(path), giving 256 stable
//! partitions regardless of checkout order. A single SQLite FTS5 table keeps
//! BM25 scores comparable across all segments. Bounded updates run in one
//! transaction; stale segments are excluded until rebuilt.
//! No source checkout files are read or modified.
use crate::{Result, discovery, git, refuse};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const CACHE_VERSION: &str = "agent-locate-segments-v3";
pub(crate) const SEGMENT_COUNT: usize = 256;
pub(crate) const DEFAULT_REFRESH_SEGMENTS: usize = 8;
pub(crate) const MAX_FILES: usize = 6000;
pub(crate) const MAX_FILE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

/// Caller-selected processing budgets. The defaults preserve existing behavior;
/// changing a budget invalidates the corresponding Git-bound cache partition.
/// These are *per index refresh* limits, not claims about repository coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IndexLimits {
    pub max_files_per_segment: usize,
    pub max_file_bytes: usize,
    pub max_bytes_per_segment: usize,
}

impl Default for IndexLimits {
    fn default() -> Self {
        Self {
            max_files_per_segment: MAX_FILES,
            max_file_bytes: MAX_FILE_BYTES,
            max_bytes_per_segment: MAX_TOTAL_BYTES,
        }
    }
}

impl IndexLimits {
    pub fn validate(self) -> Result<Self> {
        if self.max_files_per_segment == 0
            || self.max_file_bytes < 256
            || self.max_bytes_per_segment < 256
        {
            return refuse("agent locate indexing budgets must be positive");
        }
        if self.max_file_bytes > self.max_bytes_per_segment {
            return refuse("agent locate max-file-bytes must not exceed max-segment-bytes");
        }
        if [
            self.max_files_per_segment,
            self.max_file_bytes,
            self.max_bytes_per_segment,
        ]
        .iter()
        .any(|&n| i64::try_from(n).is_err())
        {
            return refuse("agent locate budget exceeds SQLite integer range");
        }
        Ok(self)
    }
}

pub(crate) struct Hit {
    pub entry: git::Entry,
    pub body: String,
    pub start_line: usize,
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
    pub total_source_files: usize,
    pub populated_segments: usize,
    pub ready_segments: usize,
    pub pending_segments: usize,
    pub refreshed_segments: usize,
}

fn sql_error(operation: &str, error: impl std::fmt::Display) -> String {
    format!("agent locate cache {operation}: {error}")
}

/// Refuse any cache inside the checkout, including symlinked ancestors.
fn external_cache(discovered: &discovery::Discovery) -> Result<PathBuf> {
    // New filename leaves the v1 index untouched during the schema upgrade.
    let path = discovered
        .index_path
        .with_file_name("agent-locate-chunks.sqlite");
    let parent = path.parent().ok_or("agent locate cache missing parent")?;
    let root = fs::canonicalize(&discovered.root).map_err(|e| sql_error("root canonicalize", e))?;
    if !parent.is_absolute()
        || parent
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
            .map_err(|e| sql_error("private cache directory permissions", e))?;
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
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS locator_segments(
            segment INTEGER PRIMARY KEY,
            fingerprint TEXT NOT NULL,
            indexed_files INTEGER NOT NULL,
            indexed_bytes INTEGER NOT NULL,
            skipped_files INTEGER NOT NULL,
            truncated INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS locator_files(
            id INTEGER PRIMARY KEY,
            segment INTEGER NOT NULL,
            path TEXT UNIQUE NOT NULL,
            blob TEXT NOT NULL,
            mode TEXT NOT NULL,
            bytes INTEGER NOT NULL,
            status TEXT NOT NULL,
            chunk_bytes INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS locator_chunks(
            id INTEGER PRIMARY KEY,
            file_id INTEGER NOT NULL,
            ordinal INTEGER NOT NULL,
            start_byte INTEGER NOT NULL,
            start_line INTEGER NOT NULL,
            UNIQUE(file_id,ordinal)
        );
        CREATE INDEX IF NOT EXISTS locator_chunks_file ON locator_chunks(file_id);
        CREATE INDEX IF NOT EXISTS locator_file_segments
            ON locator_files(segment);
        CREATE VIRTUAL TABLE IF NOT EXISTS source_search
            USING fts5(path, body, tokenize='unicode61');
        CREATE TABLE IF NOT EXISTS locator_active(
            segment INTEGER PRIMARY KEY
        );",
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
        Some(CACHE_VERSION) => Ok(()),
        None => {
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
            Ok(())
        }
        Some(_) => refuse("unsupported agent locate cache version"),
    }
}

struct CachedFile {
    id: i64,
    blob: String,
    mode: String,
    size: usize,
    status: String,
    chunk_bytes: usize,
}

struct SegmentMeta {
    fingerprint: String,
    indexed_files: usize,
    indexed_bytes: usize,
    skipped_files: usize,
    truncated: bool,
}

fn nonnegative(value: i64) -> Result<usize> {
    usize::try_from(value).map_err(|_| "negative or oversized cached count".to_owned())
}

fn previous(tx: &Transaction<'_>, path: &str) -> Result<Option<CachedFile>> {
    let row: Option<(i64, String, String, i64, String, i64)> = tx
        .query_row(
            "SELECT id,blob,mode,bytes,status,chunk_bytes FROM locator_files WHERE path=?1",
            params![path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| sql_error("select cached path", e))?;
    row.map(|(id, blob, mode, size, status, chunk_bytes)| {
        Ok(CachedFile {
            id,
            blob,
            mode,
            size: nonnegative(size)?,
            status,
            chunk_bytes: nonnegative(chunk_bytes)?,
        })
    })
    .transpose()
}

fn prior_segments(tx: &Transaction<'_>) -> Result<Vec<Option<SegmentMeta>>> {
    let mut result = (0..SEGMENT_COUNT).map(|_| None).collect::<Vec<_>>();
    let mut stmt = tx
        .prepare(
            "SELECT segment,fingerprint,indexed_files,indexed_bytes,skipped_files,truncated
         FROM locator_segments",
        )
        .map_err(|e| sql_error("segment prepare", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| sql_error("segment read", e))?;
    for row in rows {
        let (id, fingerprint, files, bytes, skipped, truncated) =
            row.map_err(|e| sql_error("segment record", e))?;
        let id = nonnegative(id)?;
        if id >= SEGMENT_COUNT || result[id].is_some() {
            return refuse("invalid or duplicate cached segment");
        }
        result[id] = Some(SegmentMeta {
            fingerprint,
            indexed_files: nonnegative(files)?,
            indexed_bytes: nonnegative(bytes)?,
            skipped_files: nonnegative(skipped)?,
            truncated: truncated != 0,
        });
    }
    Ok(result)
}

fn delete_file(tx: &Transaction<'_>, id: i64) -> Result<()> {
    tx.execute(
        "DELETE FROM source_search WHERE rowid IN
         (SELECT id FROM locator_chunks WHERE file_id=?1)",
        params![id],
    )
    .map_err(|e| sql_error("delete FTS file chunks", e))?;
    tx.execute("DELETE FROM locator_chunks WHERE file_id=?1", params![id])
        .map_err(|e| sql_error("delete file chunks", e))?;
    tx.execute("DELETE FROM locator_files WHERE id=?1", params![id])
        .map_err(|e| sql_error("delete file row", e))?;
    Ok(())
}

fn insert_chunk(
    tx: &Transaction<'_>,
    file_id: i64,
    entry: &git::Entry,
    ordinal: usize,
    start_byte: u64,
    start_line: usize,
    body: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO locator_chunks(file_id,ordinal,start_byte,start_line)
         VALUES(?1,?2,?3,?4)",
        params![
            file_id,
            ordinal as i64,
            start_byte as i64,
            start_line as i64
        ],
    )
    .map_err(|e| sql_error("insert source chunk", e))?;
    tx.execute(
        "INSERT INTO source_search(rowid,path,body) VALUES(?1,?2,?3)",
        params![tx.last_insert_rowid(), &entry.path, body],
    )
    .map_err(|e| sql_error("insert chunk FTS row", e))?;
    Ok(())
}

fn insert_file(
    tx: &Transaction<'_>,
    segment: usize,
    entry: &git::Entry,
    body: Option<&str>,
    status: &str,
    chunk_bytes: usize,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO locator_files(segment,path,blob,mode,bytes,status,chunk_bytes)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            segment as i64,
            &entry.path,
            &entry.oid,
            &entry.mode,
            body.map_or(0usize, str::len) as i64,
            status,
            chunk_bytes as i64,
        ],
    )
    .map_err(|e| sql_error("insert cached file", e))?;
    let id = tx.last_insert_rowid();
    if let Some(text) = body
        && !text.is_empty()
    {
        insert_chunk(tx, id, entry, 0, 0, 1, text)?;
    }
    Ok(id)
}

fn existing_chunks(tx: &Transaction<'_>, file_id: i64) -> Result<usize> {
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM locator_chunks WHERE file_id=?1",
            params![file_id],
            |row| row.get(0),
        )
        .map_err(|e| sql_error("count existing source chunks", e))?;
    nonnegative(count)
}

fn verify_cached_chunk(
    tx: &Transaction<'_>,
    file_id: i64,
    ordinal: usize,
    start_byte: u64,
    start_line: usize,
    body: &str,
) -> Result<()> {
    let stored: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT c.start_byte,c.start_line,s.body
         FROM locator_chunks c JOIN source_search s ON s.rowid=c.id
         WHERE c.file_id=?1 AND c.ordinal=?2",
            params![file_id, ordinal as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| sql_error("verify staged chunk", e))?;
    let Some((position, line, text)) = stored else {
        return refuse("missing previously checkpointed source chunk");
    };
    if position != start_byte as i64 || line != start_line as i64 || text != body {
        return refuse("checkpointed chunk disagrees with authenticated Git source");
    }
    Ok(())
}

fn segment_for(path: &str) -> usize {
    let digest = crate::digest::sha256(path.as_bytes());
    usize::from(u8::from_str_radix(&digest[..2], 16).expect("hex digest"))
}

/// Prefer cold partitions with lexical evidence in their Git paths.
/// Only refresh order changes; evidence and completeness remain Git-bound.
fn path_priority(entries: &[git::Entry], query: &str) -> usize {
    let words = query
        .split(" OR ")
        .map(|word| word.trim_matches('"'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    entries
        .iter()
        .map(|entry| {
            let path = entry.path.to_ascii_lowercase();
            words.iter().filter(|word| path.contains(*word)).count()
        })
        .sum()
}

fn fingerprint(entries: &[git::Entry], limits: IndexLimits) -> String {
    // Index eligibility is part of the cache identity. If size limits change,
    // unchanged Git trees must refresh previously skipped source blobs.
    let mut data = b"agent-locate-eligibility-v1\0".to_vec();
    for limit in [
        limits.max_files_per_segment,
        limits.max_file_bytes,
        limits.max_bytes_per_segment,
    ] {
        data.extend_from_slice(&(limit as u64).to_le_bytes());
    }
    for entry in entries {
        for part in [&entry.path, &entry.mode, &entry.oid] {
            data.extend_from_slice(part.as_bytes());
            data.push(0);
        }
    }
    crate::digest::sha256(&data)
}

struct RefreshState<'a> {
    repo: &'a std::path::Path,
    reader: Option<git::BlobReader>,
    reused_blobs: usize,
    read_blobs: usize,
}

fn update_segment(
    tx: &Transaction<'_>,
    segment: usize,
    entries: &[git::Entry],
    expected: &str,
    state: &mut RefreshState<'_>,
    limits: IndexLimits,
) -> Result<SegmentMeta> {
    let mut seen = BTreeSet::new();
    let mut files = 0usize;
    let mut bytes = 0usize;
    let mut refreshed_files = 0usize;
    let mut refreshed_bytes = 0usize;
    let mut skipped = 0usize;
    let mut truncated = false;
    let chunk_bytes = limits
        .max_file_bytes
        .min(limits.max_bytes_per_segment)
        .min(64 * 1024);

    for entry in entries {
        seen.insert(entry.path.as_str());
        let old = previous(tx, &entry.path)?;
        if let Some(ref cached) = old
            && cached.blob == entry.oid
            && cached.mode == entry.mode
        {
            match cached.status.as_str() {
                "OK" if cached.chunk_bytes == chunk_bytes => {
                    // A completed Git-verified index may be reused only
                    // under the same chunking policy. Different boundaries
                    // can change FTS ranking, even for the same Git blob.
                    files += 1;
                    bytes += cached.size;
                    state.reused_blobs += 1;
                    continue;
                }
                "NON_UTF8" => {
                    skipped += 1;
                    continue;
                }
                _ => {}
            }
        }

        let can_resume = old.as_ref().is_some_and(|cached| {
            cached.blob == entry.oid
                && cached.mode == entry.mode
                && cached.status == "BUDGET"
                && cached.chunk_bytes == chunk_bytes
        });
        if refreshed_files >= limits.max_files_per_segment
            || refreshed_bytes >= limits.max_bytes_per_segment
        {
            if !can_resume {
                if let Some(ref cached) = old {
                    delete_file(tx, cached.id)?;
                }
                insert_file(tx, segment, entry, None, "BUDGET", chunk_bytes)?;
            }
            skipped += 1;
            truncated = true;
            continue;
        }

        let file_id = if can_resume {
            old.as_ref().ok_or("missing resumable file")?.id
        } else {
            if let Some(ref cached) = old {
                delete_file(tx, cached.id)?;
            }
            insert_file(tx, segment, entry, None, "BUDGET", chunk_bytes)?
        };
        let resumed_chunks = if can_resume {
            existing_chunks(tx, file_id)?
        } else {
            0
        };
        if state.reader.is_none() {
            state.reader = Some(git::BlobReader::open(state.repo)?);
        }
        let mut ordinal = 0usize;
        let mut added_bytes = 0usize;
        let mut deferred = false;
        let result = state
            .reader
            .as_mut()
            .ok_or("missing Git blob reader")?
            .stream_text_chunks_verified(
                &entry.oid,
                chunk_bytes,
                |start_byte, start_line, body| {
                    if ordinal < resumed_chunks {
                        verify_cached_chunk(tx, file_id, ordinal, start_byte, start_line, body)?;
                    } else if deferred
                        || body.len()
                            > limits
                                .max_bytes_per_segment
                                .saturating_sub(refreshed_bytes + added_bytes)
                    {
                        deferred = true;
                    } else {
                        insert_chunk(tx, file_id, entry, ordinal, start_byte, start_line, body)?;
                        added_bytes += body.len();
                    }
                    ordinal += 1;
                    Ok(())
                },
            );
        let total_bytes = match result {
            Ok(size) => size,
            Err(error)
                if error.contains("invalid UTF-8")
                    || error.contains("incomplete or invalid UTF-8")
                    || error.contains("source contains a NUL byte") =>
            {
                // No claim of valid source content is made from a rejected
                // text blob. Discard any staged chunks, including old ones.
                state.reader.take();
                delete_file(tx, file_id)?;
                insert_file(tx, segment, entry, None, "NON_UTF8", 0)?;
                refreshed_files += 1;
                skipped += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        if ordinal < resumed_chunks {
            return refuse("checkpointed chunk count exceeds authenticated Git source");
        }
        state.read_blobs += 1;
        refreshed_files += 1;
        refreshed_bytes += added_bytes;
        if deferred {
            tx.execute(
                "UPDATE locator_files SET status='BUDGET' WHERE id=?1",
                params![file_id],
            )
            .map_err(|e| sql_error("checkpoint partial source", e))?;
            skipped += 1;
            truncated = true;
        } else {
            tx.execute(
                "UPDATE locator_files SET status='OK', bytes=?2 WHERE id=?1",
                params![file_id, total_bytes as i64],
            )
            .map_err(|e| sql_error("complete source", e))?;
            files += 1;
            bytes += total_bytes as usize;
        }
    }

    let mut stale_ids = Vec::new();
    {
        let mut stmt = tx
            .prepare("SELECT id,path FROM locator_files WHERE segment=?1")
            .map_err(|e| sql_error("stale segment scan", e))?;
        let rows = stmt
            .query_map(params![segment as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| sql_error("stale segment query", e))?;
        for row in rows {
            let (id, path) = row.map_err(|e| sql_error("stale segment row", e))?;
            if !seen.contains(path.as_str()) {
                stale_ids.push(id);
            }
        }
    }
    for id in stale_ids {
        delete_file(tx, id)?;
    }
    tx.execute(
        "INSERT INTO locator_segments
         (segment,fingerprint,indexed_files,indexed_bytes,skipped_files,truncated)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(segment) DO UPDATE SET
            fingerprint=excluded.fingerprint,
            indexed_files=excluded.indexed_files,
            indexed_bytes=excluded.indexed_bytes,
            skipped_files=excluded.skipped_files,
            truncated=excluded.truncated",
        params![
            segment as i64,
            expected,
            files as i64,
            bytes as i64,
            skipped as i64,
            if truncated { 1_i64 } else { 0_i64 },
        ],
    )
    .map_err(|e| sql_error("segment commit metadata", e))?;
    Ok(SegmentMeta {
        fingerprint: expected.to_owned(),
        indexed_files: files,
        indexed_bytes: bytes,
        skipped_files: skipped,
        truncated,
    })
}

fn put_meta(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO locator_meta(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|e| sql_error("snapshot metadata", e))?;
    Ok(())
}

/// Independently authenticate an indexed excerpt against the full Git blob.
fn verify_source_window(
    repo: &std::path::Path,
    oid: &str,
    start_byte: u64,
    start_line: usize,
    body: &str,
) -> Result<()> {
    let expected = body.as_bytes();
    let end = start_byte
        .checked_add(expected.len() as u64)
        .ok_or("cached source chunk offset overflow")?;
    let mut checked = 0usize;
    let mut authenticated_line = 1usize;
    let mut reader = git::BlobReader::open(repo)?;
    let total = reader.stream_verified(oid, 64 * 1024, |offset, bytes| {
        let upper = offset + bytes.len() as u64;
        if offset < start_byte {
            let before = (start_byte - offset).min(bytes.len() as u64) as usize;
            authenticated_line += bytes[..before].iter().filter(|&&byte| byte == b'\n').count();
        }
        let a = offset.max(start_byte);
        let b = upper.min(end);
        if a < b {
            let chunk_start = (a - offset) as usize;
            let body_start = (a - start_byte) as usize;
            let count = (b - a) as usize;
            if bytes[chunk_start..chunk_start + count] != expected[body_start..body_start + count] {
                return refuse("cached source chunk disagrees with pinned Git blob");
            }
            checked += count;
        }
        Ok(())
    })?;
    reader.finish()?;
    if end > total || checked != expected.len() {
        return refuse("cached chunk lies outside authenticated Git blob");
    }
    if authenticated_line != start_line {
        return refuse("cached source chunk line differs from authenticated Git blob");
    }
    Ok(())
}

/// Refresh up to refresh_budget changed logical segments. Query results
/// always exclude any segment not matching the requested Git tree inventory.
/// A cold repository may need repeated calls or --index-segments 256.
#[cfg(test)]
pub(crate) fn search(
    discovered: &discovery::Discovery,
    entries: Vec<git::Entry>,
    limit: usize,
    expression: &str,
    refresh_budget: usize,
    source_path: impl Fn(&str) -> bool,
) -> Result<Search> {
    search_with_limits(
        discovered,
        entries,
        limit,
        expression,
        refresh_budget,
        source_path,
        IndexLimits::default(),
    )
}

pub(crate) fn search_with_limits(
    discovered: &discovery::Discovery,
    entries: Vec<git::Entry>,
    limit: usize,
    expression: &str,
    refresh_budget: usize,
    source_path: impl Fn(&str) -> bool,
    limits: IndexLimits,
) -> Result<Search> {
    let limits = limits.validate()?;
    if !(1..=SEGMENT_COUNT).contains(&refresh_budget) {
        return refuse("agent locate --index-segments must be 1..=256");
    }
    let mut groups: Vec<Vec<git::Entry>> = (0..SEGMENT_COUNT).map(|_| Vec::new()).collect();
    for entry in entries {
        if entry.kind == "blob"
            && matches!(entry.mode.as_str(), "100644" | "100755")
            && source_path(&entry.path)
        {
            let segment = segment_for(&entry.path);
            groups[segment].push(entry);
        }
    }
    for group in &mut groups {
        group.sort_by(|a, b| a.path.cmp(&b.path));
    }
    let total_source_files = groups.iter().map(Vec::len).sum();
    let populated_segments = groups.iter().filter(|x| !x.is_empty()).count();
    let fingerprints = groups
        .iter()
        .map(|group| fingerprint(group, limits))
        .collect::<Vec<_>>();

    let path = external_cache(discovered)?;
    let mut db = Connection::open(&path).map_err(|e| sql_error("open", e))?;
    db.busy_timeout(Duration::from_secs(15))
        .map_err(|e| sql_error("busy timeout", e))?;
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| sql_error("begin", e))?;
    initialize(&tx)?;
    let mut prior = prior_segments(&tx)?;
    let mut pending = (0..SEGMENT_COUNT)
        .filter(|&id| {
            prior[id]
                .as_ref()
                .is_some_and(|row| row.fingerprint != fingerprints[id] || row.truncated)
                || (prior[id].is_none() && !groups[id].is_empty())
        })
        .collect::<Vec<_>>();
    pending.sort_by_key(|&id| {
        (
            groups[id].is_empty(),
            std::cmp::Reverse(path_priority(&groups[id], expression)),
            id,
        )
    });
    let mut state = RefreshState {
        repo: &discovered.root,
        reader: None,
        reused_blobs: 0,
        read_blobs: 0,
    };
    let mut refreshed_segments = 0usize;
    for id in pending.iter().take(refresh_budget).copied() {
        let updated = update_segment(&tx, id, &groups[id], &fingerprints[id], &mut state, limits)?;
        prior[id] = Some(updated);
        refreshed_segments += 1;
        // Persist this unit of work before moving to the next partition.
        // A workflow timeout or process crash cannot erase prior segments.
        tx.commit()
            .map_err(|e| sql_error("segment checkpoint", e))?;
        tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| sql_error("resume transaction", e))?;
    }
    if let Some(reader) = state.reader.take() {
        reader.finish()?;
    }
    let mut reused_blobs = state.reused_blobs;
    let read_blobs = state.read_blobs;

    tx.execute("DELETE FROM locator_active", [])
        .map_err(|e| sql_error("clear active segments", e))?;
    let mut ready_segments = 0usize;
    let mut indexed_files = 0usize;
    let mut indexed_bytes = 0usize;
    let mut skipped_files = 0usize;
    let mut truncated = false;
    for id in 0..SEGMENT_COUNT {
        if let Some(ref snapshot) = prior[id]
            && snapshot.fingerprint == fingerprints[id]
        {
            tx.execute(
                "INSERT INTO locator_active(segment) VALUES(?1)",
                params![id as i64],
            )
            .map_err(|e| sql_error("activate segment", e))?;
            if !groups[id].is_empty() {
                ready_segments += 1;
            }
            indexed_files += snapshot.indexed_files;
            indexed_bytes += snapshot.indexed_bytes;
            skipped_files += snapshot.skipped_files;
            truncated |= snapshot.truncated;
        }
    }
    let pending_segments = (0..SEGMENT_COUNT)
        .filter(|&id| {
            !groups[id].is_empty()
                && prior[id]
                    .as_ref()
                    .is_none_or(|row| row.fingerprint != fingerprints[id] || row.truncated)
        })
        .count();
    truncated |= pending_segments > 0;
    reused_blobs += indexed_files.saturating_sub(read_blobs + reused_blobs);
    put_meta(&tx, "source_commit", &discovered.commit)?;
    put_meta(&tx, "source_tree", &discovered.tree)?;

    let mut hits = Vec::new();
    let mut returned_paths = BTreeSet::new();
    {
        let mut stmt = tx
            .prepare(
                "SELECT f.path,f.blob,f.mode,c.start_byte,c.start_line,source_search.body
                 FROM source_search
                 JOIN locator_chunks c ON c.id=source_search.rowid
                 JOIN locator_files f ON f.id=c.file_id
                 JOIN locator_active a ON a.segment=f.segment
                 WHERE source_search MATCH ?1 AND f.status='OK'
                 ORDER BY bm25(source_search,4.0,1.0), f.path, c.ordinal",
            )
            .map_err(|e| sql_error("ranking prepare", e))?;
        let mut rows = stmt
            .query(params![expression])
            .map_err(|e| sql_error("ranking query", e))?;
        while let Some(row) = rows.next().map_err(|e| sql_error("ranked row", e))? {
            let name: String = row.get(0).map_err(|e| sql_error("ranked path", e))?;
            if !returned_paths.insert(name.clone()) {
                continue;
            }
            let blob: String = row.get(1).map_err(|e| sql_error("ranked blob", e))?;
            let mode: String = row.get(2).map_err(|e| sql_error("ranked mode", e))?;
            let start_byte = nonnegative(row.get(3).map_err(|e| sql_error("chunk offset", e))?)?;
            let start_line = nonnegative(row.get(4).map_err(|e| sql_error("chunk line", e))?)?;
            let body: String = row.get(5).map_err(|e| sql_error("ranked body", e))?;
            let segment = segment_for(&name);
            let pinned = groups[segment]
                .binary_search_by(|entry| entry.path.as_str().cmp(name.as_str()))
                .map(|index| &groups[segment][index])
                .map_err(|_| "cached FTS source absent from pinned Git tree")?;
            if pinned.oid != blob || pinned.mode != mode {
                return refuse("cached FTS path/blob/mode differs from pinned Git tree");
            }
            // A chunk is not itself a Git blob. Re-stream the entire original
            // object to independently verify both its hash and the exact
            // cached byte window before exposing source text to a caller.
            verify_source_window(&discovered.root, &blob, start_byte as u64, start_line, &body)?;
            hits.push(Hit {
                entry: git::Entry {
                    path: name,
                    oid: blob,
                    mode,
                    kind: "blob".into(),
                },
                body,
                start_line,
            });
            if hits.len() == limit {
                break;
            }
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
        total_source_files,
        populated_segments,
        ready_segments,
        pending_segments,
        refreshed_segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn git_at(root: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("start git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run(root: &std::path::Path, rev: &str, budget: usize) -> Search {
        let discovered = discovery::inspect(root, rev).expect("discover");
        let entries = git::entries(root, &discovered.commit).expect("Git entries");
        search(&discovered, entries, 64, "\"needle\"", budget, |path| {
            path.ends_with(".rs")
        })
        .expect("indexed lookup")
    }

    #[test]
    fn partition_digest_is_stable_and_distributed() {
        assert_eq!(segment_for("src/alpha.rs"), segment_for("src/alpha.rs"));
        let partitions = (0..128)
            .map(|number| segment_for(&format!("src/module_{number:04}.rs")))
            .collect::<BTreeSet<_>>();
        assert!(partitions.len() > 64);
        let mut distribution = [0usize; SEGMENT_COUNT];
        for number in 0..20_000 {
            distribution[segment_for(&format!("src/module_{number:05}.rs"))] += 1;
        }
        assert_eq!(distribution.iter().sum::<usize>(), 20_000);
        assert!(distribution.iter().all(|count| *count < MAX_FILES));
        assert!(
            path_priority(
                &[git::Entry {
                    path: "src/lease_recovery.rs".into(),
                    mode: "100644".into(),
                    kind: "blob".into(),
                    oid: "0".repeat(40),
                }],
                "\"lease\" OR \"recover\"",
            ) > 0
        );
    }

    #[test]
    fn indexes_source_larger_than_the_previous_256_kib_limit() {
        let root = std::env::temp_dir().join(format!(
            "excession-large-source-locate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);

        let source = format!(
            "{}pub fn planner() {{ let needle = 42; }}\n",
            "// ordinary source line\n".repeat(18_000)
        );
        assert!(source.len() > 256 * 1024);
        assert!(source.len() < MAX_FILE_BYTES);
        fs::write(root.join("src/planner.rs"), source).unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "large source",
            ],
        );
        let found = run(&root, "HEAD", SEGMENT_COUNT);
        assert_eq!(found.total_source_files, 1);
        assert_eq!(found.indexed_files, 1);
        assert_eq!(found.skipped_files, 0);
        assert_eq!(found.pending_segments, 0);
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].entry.path, "src/planner.rs");
        assert!(found.hits[0].body.contains("let needle = 42;"));
        assert_eq!(
            found.hits[0].entry.oid,
            git::blob(&root, "HEAD", "src/planner.rs").unwrap(),
        );

        fs::remove_dir_all(found.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_budget_rechecks_large_blob_at_the_same_git_revision() {
        let root = std::env::temp_dir().join(format!(
            "excession-configured-source-locate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        fs::write(
            root.join("src/planner.rs"),
            format!(
                "{}pub fn planner() {{ let needle = 42; }}\n",
                "// source text\n".repeat(25_000)
            ),
        )
        .unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "large blob",
            ],
        );
        let query = |max_file_bytes| {
            let discovered = discovery::inspect(&root, "HEAD").unwrap();
            let entries = git::entries(&root, &discovered.commit).unwrap();
            search_with_limits(
                &discovered,
                entries,
                8,
                "\"needle\"",
                SEGMENT_COUNT,
                |p| p.ends_with(".rs"),
                IndexLimits {
                    max_file_bytes,
                    max_files_per_segment: 6000,
                    max_bytes_per_segment: 1024 * 1024,
                },
            )
            .unwrap()
        };
        let small = query(128 * 1024);
        assert_eq!(small.indexed_files, 1);
        assert_eq!(small.skipped_files, 0);
        assert_eq!(small.hits.len(), 1);
        let large = query(512 * 1024);
        assert_eq!(large.indexed_files, 1);
        assert_eq!(large.skipped_files, 0);
        assert_eq!(large.hits[0].entry.path, "src/planner.rs");
        assert_eq!(
            large.hits[0].entry.oid,
            git::blob(&root, "HEAD", "src/planner.rs").unwrap()
        );
        let smaller_again = query(128 * 1024);
        assert_eq!(smaller_again.indexed_files, 1);
        assert_eq!(smaller_again.skipped_files, 0);
        fs::remove_dir_all(large.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn segment_budget_is_checkpointed_and_resumes_to_full_coverage() {
        let root = std::env::temp_dir().join(format!(
            "excession-budget-resume-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        let first_path = "src/same-bucket-0000.rs".to_owned();
        let bucket = segment_for(&first_path);
        let paths = std::iter::once(first_path)
            .chain((1..10_000).map(|i| format!("src/same-bucket-{i:04}.rs")))
            .filter(|p| segment_for(p) == bucket)
            .take(3)
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 3);
        for (i, path) in paths.iter().enumerate() {
            fs::write(root.join(path), format!("fn needle_{i}() {{}}\n")).unwrap();
        }
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "three in same segment",
            ],
        );
        let run = || {
            let discovered = discovery::inspect(&root, "HEAD").unwrap();
            let entries = git::entries(&root, &discovered.commit).unwrap();
            search_with_limits(
                &discovered,
                entries,
                16,
                "\"needle\"",
                1,
                |p| p.ends_with(".rs"),
                IndexLimits {
                    max_files_per_segment: 1,
                    max_file_bytes: 4096,
                    max_bytes_per_segment: 4096,
                },
            )
            .unwrap()
        };
        let a = run();
        assert_eq!(a.indexed_files, 1);
        assert_eq!(a.pending_segments, 1);
        assert!(a.truncated);
        let b = run();
        assert_eq!(b.indexed_files, 2);
        assert_eq!(b.pending_segments, 1);
        let c = run();
        assert_eq!(c.indexed_files, 3);
        assert_eq!(c.skipped_files, 0);
        assert_eq!(c.pending_segments, 0);
        assert_eq!(c.hits.len(), 3);
        assert!(!c.truncated);
        fs::remove_dir_all(c.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chunk_policy_switch_rebuilds_deterministic_fts_windows() {
        let root = std::env::temp_dir().join(format!(
            "excession-chunk-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        fs::write(
            root.join("src/planner.rs"),
            format!("{}needle planner\n", "long identifier \n".repeat(500)),
        )
        .unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "indexed",
            ],
        );
        let query = |size| {
            let discovered = discovery::inspect(&root, "HEAD").unwrap();
            let entries = git::entries(&root, &discovered.commit).unwrap();
            search_with_limits(
                &discovered,
                entries,
                8,
                "\"needle\"",
                SEGMENT_COUNT,
                |path| path.ends_with(".rs"),
                IndexLimits {
                    max_file_bytes: size,
                    max_bytes_per_segment: 65536,
                    max_files_per_segment: 16,
                },
            )
            .unwrap()
        };
        let small = query(256);
        assert_eq!(small.indexed_files, 1);
        let large = query(4096);
        assert_eq!(large.indexed_files, 1);
        assert_eq!(large.read_blobs, 1);
        let restored = query(256);
        assert_eq!(restored.read_blobs, 1);
        assert_eq!(restored.indexed_files, 1);
        assert_eq!(small.hits[0].body, restored.hits[0].body);
        assert_eq!(small.hits[0].start_line, restored.hits[0].start_line);
        assert_eq!(small.hits[0].entry.oid, restored.hits[0].entry.oid);
        fs::remove_dir_all(restored.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_single_line_source_keeps_searchable_tokens_across_chunk_boundaries() {
        let root = std::env::temp_dir().join(format!(
            "excession-long-line-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        let source = format!("{} needle {} \n", "q".repeat(220), "z".repeat(2000));
        fs::write(root.join("src/minified.rs"), &source).unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "minified",
            ],
        );
        let discovered = discovery::inspect(&root, "HEAD").unwrap();
        let entries = git::entries(&root, &discovered.commit).unwrap();
        let found = search_with_limits(
            &discovered,
            entries,
            4,
            "\"needle\"",
            SEGMENT_COUNT,
            |path| path.ends_with(".rs"),
            IndexLimits {
                max_file_bytes: 256,
                max_files_per_segment: 10,
                max_bytes_per_segment: 4096,
            },
        )
        .unwrap();
        assert_eq!(found.indexed_files, 1);
        assert_eq!(found.skipped_files, 0);
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].entry.path, "src/minified.rs");
        assert!(found.hits[0].body.contains("needle"));
        assert_eq!(found.hits[0].start_line, 1);
        fs::remove_dir_all(found.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkpoints_one_large_source_in_chunks_until_fully_verified() {
        let root = std::env::temp_dir().join(format!(
            "excession-chunk-progress-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        let source = format!("{}fn needle() {{}} \n", "/* α🚀 filler */\n".repeat(1000));
        assert!(source.len() > 8 * 1024);
        fs::write(root.join("src/huge.rs"), &source).unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "large source",
            ],
        );
        let query = || {
            let discovered = discovery::inspect(&root, "HEAD").unwrap();
            let entries = git::entries(&root, &discovered.commit).unwrap();
            search_with_limits(
                &discovered,
                entries,
                4,
                "\"needle\"",
                1,
                |path| path.ends_with(".rs"),
                IndexLimits {
                    max_file_bytes: 256,
                    max_files_per_segment: 1,
                    max_bytes_per_segment: 1024,
                },
            )
            .unwrap()
        };
        let first = query();
        assert_eq!(first.indexed_files, 0);
        assert_eq!(first.skipped_files, 1);
        assert_eq!(first.pending_segments, 1);
        assert!(first.hits.is_empty());
        let mut complete = first;
        let mut rounds = 1;
        while complete.pending_segments != 0 && rounds < 40 {
            complete = query();
            rounds += 1;
        }
        assert!(rounds > 5);
        assert!(rounds < 40);
        assert_eq!(complete.pending_segments, 0);
        assert_eq!(complete.indexed_files, 1);
        assert_eq!(complete.skipped_files, 0);
        assert_eq!(complete.hits.len(), 1);
        assert!(complete.hits[0].body.contains("needle"));
        assert!(complete.hits[0].start_line > 900);
        assert_eq!(
            complete.hits[0].entry.oid,
            git::blob(&root, "HEAD", "src/huge.rs").unwrap(),
        );
        assert!(!complete.truncated);
        let position = source.find("fn needle").unwrap() as u64;
        let oid = &complete.hits[0].entry.oid;
        assert!(verify_source_window(&root, oid, position, 1001, "fn needle").is_ok());
        assert!(verify_source_window(&root, oid, position, 1000, "fn needle").is_err());
        assert!(verify_source_window(&root, oid, position, 1001, "fn forged").is_err());

        // A tampered chunk must never be returned as an authenticated source.
        let db = Connection::open(&complete.cache_path).unwrap();
        db.execute(
            "UPDATE source_search SET body='needle forged' WHERE rowid=
             (SELECT id FROM locator_chunks ORDER BY id DESC LIMIT 1)",
            [],
        )
        .unwrap();
        drop(db);
        let discovered = discovery::inspect(&root, "HEAD").unwrap();
        let entries = git::entries(&root, &discovered.commit).unwrap();
        let failed = search_with_limits(
            &discovered,
            entries,
            4,
            "\"needle\"",
            1,
            |path| path.ends_with(".rs"),
            IndexLimits {
                max_file_bytes: 256,
                max_files_per_segment: 1,
                max_bytes_per_segment: 1024,
            },
        );
        assert!(failed.is_err(), "tampered FTS text must fail closed");
        fs::remove_dir_all(complete.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_indexing_budgets() {
        let base = IndexLimits::default();
        assert!(
            IndexLimits {
                max_file_bytes: 0,
                ..base
            }
            .validate()
            .is_err()
        );
        assert!(
            IndexLimits {
                max_file_bytes: base.max_bytes_per_segment + 1,
                ..base
            }
            .validate()
            .is_err()
        );
        assert!(
            IndexLimits {
                max_files_per_segment: 0,
                ..base
            }
            .validate()
            .is_err()
        );
        assert!(
            IndexLimits {
                max_bytes_per_segment: 0,
                ..base
            }
            .validate()
            .is_err()
        );
        assert!(
            IndexLimits {
                max_file_bytes: 3,
                ..base
            }
            .validate()
            .is_err()
        );
        assert!(base.validate().is_ok());
    }

    #[test]
    fn progresses_across_partitions_and_rejects_stale_revisions() {
        let root = std::env::temp_dir().join(format!(
            "excession-segment-index-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        for number in 0..24 {
            fs::write(
                root.join(format!("src/module_{number:02}.rs")),
                format!("pub fn module_{number:02}() {{ let needle = 1; }}\n"),
            )
            .unwrap();
        }
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "initial",
            ],
        );
        let first = run(&root, "HEAD", 1);
        assert_eq!(first.total_source_files, 24);
        assert!(first.pending_segments > 8);
        assert!(first.ready_segments < first.populated_segments);
        assert!(first.truncated);
        assert!(first.indexed_files < 24);
        assert_eq!(first.read_blobs, first.indexed_files);
        // No stale segment may appear in returned evidence.
        assert_eq!(first.hits.len(), first.indexed_files);

        let mut remaining = first.pending_segments;
        for _ in 0..64 {
            if remaining == 0 {
                break;
            }
            let next = run(&root, "HEAD", 1);
            assert_eq!(next.pending_segments + 1, remaining);
            remaining = next.pending_segments;
        }
        assert_eq!(remaining, 0);
        let complete = run(&root, "HEAD", 1);
        assert_eq!(complete.indexed_files, 24);
        assert_eq!(complete.hits.len(), 24);
        assert_eq!(complete.read_blobs, 0);
        assert_eq!(complete.reused_blobs, 24);
        assert!(!complete.truncated);
        assert_eq!(complete.ready_segments, complete.populated_segments);

        // Only the changed partition needs a new Git blob read.
        fs::write(
            root.join("src/module_23.rs"),
            "pub fn module_23() { let unrelated = 3; }\n",
        )
        .unwrap();
        git_at(&root, &["add", "src/module_23.rs"]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "update source",
            ],
        );
        let changed = run(&root, "HEAD", 1);
        assert_eq!(changed.pending_segments, 0);
        assert_eq!(changed.read_blobs, 1);
        assert_eq!(changed.indexed_files, 24);
        assert_eq!(changed.hits.len(), 23);

        // Dirty edits are never read. The exact source tree is unchanged.
        fs::write(
            root.join("src/module_23.rs"),
            "pub fn module_23() { let needle = 1; }\n",
        )
        .unwrap();
        let dirty = run(&root, "HEAD", 1);
        assert_eq!(dirty.read_blobs, 0);
        assert_eq!(dirty.hits.len(), 23);

        // A deleted file must disappear from the next transaction's results.
        git_at(&root, &["rm", "-q", "src/module_22.rs"]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "delete source",
            ],
        );
        let deleted = run(&root, "HEAD", 1);
        assert_eq!(deleted.pending_segments, 0);
        assert_eq!(deleted.total_source_files, 23);
        assert_eq!(deleted.indexed_files, 23);
        assert_eq!(deleted.hits.len(), 22);

        // Old revisions are independently authenticated and reindexed.
        let old = run(&root, "HEAD~2", 1);
        assert_eq!(old.pending_segments, 1);
        let old = run(&root, "HEAD~2", 1);
        assert_eq!(old.pending_segments, 0);
        assert_eq!(old.hits.len(), 24);

        // No caller-controlled cache is written inside the Git checkout.
        let discovered = discovery::inspect(&root, "HEAD").unwrap();
        assert!(
            !discovered
                .root
                .join("agent-locate-segments.sqlite")
                .exists()
        );
        let cache_dir = old.cache_path.parent().unwrap();
        fs::remove_dir_all(cache_dir).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refuses_cached_valid_blob_under_wrong_git_path() {
        let root = std::env::temp_dir().join(format!(
            "excession-cache-identity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        fs::write(root.join("src/a.rs"), "pub fn a() { let needle = 1; }\n").unwrap();
        let second_body = "pub fn b() { let needle = 2; }\n";
        fs::write(root.join("src/b.rs"), second_body).unwrap();
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "initial",
            ],
        );
        let first = run(&root, "HEAD", SEGMENT_COUNT);
        assert_eq!(first.hits.len(), 2);
        let blob_b = git::blob(&root, "HEAD", "src/b.rs").unwrap();
        let db = Connection::open(&first.cache_path).unwrap();
        let id: i64 = db
            .query_row(
                "SELECT id FROM locator_files WHERE path='src/a.rs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.execute(
            "UPDATE locator_files SET blob=?1 WHERE id=?2",
            params![blob_b, id],
        )
        .unwrap();
        db.execute(
            "UPDATE source_search SET body=?1 WHERE rowid=?2",
            params![second_body, id],
        )
        .unwrap();
        drop(db);
        let discovered = discovery::inspect(&root, "HEAD").unwrap();
        let result = search(
            &discovered,
            git::entries(&root, &discovered.commit).unwrap(),
            64,
            "\"needle\"",
            1,
            |path| path.ends_with(".rs"),
        );
        assert!(result.is_err());
        fs::remove_dir_all(first.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "large real-Git stress fixture; run in dedicated deep CI"]
    fn indexes_more_than_the_previous_repository_wide_cap() {
        let root = std::env::temp_dir().join(format!(
            "excession-large-locate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        git_at(&root, &["init", "-q"]);
        for number in 0..6_400 {
            fs::write(
                root.join(format!("src/module_{number:05}.rs")),
                format!("pub fn module_{number:05}() {{ let needle = 1; }}\n"),
            )
            .unwrap();
        }
        git_at(&root, &["add", "."]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "large initial commit",
            ],
        );
        let started = std::time::Instant::now();
        let cold = run(&root, "HEAD", SEGMENT_COUNT);
        let cold_time = started.elapsed();
        assert_eq!(cold.total_source_files, 6_400);
        assert_eq!(cold.indexed_files, 6_400);
        assert_eq!(cold.read_blobs, 6_400);
        assert_eq!(cold.pending_segments, 0);
        assert!(!cold.truncated);
        assert_eq!(cold.hits.len(), 64);

        let started = std::time::Instant::now();
        let warm = run(&root, "HEAD", DEFAULT_REFRESH_SEGMENTS);
        let warm_time = started.elapsed();
        assert_eq!(warm.indexed_files, 6_400);
        assert_eq!(warm.read_blobs, 0);
        assert_eq!(warm.reused_blobs, 6_400);
        assert_eq!(warm.pending_segments, 0);

        fs::write(
            root.join("src/module_06399.rs"),
            "pub fn module_06399() { let needle = 2; }\n",
        )
        .unwrap();
        git_at(&root, &["add", "src/module_06399.rs"]);
        git_at(
            &root,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test",
                "commit",
                "-qm",
                "one source edit",
            ],
        );
        let started = std::time::Instant::now();
        let changed = run(&root, "HEAD", 1);
        let changed_time = started.elapsed();
        assert_eq!(changed.indexed_files, 6_400);
        assert_eq!(changed.read_blobs, 1);
        assert_eq!(changed.pending_segments, 0);
        eprintln!(
            "segment-scale files=6400 cold_ms={} warm_ms={} changed_ms={} cold_blobs={} warm_blobs={} changed_blobs={}",
            cold_time.as_millis(),
            warm_time.as_millis(),
            changed_time.as_millis(),
            cold.read_blobs,
            warm.read_blobs,
            changed.read_blobs
        );
        fs::remove_dir_all(changed.cache_path.parent().unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
