//! Evidence-bound, model-free task-to-source discovery for coding agents.
//!
//! The index is ephemeral, derived exclusively from authenticated Git blobs.
//! FTS5 ranks *textual candidates*, not validated program behavior. Output
//! preserves the boundary between lexical matches and native observations.

use crate::json::{Value, obj};
use crate::{Result, agent, agent_locate_cache, discovery, git, refuse};
pub use agent_locate_cache::IndexLimits;
#[cfg(test)]
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[cfg(test)]
use agent_locate_cache::MAX_FILE_BYTES;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_QUERY_TERMS: usize = 16;
const MAX_EXCERPT_LINES: usize = 7;
const MAX_CONTEXT_FOCUS: usize = 16;

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
            | "y"
            | "l"
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

/// Return only direct neighbors of lexical hits in the existing source-bound
/// dependency graph. A relation is *evidence*, not a new BM25 score or proof of
/// runtime behavior. Preserve the producer's original assessment and receipt.
fn related_sources(
    context: &Value,
    lexical_paths: &BTreeSet<String>,
    inventory: &BTreeMap<String, git::Entry>,
    commit: &str,
    tree: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    if context == &Value::Null {
        return Ok(Vec::new());
    }
    if context.get("source_commit")?.str()? != commit || context.get("source_tree")?.str()? != tree
    {
        return refuse("agent locate relation context belongs to another Git tree");
    }
    let mut neighbors = Vec::new();
    let mut seen = BTreeSet::new();
    for relation in context.get("relationships")?.array_ref()? {
        let kind = relation.get("kind")?.str()?;
        // Configured graph membership uses a different source/entity domain,
        // and is not silently interpreted as a source-to-source dependency.
        if !matches!(kind, "SCIP_REFERENCE" | "STATIC_IMPORT") {
            continue;
        }
        let from = relation.get("source")?;
        let to = relation.get("target")?;
        let from_path = from.get("path")?.str()?;
        let to_path = to.get("path")?.str()?;
        let from_selected = lexical_paths.contains(from_path);
        let to_selected = lexical_paths.contains(to_path);
        if from_selected == to_selected {
            continue;
        }
        // Both endpoints must still match this exact tracked Git tree.
        for endpoint in [from, to] {
            let path = endpoint.get("path")?.str()?;
            let file = inventory
                .get(path)
                .ok_or("relationship endpoint absent from current Git source tree")?;
            if endpoint.get("blob")?.str()? != file.oid
                || endpoint.get("mode")?.str()? != file.mode
                || endpoint.get("source_commit")?.str()? != commit
                || endpoint.get("source_tree")?.str()? != tree
            {
                return refuse("relationship endpoint differs from current Git source tree");
            }
        }
        let evidence = relation.get("evidence")?.array_ref()?;
        if evidence.is_empty() {
            return refuse("related source has no producer evidence");
        }
        let expected = if kind == "SCIP_REFERENCE" {
            "OBSERVED"
        } else {
            "CANDIDATE"
        };
        if evidence
            .iter()
            .any(|row| row.get("assessment").and_then(Value::str).ok() != Some(expected))
        {
            return refuse("related source carries incompatible evidence assessment");
        }
        let neighbor = if from_selected { to } else { from };
        let neighbor_path = neighbor.get("path")?.str()?;
        let key = (
            neighbor_path.to_owned(),
            kind.to_owned(),
            from_path.to_owned(),
            to_path.to_owned(),
        );
        if !seen.insert(key) {
            continue;
        }
        if neighbors.len() >= limit {
            break;
        }
        neighbors.push(obj(&[
            ("source", neighbor.clone()),
            ("relation", relation.clone()),
            ("discovery", Value::string("SOURCE_BOUND_GRAPH_NEIGHBOR")),
            ("assessment", Value::string(expected)),
            (
                "direction",
                Value::string(if from_selected {
                    "LEXICAL_SOURCE_TO_NEIGHBOR"
                } else {
                    "NEIGHBOR_TO_LEXICAL_SOURCE"
                }),
            ),
            ("semantic_correctness", Value::string("UNVERIFIED")),
        ]));
    }
    Ok(neighbors)
}

/// One bounded, Git-backed source inspection list. This only reorganizes
/// references already present in locate/context; it never infers dependencies.
struct ReadingSteps {
    seen: BTreeSet<String>,
    items: Vec<Value>,
    budget: usize,
}

