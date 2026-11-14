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

const CACHE_VERSION: &str = "agent-locate-segments-v2";
pub(crate) const SEGMENT_COUNT: usize = 256;
pub(crate) const DEFAULT_REFRESH_SEGMENTS: usize = 8;
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
        .with_file_name("agent-locate-segments.sqlite");
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
            status TEXT NOT NULL
        );
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
    let row: Option<(i64, String, String, i64, String)> = tx
        .query_row(
            "SELECT id,blob,mode,bytes,status FROM locator_files WHERE path=?1",
            params![path],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| sql_error("select cached path", e))?;
    row.map(|(id, blob, mode, size, status)| {
        Ok(CachedFile {
            id,
            blob,
            mode,
            size: nonnegative(size)?,
            status,
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
    tx.execute("DELETE FROM source_search WHERE rowid=?1", params![id])
        .map_err(|e| sql_error("delete FTS row", e))?;
    tx.execute("DELETE FROM locator_files WHERE id=?1", params![id])
        .map_err(|e| sql_error("delete file row", e))?;
    Ok(())
}

fn insert_file(
    tx: &Transaction<'_>,
    segment: usize,
    entry: &git::Entry,
    body: Option<&str>,
    status: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO locator_files(segment,path,blob,mode,bytes,status)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            segment as i64,
            &entry.path,
            &entry.oid,
            &entry.mode,
            body.map_or(0usize, str::len) as i64,
            status
        ],
    )
    .map_err(|e| sql_error("insert cached file", e))?;
    if let Some(text) = body {
        tx.execute(
            "INSERT INTO source_search(rowid,path,body) VALUES(?1,?2,?3)",
            params![tx.last_insert_rowid(), &entry.path, text],
        )
        .map_err(|e| sql_error("insert FTS row", e))?;
    }
    Ok(())
}

fn segment_for(path: &str) -> usize {
    let digest = crate::digest::sha256(path.as_bytes());
    usize::from(u8::from_str_radix(&digest[..2], 16).expect("hex digest"))
}

fn fingerprint(entries: &[git::Entry]) -> String {
    let mut data = Vec::new();
    for entry in entries {
        for part in [&entry.path, &entry.mode, &entry.oid] {
            data.extend_from_slice(part.as_bytes());
            data.push(0);
        }
    }
    crate::digest::sha256(&data)
}

fn update_segment(
    tx: &Transaction<'_>,
    segment: usize,
    entries: &[git::Entry],
    expected: &str,
    repo: &std::path::Path,
    reader: &mut Option<git::BlobReader>,
    reused_blobs: &mut usize,
    read_blobs: &mut usize,
) -> Result<SegmentMeta> {
    let mut seen = BTreeSet::new();
    let mut files = 0usize;
    let mut bytes = 0usize;
    let mut skipped = 0usize;
    let mut truncated = false;
    for entry in entries {
        seen.insert(entry.path.as_str());
        if files + skipped >= MAX_FILES || bytes >= MAX_TOTAL_BYTES {
            skipped += 1;
            truncated = true;
            if let Some(old) = previous(tx, &entry.path)? {
                delete_file(tx, old.id)?;
            }
            continue;
        }
        let old = previous(tx, &entry.path)?;
        if let Some(ref cached) = old
            && cached.blob == entry.oid
            && cached.mode == entry.mode
        {
            match cached.status.as_str() {
                "OK" if cached.size <= MAX_TOTAL_BYTES - bytes => {
                    files += 1;
                    bytes += cached.size;
                    *reused_blobs += 1;
                    continue;
                }
                "LARGE" | "NON_UTF8" => {
                    skipped += 1;
                    continue;
                }
                _ => {}
            }
        }
        if let Some(cached) = old {
            delete_file(tx, cached.id)?;
        }
        if reader.is_none() {
            *reader = Some(git::BlobReader::open(repo)?);
        }
        let data = match reader
            .as_mut()
            .ok_or("missing Git blob reader")?
            .read_capped(&entry.oid, MAX_FILE_BYTES)
        {
            Ok(data) => data,
            Err(err) if err == "Git source blob exceeds byte limit" => {
                skipped += 1;
                insert_file(tx, segment, entry, None, "LARGE")?;
                continue;
            }
            Err(err) => return Err(err),
        };
        *read_blobs += 1;
        let Ok(body) = String::from_utf8(data) else {
            skipped += 1;
            insert_file(tx, segment, entry, None, "NON_UTF8")?;
            continue;
        };
        if body.contains('\0') {
            skipped += 1;
            insert_file(tx, segment, entry, None, "NON_UTF8")?;
            continue;
        }
        if body.len() > MAX_TOTAL_BYTES - bytes {
            skipped += 1;
            truncated = true;
            insert_file(tx, segment, entry, None, "BUDGET")?;
            continue;
        }
        files += 1;
        bytes += body.len();
        insert_file(tx, segment, entry, Some(&body), "OK")?;
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

/// Refresh up to refresh_budget changed logical segments. Query results
/// always exclude any segment not matching the requested Git tree inventory.
/// A cold repository may need repeated calls or --index-segments 256.
pub(crate) fn search(
    discovered: &discovery::Discovery,
    entries: Vec<git::Entry>,
    limit: usize,
    expression: &str,
    refresh_budget: usize,
    source_path: impl Fn(&str) -> bool,
) -> Result<Search> {
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
        .map(|group| fingerprint(group))
        .collect::<Vec<_>>();

    let path = external_cache(discovered)?;
    let mut db = Connection::open(&path).map_err(|e| sql_error("open", e))?;
    db.busy_timeout(Duration::from_secs(15))
        .map_err(|e| sql_error("busy timeout", e))?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| sql_error("begin", e))?;
    initialize(&tx)?;
    let mut prior = prior_segments(&tx)?;
    let mut pending = (0..SEGMENT_COUNT)
        .filter(|&id| {
            prior[id]
                .as_ref()
                .is_some_and(|row| row.fingerprint != fingerprints[id])
                || (prior[id].is_none() && !groups[id].is_empty())
        })
        .collect::<Vec<_>>();
    pending.sort_by_key(|&id| (groups[id].is_empty(), id));
    let mut reader = None;
    let mut reused_blobs = 0usize;
    let mut read_blobs = 0usize;
    let mut refreshed_segments = 0usize;
    for id in pending.iter().take(refresh_budget).copied() {
        let updated = update_segment(
            &tx,
            id,
            &groups[id],
            &fingerprints[id],
            &discovered.root,
            &mut reader,
            &mut reused_blobs,
            &mut read_blobs,
        )?;
        prior[id] = Some(updated);
        refreshed_segments += 1;
    }
    if let Some(reader) = reader {
        reader.finish()?;
    }

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
    let pending_segments = pending.len().saturating_sub(refreshed_segments);
    truncated |= pending_segments > 0;
    reused_blobs += indexed_files.saturating_sub(read_blobs + reused_blobs);
    put_meta(&tx, "source_commit", &discovered.commit)?;
    put_meta(&tx, "source_tree", &discovered.tree)?;

    let mut hits = Vec::new();
    {
        let mut stmt = tx
            .prepare(
                "SELECT f.path,f.blob,f.mode,source_search.body
             FROM source_search
             JOIN locator_files f ON f.id=source_search.rowid
             JOIN locator_active a ON a.segment=f.segment
             WHERE source_search MATCH ?1
             ORDER BY bm25(source_search,4.0,1.0), source_search.rowid
             LIMIT ?2",
            )
            .map_err(|e| sql_error("ranking prepare", e))?;
        let rows = stmt
            .query_map(params![expression, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| sql_error("ranking query", e))?;
        for row in rows {
            let (name, blob, mode, body) = row.map_err(|e| sql_error("ranked row", e))?;
            if crate::digest::git_blob(body.as_bytes()) != blob {
                return refuse("cached FTS source failed Git blob identity verification");
            }
            hits.push(Hit {
                entry: git::Entry {
                    path: name,
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
}
