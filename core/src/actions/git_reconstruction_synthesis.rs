//! Generate exact-blob candidate histories in a separate Git repository.
//! Git/source identities are observed; dependency and contract declarations are hypotheses.
//! Failure hints never establish that a candidate has repaired incorrect behavior.
use crate::json::{self, Value, obj};
use crate::{Result, git, git_reconstruction, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const MAX_INPUT: u64 = 1024 * 1024;
const MAX_FILES: usize = 20000;
const MAX_CONTRACTS: usize = 60;
const MAX_CANDIDATES: usize = 8;

fn fields(v: &Value, required: &[&str]) -> Result<()> {
    let map = v.entries()?;
    if map.len() != required.len() || required.iter().any(|k| !map.contains_key(*k)) {
        return refuse("reconstruction synthesis has unexpected or missing fields");
    }
    Ok(())
}
fn read(path: &Path) -> Result<Value> {
    let meta = fs::metadata(path).map_err(|e| format!("synthesis input metadata: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_INPUT {
        return refuse("synthesis JSON must be a bounded regular file");
    }
    let raw = fs::read(path).map_err(|e| format!("synthesis input read: {e}"))?;
    if raw.len() as u64 > MAX_INPUT {
        return refuse("synthesis input changed during read");
    }
    json::parse(&raw)
}
fn string_set(v: &Value, empty_ok: bool) -> Result<BTreeSet<String>> {
    let items = v.array_ref()?;
    let mut set = BTreeSet::new();
    for item in items {
        let value = item.str()?;
        if value.is_empty() || !set.insert(value.to_owned()) {
            return refuse("duplicate or empty synthesis identifier");
        }
    }
    if !empty_ok && set.is_empty() {
        return refuse("empty synthesis collection");
    }
    Ok(set)
}
fn json_array<'a>(s: impl Iterator<Item = &'a String>) -> Value {
    Value::array(s.map(Value::string).collect::<Vec<_>>())
}
fn git_run(
    repo: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    index: Option<&Path>,
) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Excession synthetic reconstruction")
        .env("GIT_AUTHOR_EMAIL", "synthetic@example.invalid")
        .env("GIT_COMMITTER_NAME", "Excession synthetic reconstruction")
        .env("GIT_COMMITTER_EMAIL", "synthetic@example.invalid")
        .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00 +0000")
        .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00 +0000")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("synthesis Git launch: {e}"))?;
    if let Some(data) = input {
        child
            .stdin
            .take()
            .ok_or("missing Git stdin")?
            .write_all(data)
            .map_err(|e| format!("synthesis Git stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("synthesis Git wait: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "synthetic Git {} refused: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(|_| "non-UTF-8 Git response")?
        .trim()
        .to_owned())
}

#[derive(Clone)]
struct Contract {
    sources: BTreeSet<String>,
    requires: BTreeSet<String>,
}
fn stage(
    repo: &Path,
    index: &Path,
    entries: &BTreeMap<String, git::Entry>,
    paths: &BTreeSet<String>,
    parents: &[&str],
    label: &str,
) -> Result<(String, String)> {
    git_run(repo, &["read-tree", "--empty"], None, Some(index))?;
    let mut index_info = Vec::new();
    for path in paths {
        let entry = entries
            .get(path)
            .ok_or("stage references nonexistent target source")?;
        index_info
            .extend_from_slice(format!("{} {}\t{}\n", entry.mode, entry.oid, path).as_bytes());
    }
    if !index_info.is_empty() {
        git_run(
            repo,
            &["update-index", "--index-info"],
            Some(&index_info),
            Some(index),
        )?;
    }
    let tree = git_run(repo, &["write-tree"], None, Some(index))?;
    git::oid(&tree)?;
    let mut args = vec!["commit-tree", tree.as_str()];
    for parent in parents {
        args.extend(["-p", parent]);
    }
    args.extend(["-m", label]);
    let commit = git_run(repo, &args, None, None)?;
    git::oid(&commit)?;
    Ok((commit, tree))
}
fn order(contracts: &BTreeMap<String, Contract>, variant: usize) -> Result<Vec<String>> {
    let mut done = BTreeSet::new();
    let mut result = Vec::new();
    let mut available_paths = BTreeSet::new();
    while done.len() < contracts.len() {
        let mut choices = contracts
            .iter()
            .filter(|(id, contract)| !done.contains(*id) && contract.requires.is_subset(&done))
            .map(|(id, contract)| {
                (
                    id.clone(),
                    contract.sources.difference(&available_paths).count(),
                )
            })
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return refuse("cyclic contract prerequisites cannot be synthesized");
        }
        match variant % 4 {
            0 => choices.sort_by(|a, b| a.0.cmp(&b.0)),
            1 => choices.sort_by(|a, b| b.0.cmp(&a.0)),
            2 => choices.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0))),
            _ => choices.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0))),
        }
        let id = choices[0].0.clone();
        available_paths.extend(contracts[&id].sources.iter().cloned());
        done.insert(id.clone());
        result.push(id);
    }
    Ok(result)
}

