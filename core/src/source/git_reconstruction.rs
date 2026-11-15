//! Read-only replay of proposed construction histories against pinned Git objects.
//! A valid source closure is not evidence of behavioral correctness or original chronology.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_PLAN_BYTES: u64 = 1024 * 1024;
const MAX_STAGES: usize = 64;
const MAX_HISTORIES: usize = 8;
const MAX_CONTRACTS: usize = 256;
const MAX_EDGES: usize = 1024;
const MAX_FILES: usize = 20000;

fn exact_fields(value: &Value, fields: &[&str], context: &str) -> Result<()> {
    let keys = value.entries()?;
    if keys.len() != fields.len() || fields.iter().any(|key| !keys.contains_key(*key)) {
        return refuse(format!("{context} has missing or unexpected fields"));
    }
    Ok(())
}

fn strings(value: &Value) -> Result<Vec<String>> {
    value
        .array_ref()?
        .iter()
        .map(|v| Ok(v.str()?.to_owned()))
        .collect()
}

fn bounded<T>(items: &[T], maximum: usize, name: &str) -> Result<()> {
    if items.is_empty() || items.len() > maximum {
        return refuse(format!("{name} must contain 1..={maximum} entries"));
    }
    Ok(())
}

fn unique(values: &[String], name: &str) -> Result<BTreeSet<String>> {
    let set = values.iter().cloned().collect::<BTreeSet<_>>();
    if set.len() != values.len() || set.contains("") {
        return refuse(format!("{name} has empty or duplicate values"));
    }
    Ok(set)
}

#[derive(Clone)]
struct Contract {
    sources: BTreeSet<String>,
    requires: BTreeSet<String>,
}

fn tracked(repo: &Path, commit: &str) -> Result<BTreeMap<String, git::Entry>> {
    let mut entries = BTreeMap::new();
    for entry in git::entries(repo, commit)? {
        if entry.kind == "tree" {
            continue;
        }
        if entry.kind != "blob" {
            return refuse("Git reconstruction does not support gitlinks");
        }
        if entries.insert(entry.path.clone(), entry).is_some() || entries.len() > MAX_FILES {
            return refuse("duplicate or oversized Git tree inventory");
        }
    }
    Ok(entries)
}

