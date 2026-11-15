//! Isolated Git DAG condensation. Original Git objects and refs stay authoritative.
//! Synthetic commits never inherit original signatures, committer identity, or receipts.
use crate::{Result, git, history, refuse};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_COMMITS: usize = 100_000;
const MAX_REFS: usize = 100_000;
const PREVIEW_REF: &str = "refs/heads/excession-dag-preview";

#[derive(Clone)]
struct Node {
    tree: String,
    parents: Vec<String>,
    subject: String,
}

struct SourceGraph {
    commits: BTreeMap<String, Node>,
    order: Vec<String>,
    head_plan: Value,
    refs: Value,
    non_commit_refs: Vec<Value>,
}

fn git_run(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()
        .map_err(|e| format!("Git command failed to start: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "Git command returned non-UTF-8 output".to_owned())
}

fn strings(value: &Value) -> Result<Vec<String>> {
    value
        .as_array()
        .ok_or("expected JSON array")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("expected JSON string".into())
        })
        .collect()
}

fn json_report(value: &crate::json::Value) -> Result<Value> {
    serde_json::from_str(&value.canonical()).map_err(|e| format!("decode canonical report: {e}"))
}

fn save(output: &Path, name: &str, report: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    fs::write(output.join(name), bytes).map_err(|e| format!("write {name}: {e}"))
}

fn refs_before(repo: &Path) -> Result<String> {
    git_run(
        repo,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname) %(objectname)",
        ],
    )
}

fn full_graph(
    repo: &Path,
    head: &str,
    limit: usize,
) -> Result<SourceGraph> {
    let ref_report = history::refs(repo, MAX_REFS)?;
    let refs = json_report(&ref_report)?;
    if refs["truncated"] != false {
        return refuse("local ref inventory truncated");
    }
    if refs["refs"]
        .as_array()
        .ok_or("missing refs")?
        .iter()
        .any(|r| r["ref"].as_str().unwrap_or("").starts_with("refs/replace/"))
    {
        return refuse("Git replacement refs cannot be used to reconstruct true ancestry");
    }
    let mut tips = BTreeSet::new();
    tips.insert(head.to_owned());
    let mut non_commit_refs = Vec::new();
    for row in refs["refs"].as_array().ok_or("missing refs")? {
        let name = row["ref"].as_str().ok_or("invalid local ref name")?;
        match git_run(
            repo,
            &["rev-parse", "-q", "--verify", &format!("{name}^{{commit}}")],
        ) {
            Ok(tip) => {
                git::oid(&tip)?;
                tips.insert(tip);
            }
            Err(_) => non_commit_refs.push(json!({
                "ref":name,
                "object":row["object"],
                "status":"NON_COMMIT_REF_UNMAPPED"
            })),
        }
    }
    let head_plan = json_report(&history::plan(repo, head, None, limit)?)?;
    let mut graph = BTreeMap::<String, Node>::new();
    let mut manifests = Vec::new();
    for tip in &tips {
        let plan = json_report(&history::plan(repo, tip, None, limit)?)?;
        if plan["coverage"]["commit_range"] != "COMPLETE"
            || plan["coverage"]["truncated"] != false
            || plan["safety"]["rewrite_authorized"] != false
        {
            return refuse(format!("incomplete history at tip {tip}"));
        }
        for record in plan["commits"].as_array().ok_or("missing commits")? {
            let oid = record["commit"].as_str().ok_or("missing commit oid")?;
            let row = Node {
                tree: record["tree"]
                    .as_str()
                    .ok_or("missing Git tree")?
                    .to_owned(),
                parents: strings(&record["parents"])?,
                subject: record["subject"]
                    .as_str()
                    .ok_or("missing subject")?
                    .to_owned(),
            };
            if let Some(previous) = graph.insert(oid.to_owned(), row.clone())
                && (previous.tree != row.tree
                    || previous.parents != row.parents
                    || previous.subject != row.subject)
            {
                return refuse("conflicting Git identities across ref ancestry");
            }
        }
        manifests.push(json!({
            "tip":tip,
            "plan_id":plan["report_id"],
            "commit_count":plan["commit_count"],
            "status":"EXACT_GIT_ANCESTRY_REPLAYED"
        }));
    }
    let lines = git_run(
        repo,
        &[
            "rev-list",
            "--topo-order",
            "--reverse",
            "--parents",
            "--all",
            "HEAD",
        ],
    )?;
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    for line in lines.lines() {
        let mut fields = line.split_whitespace();
        let oid = fields.next().ok_or("empty Git ancestry row")?;
        let parents = fields.map(str::to_owned).collect::<Vec<_>>();
        let node = graph
            .get(oid)
            .ok_or("Git commit missing from full source plan")?;
        if node.parents != parents
            || !seen.insert(oid.to_owned())
            || parents.iter().any(|p| !seen.contains(p))
        {
            return refuse("Git parent mismatch, duplicate or non-topological ancestry");
        }
        order.push(oid.to_owned());
    }
    if order.len() != graph.len() || order.len() > limit {
        return refuse("complete local ref DAG exceeds limit or source plans");
    }
    Ok(SourceGraph {
        commits: graph,
        order,
        head_plan,
        refs,
        non_commit_refs,
    })
}

