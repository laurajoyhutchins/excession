//! Full-depth self-replay against independent Git plumbing, with an isolated
//! source-identical candidate commit. No source repository ref is moved.
use excession::history;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let result = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()
        .map_err(|e| format!("run git: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(|e| format!("git UTF-8: {e}"))
}

fn required<'a>(row: &'a Value, field: &str) -> Result<&'a str, String> {
    row[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}"))
}

fn arguments() -> Result<(PathBuf, PathBuf, usize), String> {
    let mut args = std::env::args().skip(1);
    let mut repo = None;
    let mut output = None;
    let mut limit = 100_000usize;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--repo" => repo = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--max-commits" => limit = value.parse().map_err(|_| "invalid max-commits")?,
            _ => return Err(format!("unknown option {option}")),
        }
    }
    if !(1..=100_000).contains(&limit) {
        return Err("max-commits must be 1..=100000".into());
    }
    let repo = repo
        .ok_or("missing --repo")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output: PathBuf = output.ok_or("missing --output")?;
    let parent = output
        .parent()
        .ok_or("output needs a parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(&repo) {
        return Err("output must be outside source checkout".into());
    }
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    if output.starts_with(&repo) {
        return Err("output must be outside source checkout".into());
    }
    Ok((repo, output, limit))
}

fn independently_check(repo: &Path, plan: &Value) -> Result<(usize, usize), String> {
    let mut parents = BTreeMap::<String, Vec<String>>::new();
    for line in git(repo, &["rev-list", "--parents", "HEAD"])?.lines() {
        let mut columns = line.split_whitespace();
        let oid = columns.next().ok_or("empty rev-list row")?;
        let prior = columns.map(str::to_owned).collect::<Vec<_>>();
        if parents.insert(oid.to_owned(), prior).is_some() {
            return Err("duplicate independent commit".into());
        }
    }
    let mut trees = BTreeMap::<String, String>::new();
    for line in git(
        repo,
        &["log", "--no-show-signature", "--format=%H:%T", "HEAD"],
    )?
    .lines()
    {
        let (oid, tree) = line.split_once(':').ok_or("malformed independent tree")?;
        if trees.insert(oid.to_owned(), tree.to_owned()).is_some() {
            return Err("duplicate independent tree".into());
        }
    }
    let rows = plan["commits"].as_array().ok_or("missing commits")?;
    if parents.len() != rows.len() || trees.len() != rows.len() {
        return Err("source plan does not cover full reachable Git graph".into());
    }
    let mut seen = BTreeSet::new();
    let mut merges = 0;
    for row in rows {
        let oid = required(row, "commit")?;
        if !seen.insert(oid) {
            return Err("duplicate planned commit".into());
        }
        let original = parents.get(oid).ok_or("planned commit not reachable")?;
        let declared = row["parents"]
            .as_array()
            .ok_or("missing parents")?
            .iter()
            .map(|v| v.as_str().ok_or("invalid parent").map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        if original != &declared {
            return Err(format!("parent mismatch at {oid}"));
        }
        if trees.get(oid).map(String::as_str) != Some(required(row, "tree")?) {
            return Err(format!("tree mismatch at {oid}"));
        }
        merges += usize::from(original.len() > 1);
    }
    if plan["commit_count"].as_u64() != Some(rows.len() as u64)
        || plan["summary"]["merge_commits"].as_u64() != Some(merges as u64)
    {
        return Err("aggregate plan counts disagree with independent Git DAG".into());
    }
    Ok((rows.len(), merges))
}

/// Independently replay the union of all locally available refs and detached
/// HEAD. This can be substantially larger than the HEAD ancestry alone.
fn replay_local_refs(
    repo: &Path,
    output: &Path,
    refs: &excession::json::Value,
    head: &str,
    limit: usize,
) -> Result<(usize, usize, usize), String> {
    let mut independent = BTreeMap::<String, Vec<String>>::new();
    for row in git(repo, &["rev-list", "--parents", "--all", "HEAD"])?.lines() {
        let mut fields = row.split_whitespace();
        let oid = fields.next().ok_or("empty all-ref DAG record")?;
        if independent
            .insert(oid.to_owned(), fields.map(str::to_owned).collect())
            .is_some()
        {
            return Err("duplicate all-ref DAG record".into());
        }
    }
    let mut expected_trees = BTreeMap::<String, String>::new();
    for row in git(
        repo,
        &["log", "--all", "--no-show-signature", "--format=%H:%T", "HEAD"],
    )?
    .lines()
    {
        let (commit, tree) = row.split_once(':').ok_or("malformed all-ref tree record")?;
        if expected_trees.insert(commit.to_owned(), tree.to_owned()).is_some() {
            return Err("duplicate all-ref tree record".into());
        }
    }
    if independent.len() != expected_trees.len() || independent.len() > limit {
        return Err("all-ref history is inconsistent or exceeds configured capacity".into());
    }
    let mut tips = BTreeSet::new();
    tips.insert(head.to_owned());
    let mut non_commit_refs = 0usize;
    for row in refs.get("refs")?.array_ref()? {
        let name = row.get("ref")?.str()?;
        let spec = format!("{name}^{{commit}}");
        match git(repo, &["rev-parse", "-q", "--verify", &spec]) {
            Ok(commit) => {
                tips.insert(commit.trim().to_owned());
            }
            Err(_) => non_commit_refs += 1,
        }
    }
    let mut seen = BTreeSet::new();
    let mut manifests = Vec::new();
    for tip in &tips {
        let plan = history::plan(repo, tip, None, limit)?;
        let report: Value = serde_json::from_str(&plan.canonical()).map_err(|e| e.to_string())?;
        if report["coverage"]["commit_range"] != "COMPLETE"
            || report["coverage"]["truncated"] != false
            || report["safety"]["rewrite_authorized"] != false
        {
            return Err(format!("ref tip {tip} has incomplete ancestry"));
        }
        let rows = report["commits"].as_array().ok_or("missing branch commits")?;
        for row in rows {
            let oid = required(row, "commit")?;
            let original = independent.get(oid).ok_or("ref plan includes an unreachable commit")?;
            let parents = row["parents"].as_array().ok_or("missing ref parents")?
                .iter().map(|p| p.as_str().ok_or("invalid ref parent").map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()?;
            if &parents != original
                || expected_trees.get(oid).map(String::as_str) != Some(required(row, "tree")?)
            {
                return Err(format!("ref plan differs from independent Git object at {oid}"));
            }
            seen.insert(oid.to_owned());
        }
        manifests.push(json!({
            "tip":tip,
            "plan_id":report["report_id"],
            "reachable_commit_count":rows.len(),
            "complete":true,
            "rewrite_authorized":false
        }));
    }
    if seen.len() != independent.len() || seen.len() != expected_trees.len() {
        return Err(format!(
            "local-ref union incomplete: {} planned, {} independently reachable",
            seen.len(),
            independent.len()
        ));
    }
    save(
        output,
        "ref-plans.json",
        &serde_json::to_string_pretty(&json!({
            "schema":"excession.history-local-ref-replay.v1",
            "pinned_head":head,
            "unique_reachable_commits":seen.len(),
            "replayed_distinct_tips":tips.len(),
            "non_commit_refs":non_commit_refs,
            "plans":manifests,
            "external_refs_complete":false,
            "rewrite_authorized":false
        })).map_err(|e| e.to_string())?,
    )?;
    Ok((seen.len(), tips.len(), non_commit_refs))
}

fn candidate(repo: &Path, output: &Path, plan: &Value) -> Result<Value, String> {
    let groups = plan["candidate_groups"]
        .as_array()
        .ok_or("missing review groups")?;
    let Some(group) = groups
        .iter()
        .max_by_key(|g| g["commits"].as_array().map_or(0, Vec::len))
    else {
        return Ok(json!({"status":"NO_CANDIDATE", "rewrite_authorized":false}));
    };
    let destination = output.join("ephemeral-preview");
    if destination.exists() {
        return Err("preview repository already exists".into());
    }
    let cloned = Command::new("git")
        .args(["clone", "--quiet", "--shared", "--no-checkout", "--"])
        .arg(repo)
        .arg(&destination)
        .status()
        .map_err(|e| format!("clone local history: {e}"))?;
    if !cloned.success() {
        return Err("isolated clone failed".into());
    }
    let parent = required(group, "original_parent")?;
    let tree = required(group, "result_tree")?;
    let old = group["commits"]
        .as_array()
        .ok_or("missing grouped commits")?
        .iter()
        .map(|v| v.as_str().ok_or("invalid group commit").map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    let message = format!(
        "EXCESSION SYNTHETIC REVIEW-ONLY COMMIT\n\nGroup of {} original commits; original tip {}\nNo source review, author equivalence or rewrite authorization.\n",
        old.len(),
        required(group, "original_tip")?
    );
    let output_commit = Command::new("git")
        .arg("-C")
        .arg(&destination)
        .args(["commit-tree", tree, "-p", parent, "-m", &message])
        .env("GIT_AUTHOR_NAME", "Excession temporary preview")
        .env("GIT_AUTHOR_EMAIL", "preview@example.invalid")
        .env("GIT_COMMITTER_NAME", "Excession temporary preview")
        .env("GIT_COMMITTER_EMAIL", "preview@example.invalid")
        .output()
        .map_err(|e| e.to_string())?;
    if !output_commit.status.success() {
        return Err(format!(
            "commit-tree refused: {}",
            String::from_utf8_lossy(&output_commit.stderr)
        ));
    }
    let new_oid = String::from_utf8(output_commit.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    if git(&destination, &["rev-parse", &format!("{new_oid}^{{tree}}")])?.trim() != tree {
        return Err("synthetic commit changed the group's terminal tree".into());
    }
    if git(
        &destination,
        &["rev-list", "--parents", "--max-count=1", &new_oid],
    )?
    .split_whitespace()
    .collect::<Vec<_>>()
        != vec![new_oid.as_str(), parent]
    {
        return Err("synthetic commit changed the required parent".into());
    }
    let mapping = old
        .iter()
        .map(|oid| {
            json!({
                "original":oid, "candidate":new_oid,
                "intermediate_state_preserved":false
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "status":"ISOLATED_TREE_IDENTICAL_PREVIEW",
        "original_group":old,
        "original_parent":parent,
        "terminal_tree":tree,
        "candidate_commit":new_oid,
        "old_to_new_mapping":mapping,
        "external_refs":"NOT_RECONCILED",
        "intermediate_history":"NOT_PRESERVED",
        "semantic_equivalence":"NOT_ASSESSED",
        "candidate_commit_ephemeral":true,
        "source_refs_changed":false,
        "rewrite_authorized":false
    }))
}

fn save(dir: &Path, name: &str, contents: &str) -> Result<(), String> {
    fs::write(dir.join(name), contents).map_err(|e| format!("save {name}: {e}"))
}

fn run() -> Result<(), String> {
    let (repo, output, limit) = arguments()?;
    if git(&repo, &["rev-parse", "--is-shallow-repository"])?.trim() != "false" {
        return Err("requires a full, non-shallow Git clone".into());
    }
    let head = git(&repo, &["rev-parse", "HEAD"])?;
    let before_refs = git(
        &repo,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname):%(objectname)",
        ],
    )?;
    let before_status = git(&repo, &["status", "--porcelain=v1"])?;
    let started = Instant::now();
    let plan = history::plan(&repo, head.trim(), None, limit)?;
    let plan_json: Value = serde_json::from_str(&plan.canonical()).map_err(|e| e.to_string())?;
    if plan_json["coverage"]["commit_range"] != "COMPLETE"
        || plan_json["coverage"]["truncated"] != false
        || plan_json["safety"]["rewrite_authorized"] != false
    {
        return Err("source DAG is partial or claims rewrite authorization".into());
    }
    let (commits, merges) = independently_check(&repo, &plan_json)?;
    let rev_count: usize = git(&repo, &["rev-list", "--count", "HEAD"])?
        .trim()
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    let all_refs: usize = git(&repo, &["rev-list", "--count", "--all", "HEAD"])?
        .trim()
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    if rev_count != commits || all_refs < commits {
        return Err("independent history counts disagree".into());
    }
    save(&output, "plan.json", &plan.canonical())?;
    let audit = history::audit(&repo, &output.join("plan.json"))?;
    if audit.get("status")?.str()? != "SOURCE_DAG_REPLAY_VERIFIED" {
        return Err("source-DAG audit failed".into());
    }
    save(&output, "audit.json", &audit.canonical())?;
    let refs = history::refs(&repo, 100_000)?;
    if refs.get("truncated")?.bool()? {
        return Err("local ref inventory truncated".into());
    }
    save(&output, "refs.json", &refs.canonical())?;
    let (unique_reachable, replayed_tips, non_commit_refs) =
        replay_local_refs(&repo, &output, &refs, head.trim(), limit)?;
    if unique_reachable != all_refs {
        return Err("all-ref replay count disagrees with independent Git count".into());
    }
    let target = history::verify_target(&repo, head.trim(), head.trim())?;
    if !target.get("exact_tree_equal")?.bool()? {
        return Err("original target tree differs".into());
    }
    save(&output, "target.json", &target.canonical())?;
    let preview = candidate(&repo, &output, &plan_json)?;
    save(
        &output,
        "preview.json",
        &serde_json::to_string_pretty(&preview).map_err(|e| e.to_string())?,
    )?;
    if git(&repo, &["rev-parse", "HEAD"])? != head
        || git(
            &repo,
            &[
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname):%(objectname)",
            ],
        )? != before_refs
        || git(&repo, &["status", "--porcelain=v1"])? != before_status
    {
        return Err("source refs, HEAD or worktree changed during replay".into());
    }
    let summary = json!({
        "schema":"excession.history-self-replay.v1",
        "original_tip":head.trim(),
        "plan_id":plan_json["report_id"],
        "commit_count":commits,
        "merge_commits":merges,
        "all_local_ref_reachable_commits":all_refs,
        "all_local_reachable_commits_replayed":unique_reachable,
        "other_local_reachable_commits":all_refs-commits,
        "distinct_replayed_commit_tips":replayed_tips,
        "non_commit_refs":non_commit_refs,
        "review_candidate_groups":plan_json["candidate_groups"].as_array().ok_or("groups")?.len(),
        "preview_status":preview["status"],
        "elapsed_ms":started.elapsed().as_millis() as u64,
        "independent_git_parent_and_tree_oracle":true,
        "original_ref_and_worktree_unchanged":true,
        "external_refs_complete":false,
        "rewrite_authorized":false
    });
    let text = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    save(&output, "summary.json", &text)?;
    println!("{text}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Excession full history self-replay refused: {error}");
        std::process::exit(1);
    }
}