fn check_sources(
    name: &str,
    paths: &BTreeSet<String>,
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    for source in paths {
        if let Some(required) = dependencies.get(source) {
            for dependency in required {
                if !paths.contains(dependency) {
                    return refuse(format!(
                        "{name} lacks declared dependency {dependency} of {source}"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn pinned_commit(repo: &Path, value: &str) -> Result<String> {
    git::oid(value)?;
    let commit = git::rev(repo, &format!("{value}^{{commit}}"))?;
    if commit != value {
        return refuse("Git reconstruction requires exact commit object identities");
    }
    Ok(commit)
}

/// Replay a bounded, explicitly authored plan. Does not execute source, modify
/// Git, infer missing dependencies, or promote opaque receipts to oracle proof.
/// Every reported source identity is read directly from immutable Git objects.
pub fn verify(repo: &Path, plan_path: &Path) -> Result<Value> {
    git::verify_repo(repo)?;
    let meta = fs::metadata(plan_path).map_err(|e| format!("plan metadata: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_PLAN_BYTES {
        return refuse("Git reconstruction plan must be a regular file <= 1 MiB");
    }
    let plan = json::parse(&fs::read(plan_path).map_err(|e| format!("plan read: {e}"))?)?;
    exact_fields(
        &plan,
        &[
            "schema",
            "target_commit",
            "source_dependencies",
            "contracts",
            "histories",
            "integration_nodes",
        ],
        "plan",
    )?;
    if plan.get("schema")?.str()? != "excession.git-reconstruction-plan.v1" {
        return refuse("unsupported Git reconstruction plan schema");
    }
    let target = pinned_commit(repo, plan.get("target_commit")?.str()?)?;
    let target_tree = git::rev(repo, &format!("{target}^{{tree}}"))?;
    let target_entries = tracked(repo, &target)?;
    let target_paths = target_entries.keys().cloned().collect::<BTreeSet<_>>();

    let edges = plan.get("source_dependencies")?.array_ref()?;
    if edges.len() > MAX_EDGES {
        return refuse("too many declared source dependencies");
    }
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in edges {
        exact_fields(edge, &["path", "requires"], "dependency edge")?;
        let path = edge.get("path")?.str()?.to_owned();
        git::validate_path(&path)?;
        if !target_paths.contains(&path) || dependencies.contains_key(&path) {
            return refuse("duplicate or non-target dependency source");
        }
        let required = strings(edge.get("requires")?)?;
        let required = unique(&required, "dependency prerequisites")?;
        for dependency in &required {
            git::validate_path(dependency)?;
            if !target_paths.contains(dependency) || dependency == &path {
                return refuse("invalid or non-target dependency prerequisite");
            }
        }
        dependencies.insert(path, required);
    }

    let declarations = plan.get("contracts")?.array_ref()?;
    bounded(declarations, MAX_CONTRACTS, "contracts")?;
    let mut contracts = BTreeMap::new();
    for declaration in declarations {
        exact_fields(
            declaration,
            &["id", "source_closure", "requires"],
            "contract",
        )?;
        let id = declaration.get("id")?.str()?.to_owned();
        if id.trim().is_empty() || id.len() > 128 || contracts.contains_key(&id) {
            return refuse("empty, oversized or duplicate contract identity");
        }
        let source_list = strings(declaration.get("source_closure")?)?;
        bounded(&source_list, MAX_FILES, "contract source closure")?;
        let sources = unique(&source_list, "contract source closure")?;
        for path in &sources {
            git::validate_path(path)?;
            if !target_paths.contains(path) {
                return refuse(format!("contract {id} refers to non-target source {path}"));
            }
        }
        check_sources(&id, &sources, &dependencies)?;
        let requires = unique(
            &strings(declaration.get("requires")?)?,
            "contract prerequisites",
        )?;
        if requires.contains(&id) {
            return refuse("self-dependent contract");
        }
        contracts.insert(id, Contract { sources, requires });
    }
    for contract in contracts.values() {
        if !contract
            .requires
            .iter()
            .all(|id| contracts.contains_key(id))
        {
            return refuse("contract refers to unknown prerequisite");
        }
    }

    let histories = plan.get("histories")?.array_ref()?;
    bounded(histories, MAX_HISTORIES, "histories")?;
    let mut names = BTreeSet::new();
    let mut history_reports = Vec::new();
    for history in histories {
        exact_fields(history, &["name", "checkpoints"], "history")?;
        let name = history.get("name")?.str()?.to_owned();
        if name.trim().is_empty() || name.len() > 128 || !names.insert(name.clone()) {
            return refuse("empty, oversized or duplicate history name");
        }
        let checkpoints = history.get("checkpoints")?.array_ref()?;
        bounded(checkpoints, MAX_STAGES, "checkpoints")?;
        let mut previous_commit: Option<String> = None;
        let mut previous_paths = BTreeSet::new();
        let mut introduced = BTreeSet::new();
        let mut stage_reports = Vec::new();
        for (ordinal, checkpoint) in checkpoints.iter().enumerate() {
            exact_fields(checkpoint, &["commit", "introduces"], "checkpoint")?;
            let commit = pinned_commit(repo, checkpoint.get("commit")?.str()?)?;
            if previous_commit.as_deref() == Some(&commit) {
                return refuse("repeated checkpoint Git commit");
            }
            if let Some(previous) = &previous_commit {
                // Actual history ancestry is checked independently of any logical
                // dependency order; Git parentage does not prove a source contract.
                git::command(repo, &["merge-base", "--is-ancestor", previous, &commit])
                    .map_err(|_| "checkpoint is not a descendant of its predecessor".to_owned())?;
            }
            let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
            let entries = tracked(repo, &commit)?;
            let paths = entries.keys().cloned().collect::<BTreeSet<_>>();
            if !previous_paths.is_subset(&paths) {
                return refuse("checkpoint dropped previously reconstructed source paths");
            }
            for (path, entry) in &entries {
                let Some(reference) = target_entries.get(path) else {
                    return refuse(format!("checkpoint includes non-target path: {path}"));
                };
                if entry.kind != reference.kind
                    || entry.mode != reference.mode
                    || entry.oid != reference.oid
                {
                    return refuse(format!("checkpoint changed target source identity: {path}"));
                }
            }
            check_sources(&format!("checkpoint {ordinal}"), &paths, &dependencies)?;
            let new_contracts = strings(checkpoint.get("introduces")?)?;
            let new_contracts_set = unique(&new_contracts, "checkpoint contract introductions")?;
            for id in &new_contracts_set {
                let Some(contract) = contracts.get(id) else {
                    return refuse(format!("checkpoint introduces undeclared contract {id}"));
                };
                if introduced.contains(id) {
                    return refuse(format!("contract introduced twice: {id}"));
                }
                if !contract.sources.is_subset(&paths) {
                    return refuse(format!(
                        "contract {id} introduced before source closure exists"
                    ));
                }
                if !contract.requires.is_subset(&introduced) {
                    return refuse(format!(
                        "contract {id} introduced before prerequisite claims"
                    ));
                }
            }
            introduced.extend(new_contracts_set.iter().cloned());
            for id in &introduced {
                if !contracts[id].sources.is_subset(&paths) {
                    return refuse("previously introduced contract lost source closure");
                }
            }
            stage_reports.push(obj(&[
                ("ordinal", Value::number(ordinal)),
                ("commit", Value::string(&commit)),
                ("tree", Value::string(tree)),
                ("source_entries", Value::number(paths.len())),
                (
                    "new_entries",
                    Value::number(paths.difference(&previous_paths).count()),
                ),
                (
                    "introduced_contracts",
                    Value::array(new_contracts.into_iter().map(Value::string)),
                ),
                ("retained_contract_count", Value::number(introduced.len())),
                ("source_identity_preserved", Value::Bool(true)),
                ("behavioral_preservation", Value::string("NOT_ESTABLISHED")),
            ]));
            previous_commit = Some(commit);
            previous_paths = paths;
        }
        if previous_paths != target_paths || introduced.len() != contracts.len() {
            return refuse(format!(
                "history {name} does not complete all target paths and contracts"
            ));
        }
        if git::rev(
            repo,
            &format!(
                "{}^{{tree}}",
                previous_commit.as_deref().ok_or("missing final commit")?
            ),
        )? != target_tree
        {
            return refuse(format!("history {name} final tree differs from target"));
        }
        history_reports.push(obj(&[
            ("name", Value::string(name)),
            (
                "final_commit",
                Value::string(previous_commit.ok_or("missing final commit")?),
            ),
            ("final_tree", Value::string(&target_tree)),
            ("checkpoints", Value::array(stage_reports)),
            (
                "structural_status",
                Value::string("EXACT_TARGET_TREE_AND_DECLARED_CLOSURES"),
            ),
        ]));
    }
    let integrations = plan.get("integration_nodes")?.array_ref()?;
    if integrations.len() > MAX_STAGES {
        return refuse("too many integration nodes");
    }
    let mut integration_commits = BTreeSet::new();
    let mut integration_reports = Vec::new();
    for integration in integrations {
        exact_fields(
            integration,
            &["commit", "parents", "preserves"],
            "integration node",
        )?;
        let commit = pinned_commit(repo, integration.get("commit")?.str()?)?;
        if !integration_commits.insert(commit.clone()) {
            return refuse("duplicate integration commit");
        }
        let raw_parents = strings(integration.get("parents")?)?;
        if raw_parents.len() != 2 || raw_parents[0] == raw_parents[1] {
            return refuse("integration needs two distinct parent commits");
        }
        for parent in &raw_parents {
            pinned_commit(repo, parent)?;
        }
        let observed = git::text(repo, &["rev-list", "--parents", "-n", "1", &commit])?;
        if observed != format!("{commit} {} {}", raw_parents[0], raw_parents[1]) {
            return refuse("integration commit does not have the declared exact parents");
        }
        let mut parents = Vec::new();
        let mut union = BTreeSet::new();
        for parent in &raw_parents {
            let entries = tracked(repo, parent)?;
            for (path, entry) in &entries {
                let Some(reference) = target_entries.get(path) else {
                    return refuse("integration parent contains a non-target path");
                };
                if entry.mode != reference.mode
                    || entry.oid != reference.oid
                    || entry.kind != reference.kind
                {
                    return refuse("integration parent changes pinned source identity");
                }
                union.insert(path.clone());
            }
            let paths = entries.keys().cloned().collect::<BTreeSet<_>>();
            check_sources("integration parent", &paths, &dependencies)?;
            parents.push(paths);
        }
        let merged = tracked(repo, &commit)?;
        let merged_paths = merged.keys().cloned().collect::<BTreeSet<_>>();
        if !union.is_subset(&merged_paths) {
            return refuse("integration dropped source from a parent");
        }
        check_sources("integration", &merged_paths, &dependencies)?;
        for (path, entry) in &merged {
            let Some(reference) = target_entries.get(path) else {
                return refuse("integration introduced non-target path");
            };
            if entry.mode != reference.mode
                || entry.oid != reference.oid
                || entry.kind != reference.kind
            {
                return refuse("integration changed pinned source identity");
            }
        }
        let preserves = strings(integration.get("preserves")?)?;
        bounded(&preserves, MAX_CONTRACTS, "integration preserved contracts")?;
        let preserved = unique(&preserves, "integration preserved contracts")?;
        for claim in &preserved {
            let Some(contract) = contracts.get(claim) else {
                return refuse("integration references an unknown contract");
            };
            if !parents
                .iter()
                .any(|paths| contract.sources.is_subset(paths))
                || !contract.sources.is_subset(&merged_paths)
            {
                return refuse(format!(
                    "integration does not preserve contract source closure: {claim}"
                ));
            }
        }
        let reachable = history_reports.iter().any(|history| {
            let final_commit = history.get("final_commit").ok().and_then(|v| v.str().ok());
            final_commit.is_some_and(|end| {
                git::command(repo, &["merge-base", "--is-ancestor", &commit, end]).is_ok()
            })
        });
        if !reachable {
            return refuse("integration is not reachable from any declared final history");
        }
        integration_reports.push(obj(&[
            ("commit", Value::string(&commit)),
            (
                "tree",
                Value::string(git::rev(repo, &format!("{commit}^{{tree}}"))?),
            ),
            (
                "parents",
                Value::array(raw_parents.into_iter().map(Value::string)),
            ),
            ("source_union_size", Value::number(union.len())),
            ("merged_source_entries", Value::number(merged_paths.len())),
            (
                "retained_contracts",
                Value::array(preserves.into_iter().map(Value::string)),
            ),
            ("source_identity_preserved", Value::Bool(true)),
            ("joint_behavior", Value::string("NOT_ESTABLISHED")),
        ]));
    }
    let mut report = obj(&[
        (
            "schema",
            Value::string("excession.git-reconstruction-report.v1"),
        ),
        ("target_commit", Value::string(target)),
        ("target_tree", Value::string(target_tree)),
        ("plan_sha256", Value::string(plan.sha256())),
        ("target_entry_count", Value::number(target_paths.len())),
        (
            "declared_dependency_source_count",
            Value::number(dependencies.len()),
        ),
        ("contract_count", Value::number(contracts.len())),
        ("histories", Value::array(history_reports)),
        ("integration_nodes", Value::array(integration_reports)),
        ("observed", Value::string("EXACT_GIT_SOURCE_IDENTITY")),
        ("dependency_coverage", Value::string("DECLARED_EDGES_ONLY")),
        ("original_history_recovered", Value::Bool(false)),
        ("build_provenance", Value::string("NOT_ESTABLISHED")),
        ("rea_evidence", Value::string("NOT_ESTABLISHED")),
        (
            "independent_contract_oracles",
            Value::string("NOT_ESTABLISHED"),
        ),
        ("integration_behavior", Value::string("NOT_ESTABLISHED")),
    ]);
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(report)
}