impl ReadingSteps {
    fn push(
        &mut self,
        item: &Value,
        action: &str,
        basis: &str,
        why: &str,
        collection: &str,
        index: usize,
    ) -> Result<()> {
        if self.items.len() >= self.budget {
            return Ok(());
        }
        let source = item.get("source")?;
        let path = source.get("path")?.str()?;
        if !self.seen.insert(path.to_owned()) {
            return Ok(());
        }
        self.items.push(obj(&[
            ("sequence", Value::number(self.items.len() + 1)),
            ("action", Value::string(action)),
            ("source", source.clone()),
            ("basis", Value::string(basis)),
            ("why_read", Value::string(why)),
            ("reference_collection", Value::string(collection)),
            ("reference_index", Value::number(index)),
            ("behavior_verified", Value::Bool(false)),
        ]));
        Ok(())
    }
}

/// Turn existing task search, source relationships, and test candidates into
/// a short inspection sequence. It does not infer program behavior, synthesize
/// missing relations, or ask an indexer to produce additional facts.
fn reading_plan(
    candidates: &[Value],
    related: &[Value],
    context: &Value,
    limit: usize,
) -> Result<Value> {
    let budget = limit.min(12);
    let mut steps = ReadingSteps {
        seen: BTreeSet::new(),
        items: Vec::new(),
        budget,
    };

    // Preserve room for a test-reading step when one is present, even under
    // small budgets. Code understanding must not collapse to graph-neighbor
    // enumeration simply because a producer supplied many observations.
    let has_tests =
        context != &Value::Null && !context.get("test_candidates")?.array_ref()?.is_empty();
    let reserved_test = usize::from(has_tests && budget > 1);
    // Read likely implementations before chasing edges. Ranking is lexical,
    // not a measured probability that the task lives in that file.
    for (i, candidate) in candidates
        .iter()
        .take(4.min(budget.saturating_sub(reserved_test)))
        .enumerate()
    {
        steps.push(
            candidate,
            "INSPECT_TASK_MATCH",
            "GIT_BLOB_LEXICAL_MATCH",
            "Read the matching source to determine inputs, outputs, state changes, and invariants.",
            "candidates",
            i,
        )?;
    }
    // Show both sides of a potential behavior path. Observed references
    // remain producer observations; static imports remain candidates.
    let relation_budget = 4.min(budget.saturating_sub(steps.items.len() + reserved_test));
    for (i, neighbor) in related.iter().take(relation_budget).enumerate() {
        let assessment = neighbor.get("assessment")?.str()?;
        let direction = neighbor.get("direction")?.str()?;
        let (action, why) = if direction == "LEXICAL_SOURCE_TO_NEIGHBOR" {
            (
                "INSPECT_POSSIBLE_DEPENDENCY",
                "Check how the focused code uses this source; confirm whether data or control actually flows to it.",
            )
        } else if direction == "NEIGHBOR_TO_LEXICAL_SOURCE" {
            (
                "INSPECT_POSSIBLE_CALLER",
                "Check whether this source invokes or configures the focused behavior.",
            )
        } else {
            return refuse("reading plan received unsupported relationship direction");
        };
        let basis = match assessment {
            "OBSERVED" => "SOURCE_BOUND_PRODUCER_OBSERVATION",
            "CANDIDATE" => "STATIC_RELATIONSHIP_CANDIDATE",
            _ => return refuse("reading plan received unsupported relationship assessment"),
        };
        steps.push(neighbor, action, basis, why, "relationship_candidates", i)?;
    }
    if context != &Value::Null {
        for (i, candidate) in context
            .get("test_candidates")?
            .array_ref()?
            .iter()
            .take(4)
            .enumerate()
        {
            steps.push(
                candidate,
                "INSPECT_RELATED_TEST",
                "TEST_PATH_HEURISTIC",
                "Read the assertions and fixtures to identify which behavior they actually cover.",
                "context.test_candidates",
                i,
            )?;
        }
    }
    Ok(obj(&[
        ("schema", Value::string("excession.agent-reading-plan.v1")),
        (
            "goal",
            Value::string("SOURCE_GUIDED_CODEBASE_UNDERSTANDING"),
        ),
        ("steps", Value::array(steps.items)),
        (
            "interpretation",
            Value::string("ORDERED_INSPECTION_SUGGESTIONS_NOT_VERIFIED_BEHAVIOR"),
        ),
        ("complete_behavior_trace", Value::Bool(false)),
    ]))
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
    locate_with_budget(
        repo,
        rev,
        task,
        limit,
        roots,
        db,
        agent_locate_cache::DEFAULT_REFRESH_SEGMENTS,
    )
}

