//! Evidence-bound, model-free task-to-source discovery for coding agents.
//!
//! The index is ephemeral, derived exclusively from authenticated Git blobs.
//! FTS5 ranks *textual candidates*, not validated program behavior. Output
//! preserves the boundary between lexical matches and native observations.

use crate::json::{Value, obj};
use crate::{Result, agent, agent_locate_cache, discovery, git, refuse, source_capabilities};
pub use agent_locate_cache::IndexLimits;

#[derive(Clone, Copy, Debug)]
pub struct LocateBudget {
    pub index_segments: usize,
    pub index: IndexLimits,
}

/// Operational discovery choices share one argument so callers cannot
/// accidentally confuse index processing budgets with enrichment policy.
#[derive(Clone, Copy, Debug)]
pub struct LocateExecution {
    pub budget: LocateBudget,
    pub force_enrichment: bool,
}
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
            | "cjs"
            | "mjs"
            | "jsx"
            | "ts"
            | "cts"
            | "mts"
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

const MAX_COMMITTED_GREP_PATHS_PER_TERM: usize = 20_000;

/// Task navigation is broader than the language-native producer inventory.
/// In particular, SQLite's Tcl .test and .tcl suites are genuine source,
/// even though they have no SCIP or static-import producer.
fn investigable_path(path: &str) -> bool {
    source_path(path)
        || (source_capabilities::test_candidate(path)
            && [".test", ".tcl", ".bats"]
                .iter()
                .any(|extension| path.ends_with(extension)))
}

fn filename_stem(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path)
}

fn filename_hits(path: &str, terms: &[String]) -> usize {
    let filename = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    terms
        .iter()
        .filter(|term| filename.contains(term.as_str()))
        .count()
}

struct CommittedMatches {
    selected: Vec<git::Entry>,
    source_file_count: usize,
    incomplete: bool,
}