fn select_group(
    plan: &Value,
    graph: &BTreeMap<String, Node>,
    refs: &Value,
    head: &str,
    requested: Option<usize>,
) -> Result<(usize, Vec<String>, Vec<Value>, usize)> {
    let candidates = plan["candidate_groups"]
        .as_array()
        .ok_or("missing candidates")?;
    let mut referenced = BTreeSet::new();
    referenced.insert(head.to_owned());
    for row in refs["refs"].as_array().ok_or("missing refs")? {
        if let Some(oid) = row["peeled_object"].as_str() {
            referenced.insert(oid.to_owned());
        } else if row["object_type"] == "commit"
            && let Some(oid) = row["object"].as_str()
        {
            referenced.insert(oid.to_owned());
        }
    }
    let mut children = BTreeMap::<String, Vec<String>>::new();
    for (oid, node) in graph {
        for parent in &node.parents {
            children.entry(parent.clone()).or_default().push(oid.clone());
        }
    }
    let mut examined = Vec::new();
    let mut best: Option<(usize, Vec<String>, usize)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        if requested.is_some_and(|r| r != index) {
            continue;
        }
        let commits = strings(&candidate["commits"])?;
        let parent = candidate["original_parent"]
            .as_str()
            .ok_or("missing group parent")?;
        let mut safe = commits.len() > 1
            && graph
                .get(&commits[0])
                .is_some_and(|n| n.parents.len() == 1 && n.parents[0] == parent);
        for (i, oid) in commits.iter().enumerate() {
            let node = graph.get(oid).ok_or("candidate references unknown commit")?;
            if i > 0 && (node.parents.len() != 1 || node.parents[0] != commits[i - 1]) {
                safe = false;
            }
            if i + 1 < commits.len()
                && (referenced.contains(oid)
                    || children.get(oid).is_none_or(|links| {
                        links.len() != 1 || links[0] != commits[i + 1]
                    }))
            {
                safe = false;
            }
        }
        if !safe {
            examined.push(json!({"group_index":index,"status":"REF_OR_SIDE_BRANCH_DEPENDENCY"}));
            continue;
        }
        // Condense only one group. Its last commit is replaced, and all
        // transitively dependent descendants have to receive new parent OIDs.
        // Count this closure before constructing *any* Git objects, avoiding
        // expensive early-history folds for negligible compression.
        let mut affected = BTreeSet::new();
        let mut pending = vec![commits.last().ok_or("missing group terminal")?.clone()];
        while let Some(oid) = pending.pop() {
            if affected.insert(oid.clone())
                && let Some(next) = children.get(&oid)
            {
                pending.extend(next.iter().cloned());
            }
        }
        let cost = affected.len();
        examined.push(json!({
            "group_index":index,
            "status":"DEPENDENCY_SAFE",
            "estimated_synthetic_commits":cost,
            "removed_intermediate_commits":commits.len() - 1
        }));
        if requested.is_some() {
            return Ok((index, commits, examined, cost));
        }
        let better = best.as_ref().is_none_or(|(_, old, old_cost)| {
            cost < *old_cost || (cost == *old_cost && commits.len() > old.len())
        });
        if better {
            best = Some((index, commits, cost));
        }
    }
    if let Some((index, commits, cost)) = best {
        return Ok((index, commits, examined, cost));
    }
    if requested.is_some() {
        return refuse("selected group has a side-branch or ref dependency; refusing squash");
    }
    refuse(format!(
        "no dependency-safe group exists across all local refs ({} candidates inspected)",
        examined.len()
    ))
}