/// Generate 1..8 bounded, distinct topological construction orders, materialized
/// as authentic, rootless Git commits in `output/repository`. The source checkout
/// is read-only; the output directory must not exist and cannot be inside it.
/// Feedback is caller-supplied counterevidence, never authenticated by its shape.
pub fn synthesize(
    repo: &Path,
    seed_path: &Path,
    output: &Path,
    count: usize,
    feedback_path: Option<&Path>,
) -> Result<Value> {
    if !(1..=MAX_CANDIDATES).contains(&count) {
        return refuse("candidate count must be 1..=8");
    }
    git::verify_repo(repo)?;
    let repo_root = fs::canonicalize(repo).map_err(|e| format!("source checkout: {e}"))?;
    if output.exists() {
        return refuse("synthesis output already exists");
    }
    if !output.is_absolute() {
        return refuse("synthesis output must be absolute");
    }
    let parent = output.parent().ok_or("synthesis output has no parent")?;
    let resolved_parent = fs::canonicalize(parent).map_err(|e| format!("synthesis parent: {e}"))?;
    if resolved_parent.starts_with(&repo_root) {
        return refuse("synthesis output cannot be inside source checkout");
    }
    let seed = read(seed_path)?;
    fields(
        &seed,
        &[
            "schema",
            "target_commit",
            "source_dependencies",
            "contracts",
        ],
    )?;
    if seed.get("schema")?.str()? != "excession.git-reconstruction-seed.v1" {
        return refuse("unexpected synthesis seed schema");
    }
    let target = seed.get("target_commit")?.str()?;
    git::oid(target)?;
    if git::rev(repo, &format!("{target}^{{commit}}"))? != target {
        return refuse("synthesis target is not an exact commit");
    }
    let target_tree = git::rev(repo, &format!("{target}^{{tree}}"))?;
    let mut entries = BTreeMap::new();
    for entry in git::entries(repo, target)? {
        if entry.kind == "tree" {
            continue;
        }
        if entry.kind != "blob" {
            return refuse("synthesis cannot materialize Git submodules");
        }
        if entry.path.contains(['\n', '\r', '\t']) {
            return refuse("unsupported control character in Git path");
        }
        if entries.insert(entry.path.clone(), entry).is_some() || entries.len() > MAX_FILES {
            return refuse("duplicate or oversized target inventory");
        }
    }
    if entries.is_empty() {
        return refuse("cannot synthesize an empty target");
    }
    let target_paths = entries.keys().cloned().collect::<BTreeSet<_>>();
    let dependencies = seed.get("source_dependencies")?.array_ref()?;
    if dependencies.len() > 1024 {
        return refuse("too many source dependency declarations");
    }
    let mut seen_dependencies = BTreeSet::new();
    let mut deps = BTreeMap::<String, BTreeSet<String>>::new();
    for declaration in dependencies {
        fields(declaration, &["path", "requires"])?;
        let path = declaration.get("path")?.str()?;
        git::validate_path(path)?;
        if !target_paths.contains(path) || !seen_dependencies.insert(path) {
            return refuse("invalid dependency source");
        }
        let required = string_set(declaration.get("requires")?, true)?;
        if required.contains(path) || !required.is_subset(&target_paths) {
            return refuse("invalid dependency prerequisite");
        }
        deps.insert(path.to_owned(), required);
    }
    let declarations = seed.get("contracts")?.array_ref()?;
    if declarations.is_empty() || declarations.len() > MAX_CONTRACTS {
        return refuse("synthesis needs 1..=60 contracts");
    }
    let mut contracts = BTreeMap::new();
    for declaration in declarations {
        fields(declaration, &["id", "source_closure", "requires"])?;
        let id = declaration.get("id")?.str()?;
        if id.trim().is_empty()
            || id.len() > 128
            || id.contains(['\n', '\r'])
            || contracts.contains_key(id)
        {
            return refuse("invalid contract identity");
        }
        let sources = string_set(declaration.get("source_closure")?, false)?;
        let requires = string_set(declaration.get("requires")?, true)?;
        if !sources.is_subset(&target_paths) || requires.contains(id) {
            return refuse("invalid contract source or prerequisite");
        }
        for source in &sources {
            if deps
                .get(source)
                .is_some_and(|required| !required.is_subset(&sources))
            {
                return refuse("declared contract closure omits source dependency");
            }
        }
        contracts.insert(id.to_owned(), Contract { sources, requires });
    }
    if contracts
        .values()
        .any(|c| !c.requires.iter().all(|id| contracts.contains_key(id)))
    {
        return refuse("contract refers to unknown prerequisite");
    }
    // Check for cycles before writing anything.
    order(&contracts, 0)?;
    let mut failures = BTreeSet::<(String, String)>::new();
    let mut feedback_id = Value::Null;
    if let Some(path) = feedback_path {
        let feedback = read(path)?;
        fields(&feedback, &["schema", "failures"])?;
        if feedback.get("schema")?.str()? != "excession.git-reconstruction-counterexamples.v1" {
            return refuse("unexpected counterexample schema");
        }
        if feedback.get("failures")?.array_ref()?.len() > 256 {
            return refuse("counterexample budget exceeded");
        }
        for item in feedback.get("failures")?.array_ref()? {
            fields(item, &["contract", "tree", "oracle_sha256"])?;
            let contract = item.get("contract")?.str()?;
            let tree = item.get("tree")?.str()?;
            let oracle_sha = item.get("oracle_sha256")?.str()?;
            if !contracts.contains_key(contract) {
                return refuse("counterexample references unknown contract");
            }
            git::oid(tree)?;
            if oracle_sha.len() != 64
                || !oracle_sha
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return refuse("counterexample has malformed oracle digest");
            }
            if !failures.insert((contract.to_owned(), tree.to_owned())) {
                return refuse("duplicate counterexample");
            }
        }
        feedback_id = Value::string(feedback.sha256());
    }
    let mut orders = Vec::<Vec<String>>::new();
    for i in 0..16 {
        let choice = order(&contracts, i)?;
        if !orders.contains(&choice) {
            orders.push(choice);
        }
        if orders.len() == count {
            break;
        }
    }
    fs::create_dir(output).map_err(|e| format!("create synthesis output: {e}"))?;
    let result = (|| -> Result<Value> {
        let scratch = output.join("repository");
        fs::create_dir(&scratch).map_err(|e| format!("create synthetic Git repository: {e}"))?;
        git_run(
            &scratch,
            &["init", "-q", "--object-format=sha1"],
            None,
            None,
        )?;
        git_run(
            &scratch,
            &[
                "fetch",
                "-q",
                "--no-tags",
                "--no-recurse-submodules",
                &repo_root.to_string_lossy(),
                target,
            ],
            None,
            None,
        )?;
        let found = git::rev(&scratch, &format!("{target}^{{tree}}"))?;
        if found != target_tree {
            return refuse("imported target tree differs from source repository");
        }
        git_run(
            &scratch,
            &["update-ref", "refs/heads/reference", target],
            None,
            None,
        )?;
        let index = output.join("git-index");
        let mut candidate_reports = Vec::new();
        let mut histories = Vec::new();
        let mut oracle_selections = Vec::new();
        let mut failure_hits = 0usize;
        let mut integrations = Vec::new();
        // A genuine two-parent join is preferable to pretending one branch
        // developed after the other. Only use this when two contracts are
        // explicitly independent; arbitrary edge independence is not inferred.
        let independent_pair =
            contracts.len() == 2 && contracts.values().all(|c| c.requires.is_empty()) && count >= 2;
        if independent_pair {
            let names = contracts.keys().cloned().collect::<Vec<_>>();
            let (root, _) = stage(
                &scratch,
                &index,
                &entries,
                &BTreeSet::new(),
                &[],
                "synthetic empty foundation",
            )?;
            let (left, left_tree) = stage(
                &scratch,
                &index,
                &entries,
                &contracts[&names[0]].sources,
                &[&root],
                "synthetic left independent capability",
            )?;
            let (right, right_tree) = stage(
                &scratch,
                &index,
                &entries,
                &contracts[&names[1]].sources,
                &[&root],
                "synthetic right independent capability",
            )?;
            let union = contracts[&names[0]]
                .sources
                .union(&contracts[&names[1]].sources)
                .cloned()
                .collect::<BTreeSet<_>>();
            let (merged, merged_tree) = stage(
                &scratch,
                &index,
                &entries,
                &union,
                &[&left, &right],
                "synthetic two-parent contract integration",
            )?;
            let (completed, final_tree) = stage(
                &scratch,
                &index,
                &entries,
                &target_paths,
                &[&merged],
                "synthetic complete source tree",
            )?;
            if final_tree != target_tree {
                return refuse("synthetic join changed the exact final tree");
            }
            integrations.push(obj(&[
                ("commit", Value::string(&merged)),
                (
                    "parents",
                    Value::array(vec![Value::string(&left), Value::string(&right)]),
                ),
                ("preserves", json_array(names.iter())),
            ]));
            oracle_selections.push(obj(&[
                ("candidate", Value::string("ALL")),
                ("phase", Value::string("INTEGRATION")),
                ("contract", json_array(names.iter())),
                ("checkpoint", Value::string(&merged)),
                ("tree", Value::string(&merged_tree)),
                (
                    "state",
                    Value::string(
                        if names
                            .iter()
                            .any(|id| failures.contains(&(id.clone(), merged_tree.clone())))
                        {
                            "COUNTEREXAMPLE_REQUIRES_REPAIR"
                        } else {
                            "NEEDS_JOINT_INDEPENDENT_ORACLE"
                        },
                    ),
                ),
            ]));
            for (number, sequence) in orders.iter().enumerate() {
                let name = format!("synthetic-{number}");
                let (first, first_tree, second) = if sequence[0] == names[0] {
                    (&left, &left_tree, &names[1])
                } else {
                    (&right, &right_tree, &names[0])
                };
                let checkpoints = vec![
                    obj(&[
                        ("commit", Value::string(&root)),
                        ("introduces", Value::array(vec![])),
                    ]),
                    obj(&[
                        ("commit", Value::string(first)),
                        (
                            "introduces",
                            Value::array(vec![Value::string(&sequence[0])]),
                        ),
                    ]),
                    obj(&[
                        ("commit", Value::string(&merged)),
                        ("introduces", Value::array(vec![Value::string(second)])),
                    ]),
                    obj(&[
                        ("commit", Value::string(&completed)),
                        ("introduces", Value::array(vec![])),
                    ]),
                ];
                histories.push(obj(&[
                    ("name", Value::string(&name)),
                    ("checkpoints", Value::array(checkpoints)),
                ]));
                git_run(
                    &scratch,
                    &["update-ref", &format!("refs/heads/{name}"), &completed],
                    None,
                    None,
                )?;
                let stages = vec![
                    obj(&[
                        ("commit", Value::string(first)),
                        ("tree", Value::string(first_tree)),
                        ("introduced", Value::string(&sequence[0])),
                    ]),
                    obj(&[
                        ("commit", Value::string(&merged)),
                        ("tree", Value::string(&merged_tree)),
                        ("introduced", Value::string(second)),
                    ]),
                    obj(&[
                        ("commit", Value::string(&completed)),
                        ("tree", Value::string(&target_tree)),
                        ("introduced", Value::Null),
                    ]),
                ];
                for (contract, commit, tree) in [
                    (&sequence[0], first, first_tree),
                    (second, &merged, &merged_tree),
                ] {
                    if failures.contains(&(contract.clone(), tree.clone())) {
                        failure_hits += 1;
                    }
                    oracle_selections.push(obj(&[
                        ("candidate", Value::string(&name)),
                        ("phase", Value::string("INTRODUCTION")),
                        ("contract", Value::string(contract)),
                        ("checkpoint", Value::string(commit)),
                        ("tree", Value::string(tree)),
                        (
                            "state",
                            Value::string(
                                if failures.contains(&(contract.clone(), tree.clone())) {
                                    "COUNTEREXAMPLE_REQUIRES_REPAIR"
                                } else {
                                    "NEEDS_INDEPENDENT_ORACLE"
                                },
                            ),
                        ),
                    ]));
                }
                candidate_reports.push(obj(&[
                    ("name", Value::string(&name)),
                    ("order", json_array(sequence.iter())),
                    ("final_commit", Value::string(&completed)),
                    ("stages", Value::array(stages)),
                    ("verification", Value::string("NOT_EXECUTED")),
                ]));
            }
        } else {
            for (number, sequence) in orders.iter().enumerate() {
                let name = format!("synthetic-{number}");
                let mut paths = BTreeSet::new();
                let mut parent_commit: Option<String> = None;
                let mut checkpoints = Vec::new();
                let mut stage_reports = Vec::new();
                let mut seen = BTreeSet::new();
                let total = sequence.len() + 2;
                for ordinal in 0..total {
                    let introduced = if ordinal > 0 && ordinal <= sequence.len() {
                        Some(sequence[ordinal - 1].as_str())
                    } else {
                        None
                    };
                    if let Some(id) = introduced {
                        paths.extend(contracts[id].sources.iter().cloned());
                        seen.insert(id.to_owned());
                    }
                    if ordinal == sequence.len() + 1 {
                        paths = target_paths.clone();
                    }
                    let label = introduced.map_or_else(
                        || {
                            if ordinal == 0 {
                                "synthetic empty foundation".into()
                            } else {
                                "synthetic complete source tree".into()
                            }
                        },
                        |id| format!("synthetic introduction: {id}"),
                    );
                    let (commit, tree) = stage(
                        &scratch,
                        &index,
                        &entries,
                        &paths,
                        &parent_commit.as_deref().into_iter().collect::<Vec<_>>(),
                        &label,
                    )?;
                    if ordinal == sequence.len() + 1 && tree != target_tree {
                        return refuse("synthesized final Git tree differs from exact target");
                    }
                    let introduced_list =
                        introduced.map_or_else(Vec::new, |s| vec![Value::string(s)]);
                    checkpoints.push(obj(&[
                        ("commit", Value::string(&commit)),
                        ("introduces", Value::array(introduced_list)),
                    ]));
                    let blocked_contracts = seen
                        .iter()
                        .filter(|id| failures.contains(&((*id).clone(), tree.clone())))
                        .cloned()
                        .collect::<Vec<_>>();
                    failure_hits += blocked_contracts.len();
                    if let Some(id) = introduced {
                        oracle_selections.push(obj(&[
                            ("candidate", Value::string(&name)),
                            ("phase", Value::string("INTRODUCTION")),
                            ("contract", Value::string(id)),
                            ("checkpoint", Value::string(&commit)),
                            ("tree", Value::string(&tree)),
                            (
                                "state",
                                Value::string(
                                    if failures.contains(&(id.to_owned(), tree.clone())) {
                                        "COUNTEREXAMPLE_REQUIRES_REPAIR"
                                    } else {
                                        "NEEDS_INDEPENDENT_ORACLE"
                                    },
                                ),
                            ),
                        ]));
                    }
                    stage_reports.push(obj(&[
                        ("commit", Value::string(&commit)),
                        ("tree", Value::string(&tree)),
                        ("sources", Value::number(paths.len())),
                        ("introduced", introduced.map_or(Value::Null, Value::string)),
                        ("counterexamples", json_array(blocked_contracts.iter())),
                    ]));
                    parent_commit = Some(commit);
                }
                let final_commit = parent_commit.ok_or("missing synthetic final commit")?;
                git_run(
                    &scratch,
                    &["update-ref", &format!("refs/heads/{name}"), &final_commit],
                    None,
                    None,
                )?;
                histories.push(obj(&[
                    ("name", Value::string(&name)),
                    ("checkpoints", Value::array(checkpoints)),
                ]));
                candidate_reports.push(obj(&[
                    ("name", Value::string(&name)),
                    ("order", json_array(sequence.iter())),
                    ("final_commit", Value::string(final_commit)),
                    ("stages", Value::array(stage_reports)),
                    ("verification", Value::string("NOT_EXECUTED")),
                ]));
            }
        }
        let plan = obj(&[
            (
                "schema",
                Value::string("excession.git-reconstruction-plan.v1"),
            ),
            ("target_commit", Value::string(target)),
            (
                "source_dependencies",
                seed.get("source_dependencies")?.clone(),
            ),
            ("contracts", seed.get("contracts")?.clone()),
            ("histories", Value::array(histories)),
            ("integration_nodes", Value::array(integrations)),
        ]);
        let plan_path = output.join("plan.json");
        fs::write(&plan_path, format!("{}\n", plan.canonical()))
            .map_err(|e| format!("write generated plan: {e}"))?;
        let structural = git_reconstruction::verify(&scratch, &plan_path)?;
        let target_failed = failures.iter().any(|(_, tree)| tree == &target_tree);
        // Prefer the next structural candidate that does not reproduce a
        // caller-reported failing intermediate tree. This is a *retest priority*,
        // not a conclusion that reordering repaired a behavioral defect.
        let common_failure = oracle_selections.iter().any(|item| {
            item.get("candidate").ok().and_then(|v| v.str().ok()) == Some("ALL")
                && item.get("state").ok().and_then(|v| v.str().ok())
                    == Some("COUNTEREXAMPLE_REQUIRES_REPAIR")
        });
        let mut recommended = Value::Null;
        for candidate in &mut candidate_reports {
            let name = candidate.get("name")?.str()?.to_owned();
            let matched = oracle_selections
                .iter()
                .filter(|item| {
                    item.get("candidate").ok().and_then(|v| v.str().ok()) == Some(name.as_str())
                        && item.get("state").ok().and_then(|v| v.str().ok())
                            == Some("COUNTEREXAMPLE_REQUIRES_REPAIR")
                })
                .count()
                + usize::from(common_failure);
            candidate.entries_mut()?.insert(
                "known_counterexample_count".to_owned(),
                Value::number(matched),
            );
            candidate.entries_mut()?.insert(
                "candidate_status".to_owned(),
                Value::string(if target_failed || matched > 0 {
                    "COUNTEREXAMPLE_REQUIRES_REPAIR"
                } else {
                    "UNVERIFIED_NEEDS_ORACLES"
                }),
            );
            if !target_failed && matched == 0 && matches!(recommended, Value::Null) {
                recommended = Value::string(name);
            }
        }
        let mut report = obj(&[
            (
                "schema",
                Value::string("excession.git-reconstruction-synthesis.v1"),
            ),
            ("source_commit", Value::string(target)),
            ("target_tree", Value::string(&target_tree)),
            ("seed_sha256", Value::string(seed.sha256())),
            ("feedback_sha256", feedback_id),
            ("structural_report_id", structural.get("report_id")?.clone()),
            ("candidate_count", Value::number(candidate_reports.len())),
            ("candidates", Value::array(candidate_reports)),
            ("oracle_selections", Value::array(oracle_selections)),
            ("counterexample_matches", Value::number(failure_hits)),
            ("recommended_next_candidate", recommended),
            (
                "target_behavior",
                Value::string(if target_failed {
                    "COUNTEREXAMPLE_BLOCKS_ALL_SOURCE_IDENTICAL_HISTORIES"
                } else {
                    "NOT_ESTABLISHED"
                }),
            ),
            ("original_history_recovered", Value::Bool(false)),
            ("behavioral_correctness", Value::string("NOT_ESTABLISHED")),
            (
                "feedback_authority",
                Value::string("CALLER_SUPPLIED_UNATTESTED"),
            ),
            (
                "source_identity",
                Value::string("EXACT_TARGET_BLOBS_AND_MODES"),
            ),
        ]);
        let report_id = report.sha256();
        report
            .entries_mut()?
            .insert("report_id".into(), Value::string(report_id));
        fs::write(
            output.join("report.json"),
            format!("{}\n", report.canonical()),
        )
        .map_err(|e| format!("write synthesis report: {e}"))?;
        Ok(report)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}