/// Ask Git which pinned blobs contain the literal question terms, then
/// prioritize implementation and likely tests. The match is lexical, never
/// a new language semantic edge. All returned paths are authenticated
/// against the same Git tree inventory used for the FTS provider.
fn committed_matches(
    repo: &Path,
    commit: &str,
    words: &[String],
    inventory: &BTreeMap<String, git::Entry>,
    limit: usize,
) -> Result<CommittedMatches> {
    let mut matches = BTreeMap::<String, BTreeSet<String>>::new();
    let mut incomplete = false;
    for word in words {
        let (paths, truncated) =
            git::grep_committed_paths(repo, commit, word, MAX_COMMITTED_GREP_PATHS_PER_TERM)?;
        incomplete |= truncated;
        for path in paths {
            if investigable_path(&path)
                && inventory.get(&path).is_some_and(|entry| {
                    entry.kind == "blob" && matches!(entry.mode.as_str(), "100644" | "100755")
                })
            {
                matches.entry(path).or_default().insert(word.clone());
            }
        }
    }
    let source_file_count = matches.len();
    let mut implementations = Vec::new();
    let mut tests = Vec::new();
    for (path, found) in matches {
        let entry = inventory
            .get(&path)
            .ok_or("committed grep match absent from Git tree")?
            .clone();
        let item = (entry, found.len(), filename_hits(&path, words));
        if source_capabilities::test_candidate(&path) {
            tests.push(item);
        } else {
            implementations.push(item);
        }
    }
    implementations.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    // A test filename resembling one of the top task-matched
    // implementation stems is a useful navigation hypothesis.
    // It is not evidence that the test covers that implementation.
    let stems = implementations
        .iter()
        .take(12)
        .map(|item| filename_stem(&item.0.path).to_ascii_lowercase())
        .filter(|stem| stem.len() >= 3)
        .collect::<BTreeSet<_>>();
    tests.sort_by(|a, b| {
        let hint = |entry: &git::Entry| {
            let stem = filename_stem(&entry.path).to_ascii_lowercase();
            usize::from(stems.iter().any(|base| stem.starts_with(base.as_str())))
        };
        hint(&b.0)
            .cmp(&hint(&a.0))
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    let reserved_tests = if tests.is_empty() {
        0
    } else {
        (limit / 3).clamp(1, 3)
    };
    let mut chosen = Vec::new();
    let implementation_slots = limit.saturating_sub(reserved_tests);
    let mut remaining_implementations = implementations.into_iter();
    let mut remaining_tests = tests.into_iter();
    for (entry, _, _) in remaining_implementations
        .by_ref()
        .take(implementation_slots)
    {
        chosen.push(entry);
    }
    for (entry, _, _) in remaining_tests.by_ref().take(reserved_tests) {
        chosen.push(entry);
    }
    for (entry, _, _) in remaining_implementations.chain(remaining_tests) {
        if chosen.len() == limit {
            break;
        }
        chosen.push(entry);
    }
    Ok(CommittedMatches {
        selected: chosen,
        source_file_count,
        incomplete,
    })
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
        "when",
        "why",
        "what",
        "with",
        "does",
        "after",
        "before",
        "versus",
        "under",
        "through",
        "that",
        "these",
        "then",
        "test",
        "tests",
        "tested",
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
fn excerpt(body: &str, words: &[String], first_line: usize) -> Value {
    let lines = body.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return obj(&[
            ("start_line", Value::number(first_line)),
            ("end_line", Value::number(first_line)),
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
        ("start_line", Value::number(start + first_line)),
        ("end_line", Value::number(first_line + end - 1)),
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

    let task_sources = candidates
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.get("source")
                .and_then(|source| source.get("path"))
                .and_then(Value::str)
                .map(|path| !source_capabilities::test_candidate(path))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let task_tests = candidates
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.get("source")
                .and_then(|source| source.get("path"))
                .and_then(Value::str)
                .map(source_capabilities::test_candidate)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let has_tests = !task_tests.is_empty()
        || (context != &Value::Null && !context.get("test_candidates")?.array_ref()?.is_empty());
    let reserved_test = usize::from(has_tests && budget > 1);

    // Prioritize the implementation. A literal match in a committed test is
    // better evidence for inspection than an unrelated graph neighbor.
    for (i, candidate) in task_sources
        .iter()
        .copied()
        .take(4.min(budget.saturating_sub(reserved_test)))
    {
        steps.push(
            candidate,
            "INSPECT_TASK_MATCH",
            "GIT_BLOB_LEXICAL_MATCH",
            "Read inputs, outputs, state changes and the conditions governing this implementation.",
            "candidates",
            i,
        )?;
    }
    for (i, candidate) in task_tests.iter().copied().take(3) {
        steps.push(
            candidate,
            "INSPECT_RELATED_TEST",
            "GIT_BLOB_LEXICAL_MATCH",
            "Read this task-matching test and verify which behavior its assertions cover.",
            "candidates",
            i,
        )?;
    }
    if context != &Value::Null {
        for (i, candidate) in context
            .get("test_candidates")?
            .array_ref()?
            .iter()
            .take(3)
            .enumerate()
        {
            // Filename-based tests are weaker suggestions and must not evict
            // matching test source just to maximize graph neighbor count.
            steps.push(
                candidate,
                "INSPECT_RELATED_TEST",
                "TEST_PATH_HEURISTIC",
                "Inspect the test assertions; its filename alone does not establish coverage.",
                "context.test_candidates",
                i,
            )?;
        }
    }
    // Related code is useful when it answers the task, but never outranks
    // original source and test inspection by default.
    let relation_budget = 3.min(budget.saturating_sub(steps.items.len()));
    for (i, neighbor) in related.iter().take(relation_budget).enumerate() {
        let assessment = neighbor.get("assessment")?.str()?;
        let direction = neighbor.get("direction")?.str()?;
        let (action, why) = if direction == "LEXICAL_SOURCE_TO_NEIGHBOR" {
            (
                "INSPECT_POSSIBLE_DEPENDENCY",
                "Check how the implementation uses this source; do not infer an executed call.",
            )
        } else if direction == "NEIGHBOR_TO_LEXICAL_SOURCE" {
            (
                "INSPECT_POSSIBLE_CALLER",
                "Check whether this source actually invokes or configures the focused behavior.",
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
    // Remaining explicit lexical matches are better fallback reading than
    // made-up behavioral edges.
    for (i, candidate) in task_sources.into_iter().skip(4) {
        steps.push(
            candidate,
            "INSPECT_TASK_MATCH",
            "GIT_BLOB_LEXICAL_MATCH",
            "Follow up on another independently located task-matching source file.",
            "candidates",
            i,
        )?;
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
        LocateBudget {
            index_segments,
            index: IndexLimits::default(),
        },
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
    budget: LocateBudget,
) -> Result<Value> {
    locate_with_mode(
        repo,
        rev,
        task,
        limit,
        roots,
        db,
        LocateExecution {
            budget,
            force_enrichment: true,
        },
    )
}

/// Default CLI discovery is source-first: query the pinned tree and only
/// create FTS/relationship evidence when the Git results cannot fill the
/// request, or when the caller explicitly asks for enrichment.
///
/// The existing locate_with_limits API remains fully enriched for callers
/// that depend on the native context and index receipts.
pub fn locate_with_mode(
    repo: &Path,
    rev: &str,
    task: &str,
    limit: usize,
    roots: Option<&str>,
    db: Option<&str>,
    execution: LocateExecution,
) -> Result<Value> {
    let budget = execution.budget;
    let force_enrichment = execution.force_enrichment;
    let limits = budget.index.validate()?;
    let index_segments = budget.index_segments;
    if !(1..=256).contains(&index_segments) {
        return refuse("agent locate --index-segments must be 1..=256");
    }
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
    // Git is authoritative for *text presence* in this committed tree. It
    // can find real test files that have no installed language indexer.
    // FTS remains a useful, independently ranked fallback, not the product
    // definition of source completeness.
    let committed = committed_matches(
        &discovered.root,
        &discovered.commit,
        &words,
        &inventory,
        limit,
    )?;
    let mut candidates = Vec::new();
    let mut selected_paths = Vec::new();
    let mut skipped_git_excerpts = 0usize;
    let mut selected_git_count = 0usize;
    if !committed.selected.is_empty() {
        let mut reader = git::BlobReader::open(&discovered.root)?;
        for entry in &committed.selected {
            // The entire original blob must be verified before exposing a
            // source excerpt. Refuse to claim coverage of an oversized,
            // binary or non-UTF-8 blob, and retain an explicit gap count.
            let body = match reader.read_capped(&entry.oid, limits.max_file_bytes) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) if !text.contains('\0') => text,
                    _ => {
                        skipped_git_excerpts += 1;
                        continue;
                    }
                },
                Err(error)
                    if error
                        .to_string()
                        .contains("Git source blob exceeds byte limit") =>
                {
                    skipped_git_excerpts += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            selected_paths.push(entry.path.clone());
            candidates.push(obj(&[
                ("rank", Value::number(candidates.len() + 1)),
                (
                    "source",
                    source(&discovered.commit, &discovered.tree, entry),
                ),
                ("excerpt", excerpt(&body, &words, 1)),
                ("ranking", Value::string("GIT_COMMITTED_DISTINCT_TERMS")),
                ("semantic_correctness", Value::string("UNVERIFIED")),
            ]));
            selected_git_count += 1;
        }
        reader.finish()?;
    }
    // A full, readable and nontruncated committed-tree result already
    // supplies as many authenticated source suggestions as the caller
    // requested. Indexing would add cost but no candidate in this report.
    // A separate explicit --enrich request retains the full graph workflow.
    let needs_enrichment = force_enrichment || candidates.len() < limit || committed.incomplete;
    let index = if needs_enrichment {
        Some(agent_locate_cache::search_with_limits(
            &discovered,
            entries,
            limit,
            &match_expression(&words),
            index_segments,
            source_path,
            limits,
        )?)
    } else {
        None
    };
    let indexed_hits = index
        .as_ref()
        .map(|found| found.hits.as_slice())
        .unwrap_or(&[]);
    for hit in indexed_hits {
        if candidates.len() == limit {
            break;
        }
        if selected_paths.contains(&hit.entry.path) {
            continue;
        }
        selected_paths.push(hit.entry.path.clone());
        candidates.push(obj(&[
            ("rank", Value::number(candidates.len() + 1)),
            (
                "source",
                source(&discovered.commit, &discovered.tree, &hit.entry),
            ),
            ("excerpt", excerpt(&hit.body, &words, hit.start_line)),
            ("ranking", Value::string("FTS5_BM25_TEXT_CANDIDATE")),
            ("semantic_correctness", Value::string("UNVERIFIED")),
        ]));
    }
    let skipped = index.as_ref().map_or(0, |found| found.skipped_files);
    let truncated = index.as_ref().is_some_and(|found| found.truncated);
    let bytes = index.as_ref().map_or(0, |found| found.indexed_bytes);
    let indexed_files = index.as_ref().map_or(0, |found| found.indexed_files);

    let mut gaps = vec![
        gap("TASK_SEMANTICS_NOT_VERIFIED"),
        gap("TEXT_MATCHES_ARE_NOT_SOURCE_DEPENDENCIES"),
        gap("UNCOMMITTED_CHANGES_EXCLUDED"),
        gap("TEST_SELECTION_NOT_PROVEN"),
    ];
    if committed.incomplete {
        gaps.push(gap("GIT_GREP_PATH_RESULTS_TRUNCATED"));
    }
    if skipped_git_excerpts > 0 {
        gaps.push(gap("GIT_GREP_SOURCE_EXCERPTS_UNAVAILABLE"));
    }
    if skipped > 0 {
        gaps.push(gap("SOURCE_CONTENT_NOT_FULLY_INDEXED"));
    }
    if truncated {
        gaps.push(gap("INDEX_BUDGET_EXHAUSTED"));
    }
    if index
        .as_ref()
        .is_some_and(|found| found.pending_segments > 0)
    {
        gaps.push(gap("SEGMENT_INDEX_INCOMPLETE"));
    }
    if index.is_none() {
        gaps.push(gap("FTS_NOT_QUERIED_GIT_RESULTS_SUFFICIENT"));
        gaps.push(gap("RELATIONSHIP_CONTEXT_NOT_QUERIED"));
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
    let context = if index.is_none() || context_paths.is_empty() {
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
        (
            "retrieval",
            Value::string(if index.is_none() {
                "COMMITTED_GIT_GREP"
            } else {
                "COMMITTED_GIT_GREP_WITH_FTS5_FALLBACK"
            }),
        ),
        (
            "enrichment",
            Value::string(if index.is_some() {
                "PERFORMED"
            } else {
                "NOT_REQUESTED_GIT_SUFFICIENT"
            }),
        ),
        (
            "git_search",
            obj(&[
                ("searched_terms", Value::number(words.len())),
                (
                    "matching_source_files",
                    Value::number(committed.source_file_count),
                ),
                ("selected_source_files", Value::number(selected_git_count)),
                (
                    "unreadable_matched_sources",
                    Value::number(skipped_git_excerpts),
                ),
                ("truncated", Value::Bool(committed.incomplete)),
            ]),
        ),
        ("candidates", Value::array(candidates)),
        ("candidate_count", Value::number(selected_paths.len())),
        ("relationship_candidates", Value::array(related)),
        ("reading_plan", plan),
        ("indexed_source_count", Value::number(indexed_files)),
        ("skipped_source_count", Value::number(skipped)),
        ("indexed_bytes", Value::number(bytes)),
        (
            "index",
            index.as_ref().map_or(Value::Null, |found| {
                obj(&[
                    ("segmentation", Value::string("PATH_SHA256_PREFIX_BYTE")),
                    (
                        "total_source_files",
                        Value::number(found.total_source_files),
                    ),
                    (
                        "populated_segments",
                        Value::number(found.populated_segments),
                    ),
                    ("ready_segments", Value::number(found.ready_segments)),
                    ("pending_segments", Value::number(found.pending_segments)),
                    (
                        "complete",
                        Value::Bool(found.pending_segments == 0 && skipped == 0 && !truncated),
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
                        ]),
                    ),
                ])
            }),
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
    let cache = index.as_ref().map_or(Value::Null, |found| {
        obj(&[
            ("kind", Value::string("PERSISTENT_SQLITE_FTS5")),
            ("path", Value::string(found.cache_path.to_string_lossy())),
            ("reused_blobs", Value::number(found.reused_blobs)),
            ("read_blobs", Value::number(found.read_blobs)),
            (
                "refreshed_segments",
                Value::number(found.refreshed_segments),
            ),
            ("refresh_budget_segments", Value::number(index_segments)),
        ])
    });
    output.entries_mut()?.insert("cache".into(), cache);
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
                "tests/test_worker.py",
                "src/handler.py",
                "src/store.py"
            ]
        );
        assert_eq!(
            steps[1].get("action").unwrap().str().unwrap(),
            "INSPECT_RELATED_TEST"
        );
        assert_eq!(
            steps[3].get("basis").unwrap().str().unwrap(),
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
            steps[4].get("action").unwrap().str().unwrap(),
            "INSPECT_RELATED_TEST"
        );
        assert_eq!(
            steps[4].get("basis").unwrap().str().unwrap(),
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
        // Node.js uses explicit CommonJS/ESM suffixes throughout its source
        // and tests; omitting them makes an index appear more complete than
        // the source inventory actually is.
        for path in [
            "lib/internal/modules/esm/loader.mjs",
            "test/fixtures/cycles/example.cjs",
            "test/fixtures/typescript/example.mts",
            "test/fixtures/typescript/example.cts",
        ] {
            assert!(
                source_path(path),
                "missing JavaScript/TypeScript source: {path}"
            );
        }
        assert!(!source_path("node_modules/pkg/index.cjs"));
        assert!(source_path("src/parse.y"));
        assert!(source_path("src/tokenizer.l"));
    }

    #[test]
    fn line_windows_are_bounded_and_one_based() {
        let source = "alpha\nbeta\nfn recover_lease() {}\ngamma\n";
        let value = excerpt(source, &["recover_lease".into()], 1);
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
    fn git_sufficient_fast_path_does_not_create_index_or_infer_relationships() {
        use std::fs;
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_FAST: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-locate-fast-{}-{}",
            std::process::id(),
            NEXT_FAST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("test")).unwrap();
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
            root.join("src/wal.c"),
            "int checkpoint_lock(void) { return 1; }\n",
        )
        .unwrap();
        fs::write(
            root.join("test/wal.test"),
            "do_test checkpoint-locks { checkpoint lock release }\n",
        )
        .unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "pinned source",
        ]);

        let budget = LocateBudget {
            index_segments: 4,
            index: IndexLimits::default(),
        };
        let discovered = discovery::inspect(&root, "HEAD").unwrap();
        let index_file = discovered
            .index_path
            .with_file_name("agent-locate-chunks.sqlite");
        assert!(!index_file.exists());
        // Operational budgets are validated even when the fast path never
        // reaches the indexer. Invalid flags cannot become silently accepted.
        assert!(locate_with_mode(
            &root,
            "HEAD",
            "checkpoint lock",
            2,
            None,
            None,
            LocateExecution {
                budget: LocateBudget {
                    index_segments: 0,
                    index: IndexLimits::default(),
                },
                force_enrichment: false,
            },
        ).is_err());
        let fast = locate_with_mode(
            &root,
            "HEAD",
            "checkpoint lock",
            2,
            None,
            None,
            LocateExecution {
                budget,
                force_enrichment: false,
            },
        )
        .unwrap();
        assert_eq!(
            fast.get("enrichment").unwrap().str().unwrap(),
            "NOT_REQUESTED_GIT_SUFFICIENT"
        );
        assert_eq!(
            fast.get("retrieval").unwrap().str().unwrap(),
            "COMMITTED_GIT_GREP"
        );
        assert_eq!(fast.get("index").unwrap(), &Value::Null);
        assert_eq!(fast.get("cache").unwrap(), &Value::Null);
        assert_eq!(fast.get("context").unwrap(), &Value::Null);
        assert!(
            fast.get("relationship_candidates")
                .unwrap()
                .array_ref()
                .unwrap()
                .is_empty()
        );
        let steps = fast
            .get("reading_plan")
            .unwrap()
            .get("steps")
            .unwrap()
            .array_ref()
            .unwrap();
        let paths = steps
            .iter()
            .map(|step| {
                step.get("source")
                    .unwrap()
                    .get("path")
                    .unwrap()
                    .str()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, ["src/wal.c", "test/wal.test"]);
        assert!(
            fast.get("coverage")
                .unwrap()
                .get("gaps")
                .unwrap()
                .array_ref()
                .unwrap()
                .iter()
                .any(|gap| gap.get("reason").unwrap().str().unwrap()
                    == "RELATIONSHIP_CONTEXT_NOT_QUERIED")
        );
        assert!(!index_file.exists());
        fs::write(root.join("src/wal.c"), "int replaced(void) { return 0; }\n").unwrap();
        let again = locate_with_mode(
            &root,
            "HEAD",
            "checkpoint lock",
            2,
            None,
            None,
            LocateExecution {
                budget,
                force_enrichment: false,
            },
        )
        .unwrap();
        assert_eq!(
            fast.get("report_id").unwrap(),
            again.get("report_id").unwrap()
        );

        // One more requested source file than Git found must activate
        // the existing FTS path instead of pretending two results suffice.
        let fallback = locate_with_mode(
            &root,
            "HEAD",
            "checkpoint lock",
            3,
            None,
            None,
            LocateExecution {
                budget,
                force_enrichment: false,
            },
        )
        .unwrap();
        assert_eq!(
            fallback.get("enrichment").unwrap().str().unwrap(),
            "PERFORMED"
        );
        assert_ne!(fallback.get("index").unwrap(), &Value::Null);
        assert_ne!(fallback.get("cache").unwrap(), &Value::Null);
        assert!(index_file.exists());
        let deep = locate_with_mode(
            &root,
            "HEAD",
            "checkpoint lock",
            2,
            None,
            None,
            LocateExecution {
                budget,
                force_enrichment: true,
            },
        )
        .unwrap();
        assert_eq!(deep.get("enrichment").unwrap().str().unwrap(), "PERFORMED");
        assert_ne!(deep.get("index").unwrap(), &Value::Null);
        assert_ne!(deep.get("cache").unwrap(), &Value::Null);
        assert_eq!(
            deep.get("candidates").unwrap().array_ref().unwrap()[0]
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap(),
            "src/wal.c"
        );
        let cache_dir = index_file.parent().unwrap();
        if cache_dir.exists() {
            fs::remove_dir_all(cache_dir).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
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
        fs::create_dir_all(root.join("test")).unwrap();
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
        fs::write(
            root.join("test/lease.test"),
            "do_test lease-recovery { lease checkout and worker assertion }\n",
        )
        .unwrap();
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
            0
        );
        assert!(first.get("context").unwrap().get("coverage").is_ok());
        assert_eq!(
            first.get("retrieval").unwrap().str().unwrap(),
            "COMMITTED_GIT_GREP_WITH_FTS5_FALLBACK"
        );
        assert!(
            first
                .get("git_search")
                .unwrap()
                .get("selected_source_files")
                .unwrap()
                .integer()
                .unwrap()
                >= 2
        );
        assert!(ranked.iter().any(|candidate| {
            candidate
                .get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                == "test/lease.test"
        }));
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
        assert!(steps.iter().any(|step| {
            step.get("source")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                == "test/lease.test"
                && step.get("action").unwrap().str().unwrap() == "INSPECT_RELATED_TEST"
        }));
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
            3
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
            3
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