fn synthetic_commit(
    clone: &Path,
    tree: &str,
    parents: &[String],
    original: &str,
    label: &str,
) -> Result<String> {
    git::oid(tree)?;
    git::oid(original)?;
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(clone)
        .args(["-c", "commit.gpgSign=false", "commit-tree", tree]);
    for parent in parents {
        git::oid(parent)?;
        command.args(["-p", parent]);
    }
    command.args([
        "-m",
        &format!(
            "Excession isolated {label}\n\nOriginal commit: {original}\n\
         Synthetic provenance; original signer and committer not transferred.\n\
         External build, PR and REA receipts require reconciliation."
        ),
    ]);
    command
        .env("GIT_AUTHOR_NAME", "Excession synthetic history")
        .env("GIT_AUTHOR_EMAIL", "preview@example.invalid")
        .env("GIT_COMMITTER_NAME", "Excession synthetic history")
        .env("GIT_COMMITTER_EMAIL", "preview@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00 +0000")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00 +0000")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    let output = command
        .output()
        .map_err(|e| format!("create synthetic commit: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "synthetic commit-tree: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let oid = String::from_utf8(output.stdout).map_err(|_| "invalid synthetic Git OID")?;
    let oid = git::oid(oid.trim())?;
    if git_run(clone, &["rev-parse", &format!("{oid}^{{tree}}")])? != tree {
        return refuse("synthetic commit changed immutable source tree");
    }
    let actual = git_run(clone, &["rev-list", "--max-count=1", "--parents", &oid])?;
    let expected = std::iter::once(oid.as_str())
        .chain(parents.iter().map(String::as_str))
        .collect::<Vec<_>>();
    if actual.split_whitespace().collect::<Vec<_>>() != expected {
        return refuse("synthetic parent graph differs from requested mapped parents");
    }
    Ok(oid)
}

fn isolated_clone(repo: &Path, output: &Path) -> Result<PathBuf> {
    let clone = output.join("candidate.git");
    let status = Command::new("git")
        .args(["clone", "--mirror", "--no-hardlinks", "--"])
        .arg(repo)
        .arg(&clone)
        .status()
        .map_err(|e| format!("start isolated clone: {e}"))?;
    if !status.success() {
        return refuse("create independent preview clone failed");
    }
    Ok(clone)
}

pub fn preview(repo: &Path, output: &Path, requested: Option<usize>, max: usize) -> Result<Value> {
    if !(1..=MAX_COMMITS).contains(&max) {
        return refuse("max-commits must be 1..=100000");
    }
    git::verify_repo(repo)?;
    let repo = repo
        .canonicalize()
        .map_err(|e| format!("source checkout: {e}"))?;
    if git_run(&repo, &["rev-parse", "--is-shallow-repository"])? != "false" {
        return refuse("complete Git history is mandatory for DAG rewriting");
    }
    if output.exists() {
        return refuse("isolated output destination already exists");
    }
    let parent = output
        .parent()
        .ok_or("output needs parent directory")?
        .canonicalize()
        .map_err(|e| format!("output parent: {e}"))?;
    if parent.starts_with(&repo) {
        return refuse("output must not be within source checkout");
    }
    let before_refs = refs_before(&repo)?;
    let before_status = git_run(
        &repo,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    let head = git_run(&repo, &["rev-parse", "HEAD"])?;
    git::oid(&head)?;
    let SourceGraph {
        commits: graph,
        order,
        head_plan,
        refs,
        non_commit_refs,
    } = full_graph(&repo, &head, max)?;
    if refs["refs"].as_array().ok_or("missing refs")?
        .iter().any(|r| r["ref"] == PREVIEW_REF)
    {
        return refuse("original checkout already owns the reserved preview ref");
    }
    let (selected_index, group, examined_groups, estimated_synthetic_commits) =
        select_group(&head_plan, &graph, &refs, &head, requested)?;
    let start = group.first().ok_or("empty candidate group")?;
    let terminal = group.last().ok_or("empty candidate group")?;
    let old_parent = graph
        .get(start)
        .ok_or("unknown group start")?
        .parents
        .first()
        .ok_or("cannot collapse a root commit")?
        .clone();
    let folded = group.iter().cloned().collect::<BTreeSet<_>>();
    let mut children = BTreeMap::<String, usize>::new();
    for node in graph.values() {
        for parent in &node.parents {
            *children.entry(parent.clone()).or_default() += 1;
        }
    }
    // Safety: no extra child of an interior commit may survive the fold.
    for old in &group[..group.len() - 1] {
        if children.get(old).copied().unwrap_or_default() != 1 {
            return refuse("cannot fold a source commit with external descendants");
        }
    }

    fs::create_dir(output).map_err(|e| format!("create isolated output: {e}"))?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    let clone = isolated_clone(&repo, &output)?;
    let cloned_refs = refs_before(&clone)?;
    let mut mapped = BTreeMap::<String, String>::new();
    let mut generated = BTreeSet::new();
    let mut collapsed = false;
    for oid in &order {
        if folded.contains(oid) && oid != terminal {
            continue;
        }
        if oid == terminal {
            let replacement_parent = mapped
                .get(&old_parent)
                .ok_or("group parent absent")?
                .clone();
            let tree = &graph.get(terminal).ok_or("terminal missing")?.tree;
            let synthesized = synthetic_commit(&clone, tree, &[replacement_parent], oid, "squash")?;
            generated.insert(synthesized.clone());
            for member in &group {
                mapped.insert(member.clone(), synthesized.clone());
            }
            collapsed = true;
            continue;
        }
        let node = graph.get(oid).ok_or("source graph object missing")?;
        let parents = node
            .parents
            .iter()
            .map(|p| {
                mapped
                    .get(p)
                    .cloned()
                    .ok_or_else(|| format!("unmapped original parent {p}"))
            })
            .collect::<Result<Vec<_>>>()?;
        let distinct = parents.iter().collect::<BTreeSet<_>>();
        if distinct.len() != parents.len() {
            return refuse("mapped merge parents collapsed to one commit");
        }
        let new_oid = if parents == node.parents {
            oid.clone()
        } else {
            let new_oid = synthetic_commit(&clone, &node.tree, &parents, oid, "descendant replay")?;
            generated.insert(new_oid.clone());
            new_oid
        };
        mapped.insert(oid.clone(), new_oid);
    }
    if !collapsed || mapped.len() != graph.len() {
        return refuse("candidate DAG rewrite omitted source commits");
    }
    if generated.len() != estimated_synthetic_commits {
        return refuse("dependency closure estimate disagrees with constructed commit graph");
    }
    let rewritten_head = mapped.get(&head).ok_or("missing remapped HEAD")?.clone();
    if git_run(
        &clone,
        &["rev-parse", &format!("{rewritten_head}^{{tree}}")],
    )? != git_run(&repo, &["rev-parse", "HEAD^{tree}"])?
    {
        return refuse("candidate HEAD changed exact final source tree");
    }
    let mut mappings = Vec::new();
    let mut impacted = Vec::new();
    for (oid, node) in &graph {
        let new_oid = mapped.get(oid).ok_or("mapping incomplete")?;
        let terminal_tree = graph.get(terminal).ok_or("group terminal missing")?;
        let lost_intermediate = folded.contains(oid) && oid != terminal;
        let exact_tree = !lost_intermediate || node.tree == terminal_tree.tree;
        if new_oid != oid {
            impacted.push(oid.clone());
        }
        mappings.push(json!({
            "original":oid,
            "candidate":new_oid,
            "original_tree":node.tree,
            "exact_tree_preserved":exact_tree,
            "intermediate_state_dropped":lost_intermediate,
            "kind":if lost_intermediate {"COLLAPSED_INTERMEDIATE"}
                   else if oid == terminal {"COLLAPSED_TERMINAL"}
                   else if new_oid != oid {"DESCENDANT_REPARENTED"}
                   else {"UNCHANGED"},
            "original_parents":node.parents
        }));
    }
    let mut proposals = Vec::new();
    let mut annotated_impacted = 0usize;
    for entry in refs["refs"].as_array().ok_or("missing refs")? {
        let kind = entry["object_type"].as_str().unwrap_or("");
        let old_ref = entry["ref"].as_str().ok_or("invalid local ref")?;
        let peeled = if let Some(p) = entry["peeled_object"].as_str() {
            p.to_owned()
        } else if kind == "commit" {
            entry["object"]
                .as_str()
                .ok_or("invalid ref object")?
                .to_owned()
        } else {
            String::new()
        };
        let candidate_oid = mapped.get(&peeled);
        if candidate_oid.is_some_and(|c| c != &peeled) && kind == "tag" {
            annotated_impacted += 1;
        }
        proposals.push(json!({
            "ref":old_ref,
            "original_object":entry["object"],
            "original_commit":if peeled.is_empty() {None} else {Some(peeled.as_str())},
            "candidate_commit":candidate_oid,
            "tag_reissue_required":kind == "tag" && candidate_oid.is_some_and(|c| c != &peeled),
            "published":false
        }));
        if let Some(to) = candidate_oid {
            let original_tree = git_run(&repo, &["rev-parse", &format!("{peeled}^{{tree}}")])?;
            let candidate_tree = git_run(&clone, &["rev-parse", &format!("{to}^{{tree}}")])?;
            if original_tree != candidate_tree {
                return refuse(format!("ref {old_ref} changed source tree"));
            }
        }
    }
    if refs_before(&clone)? != cloned_refs {
        return refuse("isolated original refs changed during synthesis");
    }
    git_run(&clone, &[
        "update-ref",
        PREVIEW_REF,
        &rewritten_head,
        "0000000000000000000000000000000000000000",
    ])?;
    let clone_refs_after = refs_before(&clone)?;
    let clone_original_refs_after = clone_refs_after.lines()
        .filter(|line| !line.starts_with(&format!("{PREVIEW_REF} ")))
        .collect::<Vec<_>>()
        .join("\n");
    if clone_original_refs_after != cloned_refs {
        return refuse("isolated mirror modified original ref identities");
    }
    if refs_before(&repo)? != before_refs
        || git_run(
            &repo,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )? != before_status
        || git_run(&repo, &["rev-parse", "HEAD"])? != head
    {
        return refuse("original source refs or worktree mutated during isolated rewrite");
    }
    let head_tree = git_run(&repo, &["rev-parse", "HEAD^{tree}"])?;
    let receipts = json!({
        "schema":"excession.history-rewrite-obligations.v1",
        "changed_commit_ids":impacted.len(),
        "annotated_tags_requiring_reissue":annotated_impacted,
        "external_github_pr_heads":"NOT_RECONCILED",
        "build_provenance":"INVALIDATED_FOR_CHANGED_IDENTITIES",
        "rea_evidence":"REQUIRES_INDEPENDENT_REBINDING",
        "signatures":"ORIGINAL_OBJECTS_ONLY; SYNTHETIC_COMMITS_UNSIGNED",
        "intermediate_snapshots":"INTENTIONALLY_NOT_PRESERVED_FOR_FOLDED_COMMITS",
        "semantic_equivalence":"NOT_ASSESSED",
        "rewrite_authorized":false
    });
    save(
        &output,
        "mapping.json",
        &json!({
            "schema":"excession.history-dag-mapping.v1",
            "original_head":head,
            "candidate_head":rewritten_head,
            "original_head_tree":head_tree,
            "selected_group_index":selected_index,
            "selected_group":group,
            "source_commit_count":graph.len(),
            "synthetic_commit_count":generated.len(),
            "mapping":mappings,
            "examined_groups":examined_groups,
        "estimated_synthetic_commits":estimated_synthetic_commits,
            "rewrite_authorized":false
        }),
    )?;
    save(
        &output,
        "refs.json",
        &json!({
            "schema":"excession.history-dag-ref-proposals.v1",
            "proposed_refs":proposals,
            "non_commit_refs":non_commit_refs,
            "external_refs_complete":false,
            "original_refs_preserved":true,
            "candidate_ref_in_isolated_clone":PREVIEW_REF,
            "published":false
        }),
    )?;
    save(&output, "obligations.json", &receipts)?;
    let report = json!({
        "schema":"excession.history-dag-preview.v1",
        "status":"ISOLATED_DAG_REWRITE_TREE_VERIFIED",
        "original_head":head,
        "candidate_head":rewritten_head,
        "source_commit_count":graph.len(),
        "mapped_commit_count":mapped.len(),
        "synthetic_commit_count":generated.len(),
        "estimated_synthetic_commits":estimated_synthetic_commits,
        "changed_commit_ids":impacted.len(),
        "folded_commits":group.len(),
        "selected_group_index":selected_index,
        "exact_final_source_tree":true,
        "all_local_ref_terminal_trees_preserved":true,
        "original_refs_unchanged":true,
        "external_refs_complete":false,
        "build_and_rea_receipts_reconciled":false,
        "semantic_equivalence":"NOT_ASSESSED",
        "rewrite_authorized":false
    });
    save(&output, "summary.json", &report)?;
    Ok(report)
}
