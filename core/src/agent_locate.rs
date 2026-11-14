//! Evidence-bound, model-free task-to-source discovery for coding agents.
//!
//! The index is ephemeral, derived exclusively from authenticated Git blobs.
//! FTS5 ranks *textual candidates*, not validated program behavior. Output
//! preserves the boundary between lexical matches and native observations.

use crate::json::{Value, obj};
use crate::{Result, agent, discovery, git, refuse};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_FILES: usize = 6000;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_QUERY_TERMS: usize = 16;
const MAX_EXCERPT_LINES: usize = 7;
const MAX_CONTEXT_FOCUS: usize = 16;

struct IndexedFile {
    entry: git::Entry,
    body: String,
}

/// Do not silently index dependencies, generated artifacts or binary payloads.
fn source_path(path: &str) -> bool {
    if path.split('/').any(|part| {
        matches!(
            part,
            ".git"
                | "target"
                | "node_modules"
                | "vendor"
                | ".venv"
                | "__pycache__"
                | "dist"
                | "build"
        )
    }) {
        return false;
    }
    matches!(
        path.rsplit('.').next().unwrap_or(""),
        "rs" | "py"
            | "pyi"
            | "go"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "java"
            | "kt"
            | "scala"
            | "rb"
            | "swift"
            | "cs"
            | "sh"
            | "sql"
            | "proto"
            | "lean"
    )
}

fn terms(task: &str) -> Result<Vec<String>> {
    if task.trim().is_empty() || task.len() > MAX_QUERY_BYTES || task.contains('\0') {
        return refuse("agent locate task must be 1..=4096 bytes of nonempty text");
    }
    let mut found = Vec::new();
    let mut token = String::new();
    for ch in task.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            if token.len() < 64 {
                token.push(ch.to_ascii_lowercase());
            }
        } else if !token.is_empty() {
            if token.len() >= 2 && !found.contains(&token) {
                found.push(std::mem::take(&mut token));
            } else {
                token.clear();
            }
        }
    }
    const STOP: &[&str] = &[
        "a",
        "an",
        "and",
        "are",
        "by",
        "for",
        "from",
        "how",
        "in",
        "into",
        "is",
        "of",
        "on",
        "or",
        "the",
        "this",
        "to",
        "what",
        "where",
        "which",
        "with",
        "fix",
        "find",
        "implement",
        "code",
        "source",
        "file",
    ];
    found.retain(|term| !STOP.contains(&term.as_str()));
    found.truncate(MAX_QUERY_TERMS);
    if found.is_empty() {
        return refuse("agent locate task has no searchable identifier terms");
    }
    Ok(found)
}

/// Safe MATCH expression constructed from ASCII identifier tokens, never
/// interpolated untrusted task syntax or FTS query operators.
fn match_expression(words: &[String]) -> String {
    words
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn source(commit: &str, tree: &str, entry: &git::Entry) -> Value {
    obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(&entry.path)),
        ("blob", Value::string(&entry.oid)),
        ("mode", Value::string(&entry.mode)),
    ])
}

fn line_tokens(line: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut word = String::new();
    for ch in line.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            word.push(ch.to_ascii_lowercase());
        } else if !word.is_empty() {
            tokens.insert(std::mem::take(&mut word));
        }
    }
    tokens
}

