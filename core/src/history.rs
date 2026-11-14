//! Read-only, source-authenticated Git history inventory and consolidation proposals.
//! A proposed adjacent squash is a review candidate, never a semantic equivalence claim.

use crate::json::{self, Value, obj};
use crate::{Result, git, refuse};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const MAX_COMMITS: usize = 100_000;
const MAX_LOG_BYTES: usize = 64 * 1024 * 1024;
const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
struct Commit {
    oid: String,
    tree: String,
    parents: Vec<String>,
    subject: String,
    scope: Option<String>,
}

fn scope(subject: &str) -> Option<String> {
    let prefix = subject.split_once(':')?.0;
    if prefix.len() > 64 || prefix.is_empty() {
        return None;
    }
    let (kind, area) = match prefix.split_once('(') {
        Some((kind, area)) if area.ends_with(')') => (kind, &area[..area.len() - 1]),
        None => (prefix, prefix),
        _ => return None,
    };
    if !matches!(
        kind,
        "feat" | "fix" | "refactor" | "test" | "docs" | "ci" | "build"
            | "perf" | "chore" | "core" | "graph" | "syntax" | "analysis"
            | "security" | "views" | "observe" | "release" | "contracts"
            | "evidence" | "kernel" | "cli" | "inspect" | "bench"
    ) {
        return None;
    }
    if area.is_empty()
        || !area
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/'))
    {
        return None;
    }
    Some(area.to_owned())
}

fn parse_log(bytes: &[u8]) -> Result<Vec<Commit>> {
    if bytes.len() > MAX_LOG_BYTES {
        return refuse("history log exceeds 64 MiB; restrict the revision range");
    }
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.last() != Some(&0) {
        return refuse("unterminated NUL-delimited Git history");
    }
    let fields = bytes[..bytes.len() - 1]
        .split(|b| *b == 0)
        .collect::<Vec<_>>();
    if fields.len() % 4 != 0 {
        return refuse("malformed NUL-delimited Git history");
    }
    fields
        .chunks_exact(4)
        .map(|row| {
            let field = |index: usize| {
                std::str::from_utf8(row[index])
                    .map_err(|_| "non-UTF-8 Git history metadata".to_owned())
            };
            let oid = git::oid(field(0)?)?;
            let parents = field(1)?
                .split_whitespace()
                .map(git::oid)
                .collect::<Result<Vec<_>>>()?;
            let tree = git::oid(field(2)?)?;
            let subject = field(3)?.to_owned();
            let scope = scope(&subject);
            Ok(Commit {
                oid,
                tree,
                parents,
                subject,
                scope,
            })
        })
        .collect()
}

fn load(repo: &Path, tip: &str, base: Option<&str>, max: usize) -> Result<(Vec<Commit>, bool)> {
    if !(1..=MAX_COMMITS).contains(&max) {
        return refuse("history --max-commits must be in 1..=100000");
    }
    let range = base.map_or_else(|| tip.to_owned(), |b| format!("{b}..{tip}"));
    let count = format!("--max-count={}", max + 1);
    let args = [
        "log",
        "-z",
        "--topo-order",
        "--reverse",
        "--no-show-signature",
        "--format=%H%x00%P%x00%T%x00%s",
        count.as_str(),
        range.as_str(),
    ];
    let mut commits = parse_log(&git::command(repo, &args)?)?;
    let truncated = commits.len() > max;
    commits.truncate(max);
    Ok((commits, truncated))
}

fn array(values: impl IntoIterator<Item = String>) -> Value {
    Value::array(values.into_iter().map(Value::string))
}

fn candidate_group(commits: &[Commit]) -> Value {
    obj(&[
        ("kind", Value::string("ADJACENT_SCOPE_REVIEW")),
        (
            "reason",
            Value::string("Same declared commit-message scope and linear parent chain; behavior unverified"),
        ),
        ("declared_scope", Value::string(commits[0].scope.as_deref().unwrap_or(""))),
        ("commits", array(commits.iter().map(|c| c.oid.clone()))),
        ("original_parent", Value::string(&commits[0].parents[0])),
        ("original_tip", Value::string(&commits.last().unwrap().oid)),
        ("result_tree", Value::string(&commits.last().unwrap().tree)),
        ("semantic_equivalence", Value::string("UNVERIFIED")),
        ("approved", Value::Bool(false)),
    ])
}