/// Incrementally index at most index_segments stale source partitions.
/// Unindexed partitions are explicitly excluded from retrieval results.
pub fn locate_with_budget(
    repo: &Path,
    rev: &str,
    task: &str,
    limit: usize,
    roots: Option<&str>,
    db: Option<&str>,
    index_segments: usize,
) -> Result<Value> {
    locate_with_limits(
        repo,
        rev,
        task,
        limit,
        roots,
        db,
        index_segments,
        IndexLimits::default(),
    )
}

/// Run source discovery using explicitly selected index processing budgets.
/// All three budgets are included in the cache fingerprint and in the report.
pub fn locate_with_limits(
    repo: &Path,
    rev: &str,
    task: &str,
    limit: usize,
    roots: Option<&str>,
    db: Option<&str>,
    index_segments: usize,
    limits: IndexLimits,
) -> Result<Value> {
    let limits = limits.validate()?;
    if !(1..=64).contains(&limit) {
        return refuse("agent locate --limit must be 1..=64");
    }
    let words = terms(task)?;
    let discovered = discovery::inspect(repo, rev)?;
    let entries = git::entries(&discovered.root, &discovered.commit)?;
    let inventory = entries
        .iter()
        .filter(|entry| entry.kind == "blob" && matches!(entry.mode.as_str(), "100644" | "100755"))
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    let index = agent_locate_cache::search_with_limits(
        &discovered,
        entries,
        limit,
        &match_expression(&words),
        index_segments,
        source_path,
        limits,
    )?;
    let mut candidates = Vec::new();
    let mut selected_paths = Vec::new();
    for hit in &index.hits {
        selected_paths.push(hit.entry.path.clone());
        candidates.push(obj(&[
            ("rank", Value::number(candidates.len() + 1)),
            (
                "source",
                source(&discovered.commit, &discovered.tree, &hit.entry),
            ),
            ("excerpt", excerpt(&hit.body, &words)),
            ("ranking", Value::string("FTS5_BM25_TEXT_CANDIDATE")),
            ("semantic_correctness", Value::string("UNVERIFIED")),
        ]));
    }
    let skipped = index.skipped_files;
    let truncated = index.truncated;
    let bytes = index.indexed_bytes;
    let indexed_files = index.indexed_files;

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
    if index.pending_segments > 0 {
        gaps.push(gap("SEGMENT_INDEX_INCOMPLETE"));
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

    let lexical = selected_paths.iter().cloned().collect::<BTreeSet<_>>();
    let related = related_sources(
        &context,
        &lexical,
        &inventory,
        &discovered.commit,
        &discovered.tree,
        limit,
    )?;
    if context != Value::Null
        && context.get("relationship_count")?.integer()?
            > context.get("relationships")?.array_ref()?.len() as i64
    {
        gaps.push(gap("RELATIONSHIP_CONTEXT_TRUNCATED"));
    }

    let plan = reading_plan(&candidates, &related, &context, limit)?;
    let mut output = obj(&[
        ("schema", Value::string("excession.agent-locate.v1")),
        ("source_commit", Value::string(&discovered.commit)),
        ("source_tree", Value::string(&discovered.tree)),
        ("task", Value::string(task)),
        ("retrieval", Value::string("GIT_BLOB_FTS5_BM25")),
        ("candidates", Value::array(candidates)),
        ("candidate_count", Value::number(selected_paths.len())),
        ("relationship_candidates", Value::array(related)),
        ("reading_plan", plan),
        ("indexed_source_count", Value::number(indexed_files)),
        ("skipped_source_count", Value::number(skipped)),
        ("indexed_bytes", Value::number(bytes)),
        (
            "index",
            obj(&[
                ("segmentation", Value::string("PATH_SHA256_PREFIX_BYTE")),
                (
                    "total_source_files",
                    Value::number(index.total_source_files),
                ),
                (
                    "populated_segments",
                    Value::number(index.populated_segments),
                ),
                ("ready_segments", Value::number(index.ready_segments)),
                ("pending_segments", Value::number(index.pending_segments)),
                (
                    "complete",
                    Value::Bool(index.pending_segments == 0 && skipped == 0 && !truncated),
                ),
                (
                    "limits",
                    obj(&[
                        (
                            "max_files_per_segment",
                            Value::number(limits.max_files_per_segment),
                        ),
                        ("max_file_bytes", Value::number(limits.max_file_bytes)),
                        (
                            "max_bytes_per_segment",
                            Value::number(limits.max_bytes_per_segment),
                        ),
                        ("refresh_segments", Value::number(index_segments)),
                    ]),
                ),
            ]),
        ),
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
    // The report identity commits to reproducible source evidence, not the
    // incidental state of a local incremental cache.
    let report_id = output.sha256();
    output
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    output.entries_mut()?.insert(
        "cache".into(),
        obj(&[
            ("kind", Value::string("PERSISTENT_SQLITE_FTS5")),
            ("path", Value::string(index.cache_path.to_string_lossy())),
            ("reused_blobs", Value::number(index.reused_blobs)),
            ("read_blobs", Value::number(index.read_blobs)),
            (
                "refreshed_segments",
                Value::number(index.refreshed_segments),
            ),
        ]),
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_plan_orients_to_source_then_related_code_then_tests() {
        let git_source = |path: &str| {
            obj(&[
                ("source_commit", Value::string("a".repeat(40))),
                ("source_tree", Value::string("b".repeat(40))),
                ("path", Value::string(path)),
                ("blob", Value::string("c".repeat(40))),
                ("mode", Value::string("100644")),
            ])
        };
        let lexical = obj(&[("source", git_source("src/worker.py"))]);
        let incoming = obj(&[
            ("source", git_source("src/handler.py")),
            ("assessment", Value::string("OBSERVED")),
            ("direction", Value::string("NEIGHBOR_TO_LEXICAL_SOURCE")),
        ]);
        let outgoing = obj(&[
            ("source", git_source("src/store.py")),
            ("assessment", Value::string("CANDIDATE")),
            ("direction", Value::string("LEXICAL_SOURCE_TO_NEIGHBOR")),
        ]);
        let duplicate = obj(&[
            ("source", git_source("src/worker.py")),
            ("assessment", Value::string("OBSERVED")),
            ("direction", Value::string("LEXICAL_SOURCE_TO_NEIGHBOR")),
        ]);
        let context = obj(&[(
            "test_candidates",
            Value::array([obj(&[("source", git_source("tests/test_worker.py"))])]),
        )]);
        let plan = reading_plan(&[lexical], &[incoming, outgoing, duplicate], &context, 8).unwrap();
        let steps = plan.get("steps").unwrap().array_ref().unwrap();
        assert_eq!(steps.len(), 4);
        let paths = steps
            .iter()
            .map(|item| {
                item.get("source")
                    .unwrap()
                    .get("path")
                    .unwrap()
                    .str()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "src/worker.py",
                "src/handler.py",
                "src/store.py",
                "tests/test_worker.py"
            ]
        );
        assert_eq!(
            steps[1].get("action").unwrap().str().unwrap(),
            "INSPECT_POSSIBLE_CALLER"
        );
        assert_eq!(
            steps[2].get("basis").unwrap().str().unwrap(),
            "STATIC_RELATIONSHIP_CANDIDATE"
        );
        assert!(
            steps
                .iter()
                .all(|item| { !item.get("behavior_verified").unwrap().bool().unwrap() })
        );
        assert!(!plan.get("complete_behavior_trace").unwrap().bool().unwrap());
    }

    #[test]
    fn reading_plan_reserves_tests_in_small_bounded_investigations() {
        let source = |path: &str| obj(&[("path", Value::string(path))]);
        let lexical = (0..4)
            .map(|i| obj(&[("source", source(&format!("src/feature{i}.rs")))]))
            .collect::<Vec<_>>();
        let related = (0..8)
            .map(|i| {
                obj(&[
                    ("source", source(&format!("src/dependency{i}.rs"))),
                    ("assessment", Value::string("OBSERVED")),
                    ("direction", Value::string("LEXICAL_SOURCE_TO_NEIGHBOR")),
                ])
            })
            .collect::<Vec<_>>();
        let context = obj(&[(
            "test_candidates",
            Value::array([obj(&[("source", source("tests/test_feature.rs"))])]),
        )]);
        let plan = reading_plan(&lexical, &related, &context, 8).unwrap();
        let steps = plan.get("steps").unwrap().array_ref().unwrap();
        assert_eq!(steps.len(), 8);
        assert_eq!(
            steps[7].get("action").unwrap().str().unwrap(),
            "INSPECT_RELATED_TEST"
        );
        assert_eq!(
            steps[7].get("basis").unwrap().str().unwrap(),
            "TEST_PATH_HEURISTIC"
        );
    }

    #[test]
    fn reading_plan_keeps_small_budget_and_avoids_fabrication() {
        let source = obj(&[("path", Value::string("src/found.py"))]);
        let candidates = [obj(&[("source", source)])];
        let plan = reading_plan(&candidates, &[], &Value::Null, 1).unwrap();
        assert_eq!(plan.get("steps").unwrap().array_ref().unwrap().len(), 1);
        let empty = reading_plan(&[], &[], &Value::Null, 16).unwrap();
        assert!(empty.get("steps").unwrap().array_ref().unwrap().is_empty());
        let fake = obj(&[
            ("source", obj(&[("path", Value::string("src/fake.py"))])),
            ("assessment", Value::string("VERIFIED")),
            ("direction", Value::string("LEXICAL_SOURCE_TO_NEIGHBOR")),
        ]);
        assert!(reading_plan(&[], &[fake], &Value::Null, 16).is_err());
    }

    #[test]
    fn native_neighbors_preserve_receipts_and_reject_stale_git_identities() {
        let commit = "a".repeat(40);
        let tree = "b".repeat(40);
        let blob_a = "c".repeat(40);
        let blob_b = "d".repeat(40);
        let file = |path: &str, blob: &str| git::Entry {
            path: path.into(),
            oid: blob.into(),
            mode: "100644".into(),
            kind: "blob".into(),
        };
        let a = file("src/a.rs", &blob_a);
        let b = file("src/b.rs", &blob_b);
        let inventory = BTreeMap::from([(a.path.clone(), a.clone()), (b.path.clone(), b.clone())]);
        let lexical = BTreeSet::from(["src/a.rs".to_owned()]);
        let relation = obj(&[
            ("source", source(&commit, &tree, &a)),
            ("target", source(&commit, &tree, &b)),
            ("kind", Value::string("SCIP_REFERENCE")),
            (
                "evidence",
                Value::array([obj(&[
                    ("receipt_id", Value::string("e".repeat(64))),
                    ("producer", Value::string("scip:unit-test")),
                    ("assessment", Value::string("OBSERVED")),
                    ("observation_kind", Value::string("SCIP_REFERENCE")),
                ])]),
            ),
        ]);
        let context = obj(&[
            ("source_commit", Value::string(&commit)),
            ("source_tree", Value::string(&tree)),
            ("relationships", Value::array([relation])),
        ]);
        let found = related_sources(&context, &lexical, &inventory, &commit, &tree, 4).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/b.rs"
        );
        assert_eq!(
            found[0].get("assessment").unwrap().str().unwrap(),
            "OBSERVED"
        );
        assert_eq!(
            found[0]
                .get("relation")
                .unwrap()
                .get("evidence")
                .unwrap()
                .array_ref()
                .unwrap()[0]
                .get("receipt_id")
                .unwrap()
                .str()
                .unwrap(),
            "e".repeat(64)
        );
        assert_eq!(
            found[0].get("semantic_correctness").unwrap().str().unwrap(),
            "UNVERIFIED"
        );

        let mutated = obj(&[
            ("source_commit", Value::string(&commit)),
            ("source_tree", Value::string(&tree)),
            (
                "relationships",
                Value::array([obj(&[
                    ("source", source(&commit, &tree, &a)),
                    (
                        "target",
                        source(&commit, &tree, &file("src/b.rs", &"f".repeat(40))),
                    ),
                    ("kind", Value::string("SCIP_REFERENCE")),
                    (
                        "evidence",
                        Value::array([obj(&[("assessment", Value::string("OBSERVED"))])]),
                    ),
                ])]),
            ),
        ]);
        assert!(related_sources(&mutated, &lexical, &inventory, &commit, &tree, 4).is_err());
        assert!(
            related_sources(&context, &lexical, &inventory, &"f".repeat(40), &tree, 4).is_err()
        );

        // Static imports remain explicitly candidates, never observations.
        let static_relation = obj(&[
            ("source_commit", Value::string(&commit)),
            ("source_tree", Value::string(&tree)),
            (
                "relationships",
                Value::array([obj(&[
                    ("source", source(&commit, &tree, &a)),
                    ("target", source(&commit, &tree, &b)),
                    ("kind", Value::string("STATIC_IMPORT")),
                    (
                        "evidence",
                        Value::array([obj(&[
                            ("assessment", Value::string("CANDIDATE")),
                            ("observation_kind", Value::string("STATIC_IMPORT_CANDIDATE")),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let candidate =
            related_sources(&static_relation, &lexical, &inventory, &commit, &tree, 4).unwrap();
        assert_eq!(
            candidate[0].get("assessment").unwrap().str().unwrap(),
            "CANDIDATE"
        );
    }

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
        assert!(source_path("src/parse.y"));
        assert!(source_path("src/tokenizer.l"));
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

        // A caller cannot redirect the cache into a repository directory.
        // The rejection must happen before any directories are created.
        let mut forbidden = discovery::inspect(&root, "HEAD").unwrap();
        forbidden.index_path = root.join(".forbidden/index.sqlite");
        assert!(
            agent_locate_cache::search(
                &forbidden,
                git::entries(&root, &forbidden.commit).unwrap(),
                4,
                "\"lease\"",
                agent_locate_cache::DEFAULT_REFRESH_SEGMENTS,
                source_path,
            )
            .is_err()
        );
        assert!(!root.join(".forbidden").exists());

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
        let plan = first.get("reading_plan").unwrap();
        let steps = plan.get("steps").unwrap().array_ref().unwrap();
        assert!(!steps.is_empty());
        assert_eq!(
            steps[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/lease.rs"
        );
        assert_eq!(
            steps[0].get("basis").unwrap().str().unwrap(),
            "GIT_BLOB_LEXICAL_MATCH"
        );
        assert_eq!(
            steps[0]
                .get("source")
                .unwrap()
                .get("source_commit")
                .unwrap()
                .str()
                .unwrap(),
            first.get("source_commit").unwrap().str().unwrap()
        );
        assert!(
            !first
                .get("safe_to_skip_unlisted_tests")
                .unwrap()
                .bool()
                .unwrap()
        );
        let first_id = first.get("report_id").unwrap().str().unwrap().to_owned();
        assert_eq!(
            first
                .get("cache")
                .unwrap()
                .get("kind")
                .unwrap()
                .str()
                .unwrap(),
            "PERSISTENT_SQLITE_FTS5"
        );
        assert_eq!(
            first
                .get("cache")
                .unwrap()
                .get("read_blobs")
                .unwrap()
                .integer()
                .unwrap(),
            2
        );
        fs::write(root.join("src/lease.rs"), "pub fn something_else() {}\n").unwrap();
        fs::write(root.join("src/other.rs"), "pub fn renew_lease() {}\n").unwrap();
        let second = locate(&root, "HEAD", "renew lease", 4, None, None).unwrap();
        assert_eq!(second.get("report_id").unwrap().str().unwrap(), first_id);
        assert_eq!(
            second.get("reading_plan").unwrap(),
            first.get("reading_plan").unwrap(),
            "uncommitted edits cannot change the pinned source reading plan"
        );
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
        assert_eq!(
            second
                .get("cache")
                .unwrap()
                .get("read_blobs")
                .unwrap()
                .integer()
                .unwrap(),
            0
        );
        assert_eq!(
            second
                .get("cache")
                .unwrap()
                .get("reused_blobs")
                .unwrap()
                .integer()
                .unwrap(),
            2
        );
        git(&["add", "src/lease.rs", "src/other.rs"]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-qm",
            "modified",
        ]);
        let third = locate(&root, "HEAD", "renew lease", 4, None, None).unwrap();
        assert_eq!(
            third.get("candidates").unwrap().array_ref().unwrap()[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/other.rs"
        );
        assert_eq!(
            third
                .get("cache")
                .unwrap()
                .get("read_blobs")
                .unwrap()
                .integer()
                .unwrap(),
            2
        );
        git(&["rm", "-q", "src/other.rs"]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
            "commit",
            "-qm",
            "deleted",
        ]);
        let fourth = locate(&root, "HEAD", "renew", 4, None, None).unwrap();
        assert!(
            fourth
                .get("candidates")
                .unwrap()
                .array_ref()
                .unwrap()
                .is_empty()
        );
        let cache_file = fourth
            .get("cache")
            .unwrap()
            .get("path")
            .unwrap()
            .str()
            .unwrap();
        let cache_dir = std::path::Path::new(cache_file).parent().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(cache_dir).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        // Clear the fixture cache after exercising pinned revision transitions.
        std::fs::remove_dir_all(cache_dir).unwrap();

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