/// A source-location heuristic for navigation, not a validated symbol range.
fn excerpt(body: &str, words: &[String]) -> Value {
    let lines = body.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return obj(&[
            ("start_line", Value::number(1)),
            ("end_line", Value::number(1)),
            ("text", Value::string("")),
            ("method", Value::string("TEXT_LINE_HEURISTIC")),
        ]);
    }
    let mut best = (i32::MIN, 0usize);
    for (i, line) in lines.iter().enumerate() {
        let normalized = line_tokens(line);
        let hits = words
            .iter()
            .filter(|word| normalized.contains(word.as_str()))
            .count();
        if hits == 0 {
            continue;
        }
        let trimmed = line.trim_start();
        let comment = trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*');
        let declaration = [
            "fn ",
            "pub fn ",
            "def ",
            "class ",
            "func ",
            "struct ",
            "interface ",
        ]
        .iter()
        .any(|keyword| trimmed.starts_with(keyword));
        let score = hits as i32 * 10 + i32::from(declaration) * 3 - i32::from(comment) * 12;
        if score > best.0 {
            best = (score, i);
        }
    }
    let start = best.1.saturating_sub(2);
    let end = (start + MAX_EXCERPT_LINES).min(lines.len());
    let selected = lines[start..end].join("\n");
    // Truncate on a UTF-8 boundary, never corrupt a byte string.
    let cut = selected
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= 1600)
        .last()
        .unwrap_or(0);
    let shown = if selected.len() <= 1600 {
        selected.as_str()
    } else {
        &selected[..cut]
    };
    obj(&[
        ("start_line", Value::number(start + 1)),
        ("end_line", Value::number(end)),
        ("text", Value::string(shown)),
        ("method", Value::string("TEXT_LINE_HEURISTIC")),
    ])
}

fn gap(reason: &str) -> Value {
    obj(&[("reason", Value::string(reason))])
}