fn groups(commits: &[Commit]) -> Vec<Value> {
    let mut result = Vec::new();
    let mut start = 0usize;
    while start < commits.len() {
        let mut end = start + 1;
        if commits[start].scope.is_some() && commits[start].parents.len() == 1 {
            while end < commits.len()
                && commits[end].parents.len() == 1
                && commits[end].parents[0] == commits[end - 1].oid
                && commits[end].scope == commits[start].scope
            {
                end += 1;
            }
        }
        if end - start >= 2 {
            result.push(candidate_group(&commits[start..end]));
        }
        start = end;
    }
    result
}

fn with_id(mut report: Value) -> Result<Value> {
    let id = report.sha256();
    report.entries_mut()?.insert("report_id".to_owned(), Value::string(id));
    Ok(report)
}

/// Inspect the original DAG without modifying branches, the index, or a worktree.
/// The result binds every displayed commit to its immutable Git tree and parents.
/// A limited result is explicitly incomplete and must not authorize a rewrite.
pub fn plan(repo: &Path, rev: &str, base: Option<&str>, max: usize) -> Result<Value> {
    git::verify_repo(repo)?;
    let tip = git::rev(repo, &format!("{rev}^{{commit}}"))?;
    let base = match base {
        Some(b) => {
            let b = git::rev(repo, &format!("{b}^{{commit}}"))?;
            if git::command(repo, &["merge-base", "--is-ancestor", &b, &tip]).is_err() {
                return refuse("history base must be an ancestor of the selected tip");
            }
            Some(b)
        }
        None => None,
    };
    let tip_tree = git::rev(repo, &format!("{tip}^{{tree}}"))?;
    let base_tree = match &base {
        Some(b) => Some(git::rev(repo, &format!("{b}^{{tree}}"))?),
        None => None,
    };
    let (commits, truncated) = load(repo, &tip, base.as_deref(), max)?;

    let trees = commits
        .iter()
        .map(|c| (c.oid.as_str(), c.tree.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut merges = 0usize;
    let mut no_tree_change = 0usize;
    let mut unresolved_parents = 0usize;
    let mut rows = Vec::with_capacity(commits.len());
    for commit in &commits {
        let first = commit.parents.first();
        let first_tree = first.and_then(|p| {
            trees.get(p.as_str()).copied().or_else(|| {
                if base.as_deref() == Some(p.as_str()) {
                    base_tree.as_deref()
                } else {
                    None
                }
            })
        });
        let unchanged = first_tree.map(|t| t == commit.tree);
        if unchanged == Some(true) {
            no_tree_change += 1;
        }
        merges += usize::from(commit.parents.len() > 1);
        unresolved_parents += usize::from(first.is_some() && first_tree.is_none());
        rows.push(obj(&[
            ("commit", Value::string(&commit.oid)),
            ("tree", Value::string(&commit.tree)),
            ("parents", array(commit.parents.iter().cloned())),
            ("subject", Value::string(&commit.subject)),
            (
                "declared_scope",
                commit.scope.as_deref().map(Value::string).unwrap_or(Value::Null),
            ),
            (
                "first_parent_tree_unchanged",
                unchanged.map(Value::Bool).unwrap_or(Value::Null),
            ),
            ("merge", Value::Bool(commit.parents.len() > 1)),
        ]));
    }

    let candidates = groups(&commits);
    with_id(obj(&[
        ("schema", Value::string("excession.history-plan.v1")),
        ("original_tip", Value::string(&tip)),
        ("original_tip_tree", Value::string(&tip_tree)),
        (
            "excluded_base",
            base.as_deref().map(Value::string).unwrap_or(Value::Null),
        ),
        ("max_commits", Value::number(max)),
        ("commit_count", Value::number(commits.len())),
        ("commits", Value::array(rows)),
        ("candidate_groups", Value::array(candidates)),
        (
            "summary",
            obj(&[
                ("merge_commits", Value::number(merges)),
                ("same_tree_as_first_parent", Value::number(no_tree_change)),
                ("unresolved_first_parent_trees", Value::number(unresolved_parents)),
            ]),
        ),
        (
            "coverage",
            obj(&[
                (
                    "commit_range",
                    Value::string(if truncated { "TRUNCATED" } else { "COMPLETE" }),
                ),
                ("truncated", Value::Bool(truncated)),
                ("semantic_equivalence", Value::string("NOT_ASSESSED")),
                ("evidence_receipts", Value::string("NOT_INVENTORIED")),
            ]),
        ),
        (
            "safety",
            obj(&[
                ("read_only", Value::Bool(true)),
                ("rewrite_authorized", Value::Bool(false)),
                ("source_tree_preservation", Value::string("REQUIRED_NOT_PROVEN")),
                ("external_refs_and_receipts", Value::string("NOT_YET_RECONCILED")),
                ("merges_preserved_by_default", Value::Bool(true)),
            ]),
        ),
    ]))
}

/// Recompute every record from the pinned original Git objects. No refs are moved.
pub fn audit(repo: &Path, saved_plan: &Path) -> Result<Value> {
    let metadata = fs::metadata(saved_plan).map_err(|e| format!("history plan metadata: {e}"))?;
    if metadata.len() > MAX_REPORT_BYTES {
        return refuse("history plan exceeds 64 MiB");
    }
    let bytes = fs::read(saved_plan).map_err(|e| format!("history plan read: {e}"))?;
    let provided = json::parse(&bytes)?;
    let id = provided.get("report_id")?.str()?.to_owned();
    let mut unsigned = provided.clone();
    unsigned.entries_mut()?.remove("report_id");
    if unsigned.sha256() != id {
        return refuse("history plan report_id does not authenticate its contents");
    }
    if provided.get("schema")?.str()? != "excession.history-plan.v1" {
        return refuse("unsupported history plan schema");
    }
    let base = match provided.get("excluded_base")? {
        Value::Null => None,
        Value::String(s) => Some(s.as_str()),
        _ => return refuse("invalid history excluded_base"),
    };
    let max = usize::try_from(provided.get("max_commits")?.integer()?)
        .map_err(|_| "invalid history max_commits".to_owned())?;
    let recomputed = plan(repo, provided.get("original_tip")?.str()?, base, max)?;
    if provided != recomputed {
        return refuse("history plan differs from pinned Git ancestry");
    }
    Ok(obj(&[
        ("schema", Value::string("excession.history-audit.v1")),
        ("plan_id", Value::string(id)),
        ("original_tip", provided.get("original_tip")?.clone()),
        ("original_tip_tree", provided.get("original_tip_tree")?.clone()),
        ("status", Value::string("SOURCE_DAG_REPLAY_VERIFIED")),
        ("rewrite_authorized", Value::Bool(false)),
        ("semantic_equivalence", Value::string("NOT_ASSESSED")),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "excession-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&root).args(args).output().unwrap();
            assert!(out.status.success(), "git: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q"]);
        fs::write(root.join("a"), "one").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=Test", "-c", "user.email=t@example.com", "commit", "-qm", "feat(core): first"]);
        fs::write(root.join("a"), "two").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=Test", "-c", "user.email=t@example.com", "commit", "-qm", "feat(core): second"]);
        fs::write(root.join("b"), "three").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=Test", "-c", "user.email=t@example.com", "commit", "-qm", "fix(core): third"]);
        root
    }

    #[test]
    fn proposes_only_linear_declared_scope_and_never_authorizes_rewrite() {
        let root = fixture();
        let report = plan(&root, "HEAD", None, 20).unwrap();
        assert_eq!(report.get("commit_count").unwrap().integer().unwrap(), 3);
        assert_eq!(report.get("candidate_groups").unwrap().array_ref().unwrap().len(), 1);
        assert_eq!(report.get("safety").unwrap().get("rewrite_authorized").unwrap().bool().unwrap(), false);
        assert_eq!(report.get("coverage").unwrap().get("commit_range").unwrap().str().unwrap(), "COMPLETE");
        fs::write(root.join("a"), "uncommitted edit").unwrap();
        assert_eq!(plan(&root, "HEAD", None, 20).unwrap(), report);
        let saved = root.join("plan.json");
        fs::write(&saved, report.canonical()).unwrap();
        assert_eq!(audit(&root, &saved).unwrap().get("status").unwrap().str().unwrap(), "SOURCE_DAG_REPLAY_VERIFIED");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn truncation_and_tampering_are_explicit() {
        let root = fixture();
        let small = plan(&root, "HEAD", None, 1).unwrap();
        assert!(small.get("coverage").unwrap().get("truncated").unwrap().bool().unwrap());
        let saved = root.join("plan.json");
        let mut bad = plan(&root, "HEAD", None, 20).unwrap();
        bad.entries_mut().unwrap().insert("commit_count".into(), Value::number(22));
        fs::write(&saved, bad.canonical()).unwrap();
        assert!(audit(&root, &saved).is_err());
        assert!(plan(&root, "HEAD", Some("HEAD~10"), 20).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_non_conventional_and_does_not_conflate_scopes() {
        assert_eq!(scope("feat(core): test"), Some("core".to_owned()));
        assert_eq!(scope("fix(core): test"), Some("core".to_owned()));
        assert_eq!(scope("feat(agent): test"), Some("agent".to_owned()));
        assert_eq!(scope("unrelated message"), None);
        assert_eq!(scope("feat(a b): bad"), None);
    }
}