/// Locate candidate source files without requiring an initial --focus path.
///
/// No working-tree reads or checkout changes occur. Cached native observations
/// are included only through the existing agent context implementation, which
/// preserves their producer evidence and unresolved coverage obligations.
pub fn locate(
    repo: &Path,
    rev: &str,
    task: &str,
    limit: usize,
    roots: Option<&str>,
    db: Option<&str>,
) -> Result<Value> {
    if !(1..=64).contains(&limit) {
        return refuse("agent locate --limit must be 1..=64");
    }
    let words = terms(task)?;
    let discovered = discovery::inspect(repo, rev)?;
    let entries = git::entries(&discovered.root, &discovered.commit)?;
    let mut reader = git::BlobReader::open(&discovered.root)?;
    let index = Connection::open_in_memory().map_err(|e| format!("FTS5 memory index: {e}"))?;
    index
        .execute_batch(
            "CREATE VIRTUAL TABLE source_search USING fts5(path, body, tokenize='unicode61');",
        )
        .map_err(|e| format!("FTS5 unavailable: {e}"))?;

    let mut files = Vec::<IndexedFile>::new();
    let mut bytes = 0usize;
    let mut skipped = 0usize;
    let mut truncated = false;
    for entry in entries {
        if entry.kind != "blob"
            || !matches!(entry.mode.as_str(), "100644" | "100755")
            || !source_path(&entry.path)
        {
            continue;
        }
        if files.len() == MAX_FILES || bytes >= MAX_TOTAL_BYTES {
            truncated = true;
            break;
        }
        let data = match reader.read_capped(&entry.oid, MAX_FILE_BYTES) {
            Ok(data) => data,
            Err(error) if error == "Git source blob exceeds byte limit" => {
                skipped += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let Ok(body) = String::from_utf8(data) else {
            skipped += 1;
            continue;
        };
        if body.contains('\0') || body.len() + bytes > MAX_TOTAL_BYTES {
            skipped += 1;
            if body.len() + bytes > MAX_TOTAL_BYTES {
                truncated = true;
            }
            continue;
        }
        bytes += body.len();
        index
            .execute(
                "INSERT INTO source_search(rowid,path,body) VALUES(?1,?2,?3)",
                params![(files.len() + 1) as i64, entry.path, body],
            )
            .map_err(|e| format!("FTS5 insert: {e}"))?;
        files.push(IndexedFile { entry, body });
    }
    reader.finish()?;

    let mut candidates = Vec::new();
    let mut selected_paths = Vec::new();
    let expression = match_expression(&words);
    let mut statement = index
        .prepare(
            "SELECT rowid FROM source_search WHERE source_search MATCH ?1 \
         ORDER BY bm25(source_search, 4.0, 1.0), rowid LIMIT ?2",
        )
        .map_err(|e| format!("FTS5 query setup: {e}"))?;
    let rows = statement
        .query_map(params![expression, limit as i64], |row| row.get::<_, i64>(0))
        .map_err(|e| format!("FTS5 query: {e}"))?;
    for row in rows {
        let id = row.map_err(|e| format!("FTS5 row: {e}"))?;
        let file = files
            .get(usize::try_from(id - 1).map_err(|_| "invalid FTS5 row ID")?)
            .ok_or("FTS5 returned source outside indexed Git inventory")?;
        selected_paths.push(file.entry.path.clone());
        candidates.push(obj(&[
            ("rank", Value::number(candidates.len() + 1)),
            (
                "source",
                source(&discovered.commit, &discovered.tree, &file.entry),
            ),
            ("excerpt", excerpt(&file.body, &words)),
            ("ranking", Value::string("FTS5_BM25_TEXT_CANDIDATE")),
            ("semantic_correctness", Value::string("UNVERIFIED")),
        ]));
    }

    let mut gaps = vec![
        gap("TASK_SEMANTICS_NOT_VERIFIED"),
        gap("TEXT_MATCHES_ARE_NOT_SOURCE_DEPENDENCIES"),
        gap("UNCOMMITTED_CHANGES_EXCLUDED"),
        gap("TEST_SELECTION_NOT_PROVEN"),
    ];
    if skipped > 0 {
        gaps.push(gap("UNINDEXED_OVERSIZE_OR_NON_UTF8_SOURCE"));
    }
    if truncated {
        gaps.push(gap("INDEX_BUDGET_EXHAUSTED"));
    }
    if candidates.is_empty() {
        gaps.push(gap("NO_LEXICAL_MATCH_DOES_NOT_PROVE_ABSENCE"));
    }

    // Existing adapter supplies *observed* relationships and candidate tests
    // for the strongest lexical hits, preserving original producer receipts.
    // The existing agent context wire format uses comma-separated paths.
    // Git names may contain commas, so omit those only from context rather
    // than misinterpreting one source as multiple unrelated paths.
    let context_paths = selected_paths
        .iter()
        .filter(|path| !path.contains(','))
        .take(MAX_CONTEXT_FOCUS)
        .cloned()
        .collect::<Vec<_>>();
    if selected_paths.iter().any(|path| path.contains(',')) {
        gaps.push(gap("CONTEXT_UNREPRESENTABLE_COMMA_PATH"));
    }
    if selected_paths.len() > MAX_CONTEXT_FOCUS {
        gaps.push(gap("CONTEXT_FOCUS_TRUNCATED_TO_16"));
    }
    let context = if context_paths.is_empty() {
        Value::Null
    } else {
        agent::context(
            &discovered.root,
            &discovered.commit,
            &context_paths.join(","),
            Some(task),
            limit,
            roots,
            db,
        )?
    };

    let mut output = obj(&[
        ("schema", Value::string("excession.agent-locate.v1")),
        ("source_commit", Value::string(&discovered.commit)),
        ("source_tree", Value::string(&discovered.tree)),
        ("task", Value::string(task)),
        ("retrieval", Value::string("GIT_BLOB_FTS5_BM25")),
        ("candidates", Value::array(candidates)),
        ("candidate_count", Value::number(selected_paths.len())),
        ("indexed_source_count", Value::number(files.len())),
        ("skipped_source_count", Value::number(skipped)),
        ("indexed_bytes", Value::number(bytes)),
        ("context", context),
        (
            "coverage",
            obj(&[
                ("status", Value::string("PARTIAL")),
                ("truncated", Value::Bool(truncated)),
                ("gaps", Value::array(gaps)),
            ]),
        ),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]);
    let report_id = output.sha256();
    output
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_operators_cannot_escape_fts5_terms() {
        let words = terms("\"lease\" OR * NOT ; -- recovery").unwrap();
        assert_eq!(words, ["lease", "not", "recovery"]);
        assert_eq!(
            match_expression(&words),
            "\"lease\" OR \"not\" OR \"recovery\""
        );
        assert!(terms("***").is_err());
        assert!(terms(&"x".repeat(4097)).is_err());
    }

    #[test]
    fn exclude_external_artifacts_and_non_regular_source() {
        assert!(!source_path("node_modules/pkg/test.rs"));
        assert!(!source_path("target/release/main.rs"));
        assert!(!source_path("vendor/pkg/file.go"));
        assert!(!source_path("docs/readme.md"));
        assert!(source_path("src/agent.rs"));
        assert!(source_path("pkg/controller/main.go"));
    }

    #[test]
    fn line_windows_are_bounded_and_one_based() {
        let source = "alpha\nbeta\nfn recover_lease() {}\ngamma\n";
        let value = excerpt(source, &["recover_lease".into()]);
        assert_eq!(value.get("start_line").unwrap().integer().unwrap(), 1);
        assert_eq!(value.get("end_line").unwrap().integer().unwrap(), 4);
        assert!(
            value
                .get("text")
                .unwrap()
                .str()
                .unwrap()
                .contains("recover_lease")
        );
    }

    #[test]
    fn locates_pinned_code_without_reading_dirty_checkout() {
        use std::fs;
        use std::process::Command;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-locate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let git = |args: &[&str]| {
            let result = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "git: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git(&["init", "-q"]);
        fs::write(
            root.join("src/lease.rs"),
            "pub fn renew_lease() { recover_worker(); }\n",
        )
        .unwrap();
        fs::write(root.join("src/other.rs"), "pub fn render_canvas() {}\n").unwrap();
        fs::write(root.join("src/oversize.rs"), "x".repeat(MAX_FILE_BYTES + 1)).unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-qm",
            "initial",
        ]);

        let first = locate(&root, "HEAD", "renew lease", 4, None, None).unwrap();
        let ranked = first.get("candidates").unwrap().array_ref().unwrap();
        assert!(!ranked.is_empty());
        assert_eq!(
            ranked[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/lease.rs"
        );
        assert_eq!(
            first
                .get("skipped_source_count")
                .unwrap()
                .integer()
                .unwrap(),
            1
        );
        assert!(first.get("context").unwrap().get("coverage").is_ok());
        assert_eq!(
            first
                .get("safe_to_skip_unlisted_tests")
                .unwrap()
                .bool()
                .unwrap(),
            false
        );
        let first_id = first.get("report_id").unwrap().str().unwrap().to_owned();
        fs::write(root.join("src/lease.rs"), "pub fn something_else() {}\n").unwrap();
        fs::write(root.join("src/other.rs"), "pub fn renew_lease() {}\n").unwrap();
        let second = locate(&root, "HEAD", "renew lease", 4, None, None).unwrap();
        assert_eq!(second.get("report_id").unwrap().str().unwrap(), first_id);
        assert_eq!(
            second.get("candidates").unwrap().array_ref().unwrap()[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/lease.rs"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fts5_ranks_matching_code_above_unrelated_code() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE VIRTUAL TABLE source_search USING fts5(path, body);")
            .unwrap();
        db.execute(
            "INSERT INTO source_search(path,body) VALUES(?1,?2)",
            params!["src/other.rs", "fn unrelated() { println!(\"ok\"); }"],
        )
        .unwrap();
        db.execute(
            "INSERT INTO source_search(path,body) VALUES(?1,?2)",
            params!["src/lease.rs", "fn recover_lease() { restore_worker(); }"],
        )
        .unwrap();
        let best: String = db
            .query_row(
                "SELECT path FROM source_search WHERE source_search MATCH ?1 \
             ORDER BY bm25(source_search, 4.0, 1.0), rowid LIMIT 1",
                params![match_expression(&terms("recover lease").unwrap())],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(best, "src/lease.rs");
    }
}
