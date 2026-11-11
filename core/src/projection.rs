//! Deterministic, read-only repository projections over existing Git/SQLite relations.
//! A view is not a source rewrite, executable checkout, authorization proof, or CVE verdict.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

mod composition;
mod workspace;

pub use composition::{compose, compose_replay, mount, mount_replay, select, select_replay};
pub use workspace::{audit, describe, list, materialize, open, replay};

const FORMAT: &str = "excession.repository-view.v1";
const MANIFEST_NAME: &str = "view-manifest.json";
const MAX_ENTRIES: usize = 5_000;
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

// Clock observations are deliberately outside manifest identity and are only
// enabled by the dedicated profiling command.
struct PhaseProfile {
    start: Instant,
    last: Instant,
    phases: Vec<(&'static str, u128)>,
}

impl PhaseProfile {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            phases: Vec::new(),
        }
    }

    fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        self.phases
            .push((name, now.duration_since(self.last).as_micros()));
        self.last = now;
    }

    fn elapsed_us(&self) -> u128 {
        self.start.elapsed().as_micros()
    }

    fn observations(&self) -> Value {
        Value::array(self.phases.iter().map(|(phase, elapsed_us)| {
            obj(&[
                ("phase", Value::string(*phase)),
                ("elapsed_us", Value::number(elapsed_us)),
            ])
        }))
    }
}

fn stamp(profile: &mut Option<&mut PhaseProfile>, phase: &'static str) {
    if let Some(clock) = profile.as_deref_mut() {
        clock.mark(phase);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    virtual_path: String,
    source_path: String,
    blob: String,
    mode: String,
    role: String,
    detail: String,
}

impl Entry {
    fn to_json(&self) -> Value {
        obj(&[
            ("virtual_path", Value::string(&self.virtual_path)),
            ("source_path", Value::string(&self.source_path)),
            ("blob", Value::string(&self.blob)),
            ("mode", Value::string(&self.mode)),
            ("role", Value::string(&self.role)),
            ("detail", Value::string(&self.detail)),
        ])
    }
    fn from_json(v: &Value) -> Result<Self> {
        keys(
            v,
            &[
                "virtual_path",
                "source_path",
                "blob",
                "mode",
                "role",
                "detail",
            ],
        )?;
        Ok(Self {
            virtual_path: v.get("virtual_path")?.str()?.to_owned(),
            source_path: v.get("source_path")?.str()?.to_owned(),
            blob: v.get("blob")?.str()?.to_owned(),
            mode: v.get("mode")?.str()?.to_owned(),
            role: v.get("role")?.str()?.to_owned(),
            detail: v.get("detail")?.str()?.to_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    kind: String,
    commit: String,
    tree: String,
    focus: String,
    entries: Vec<Entry>,
}

fn keys(v: &Value, expected: &[&str]) -> Result<()> {
    let m = v.entries()?;
    if m.len() != expected.len() || expected.iter().any(|key| !m.contains_key(*key)) {
        return refuse("unexpected or missing view manifest field");
    }
    Ok(())
}

impl Plan {
    fn payload(&self) -> Value {
        obj(&[
            ("schema", Value::string(FORMAT)),
            ("kind", Value::string(&self.kind)),
            ("source_commit", Value::string(&self.commit)),
            ("source_tree", Value::string(&self.tree)),
            ("focus", Value::string(&self.focus)),
            (
                "entries",
                Value::array(self.entries.iter().map(Entry::to_json)),
            ),
        ])
    }
    pub fn manifest(&self) -> Value {
        let mut fields = self.payload().entries().expect("payload object").clone();
        fields.insert("view_id".to_owned(), Value::string(self.payload().sha256()));
        Value::Object(fields)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 5 * 1024 * 1024 {
            return refuse("view manifest exceeds 5 MiB");
        }
        let v = json::parse(bytes)?;
        keys(
            &v,
            &[
                "schema",
                "kind",
                "source_commit",
                "source_tree",
                "focus",
                "entries",
                "view_id",
            ],
        )?;
        if v.get("schema")?.str()? != FORMAT {
            return refuse("unknown view manifest format");
        }
        let entries = v
            .get("entries")?
            .array_ref()?
            .iter()
            .map(Entry::from_json)
            .collect::<Result<Vec<_>>>()?;
        let plan = Self {
            kind: v.get("kind")?.str()?.to_owned(),
            commit: git::oid(v.get("source_commit")?.str()?)?,
            tree: git::oid(v.get("source_tree")?.str()?)?,
            focus: v.get("focus")?.str()?.to_owned(),
            entries,
        };
        plan.check()?;
        if plan.payload().sha256() != v.get("view_id")?.str()? {
            return refuse("view ID does not authenticate canonical manifest");
        }
        Ok(plan)
    }
    fn check(&self) -> Result<()> {
        git::oid(&self.commit)?;
        git::oid(&self.tree)?;
        if !matches!(
            self.kind.as_str(),
            "dependencies" | "security-flow" | "composed"
        ) || self.focus.is_empty()
        {
            return refuse("unsupported projection kind or missing focus");
        }
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRIES {
            return refuse("view requires 1..5000 entries");
        }
        let mut unique = BTreeSet::new();
        for e in &self.entries {
            git::validate_path(&e.virtual_path)?;
            if !e.virtual_path.starts_with("views/") {
                return refuse("virtual paths must live below views/");
            }
            git::validate_path(&e.source_path)?;
            git::oid(&e.blob)?;
            if !matches!(e.mode.as_str(), "100644" | "100755") {
                return refuse("non-regular source is not a view entry");
            }
            if e.role.is_empty() || e.role.len() > 80 || e.detail.len() > 512 {
                return refuse("invalid view role or detail");
            }
            if !unique.insert(e.virtual_path.as_str()) {
                return refuse("colliding virtual paths");
            }
        }
        if self
            .entries
            .windows(2)
            .any(|pair| pair[0].virtual_path >= pair[1].virtual_path)
        {
            return refuse("view entries are not canonically sorted");
        }
        // A virtual file must never simultaneously be a parent directory.
        // This also prevents ambiguous navigation and failed materialization.
        for e in &self.entries {
            let mut ancestor = e.virtual_path.as_str();
            while let Some((parent, _)) = ancestor.rsplit_once('/') {
                if unique.contains(parent) {
                    return refuse("virtual file/directory path collision");
                }
                ancestor = parent;
            }
        }
        Ok(())
    }
}

fn source_table(repo: &Path, commit: &str, tree: &str) -> Result<BTreeMap<String, git::Entry>> {
    git::verify_repo(repo)?;
    if git::rev(repo, &format!("{commit}^{{tree}}"))? != tree {
        return refuse("source commit and root tree disagree");
    }
    let mut table = BTreeMap::new();
    for entry in git::entries(repo, commit)? {
        if entry.kind == "blob" {
            table.insert(entry.path.clone(), entry);
        }
    }
    Ok(table)
}

fn authenticate(plan: &Plan, repo: &Path) -> Result<()> {
    authenticate_profiled(plan, repo, None)
}

fn authenticate_profiled(
    plan: &Plan,
    repo: &Path,
    mut profile: Option<&mut PhaseProfile>,
) -> Result<()> {
    plan.check()?;
    stamp(&mut profile, "validate_plan");
    let actual = source_table(repo, &plan.commit, &plan.tree)?;
    stamp(&mut profile, "read_git_source_tree");
    for e in &plan.entries {
        let source = actual
            .get(&e.source_path)
            .ok_or("view references an absent source")?;
        if source.oid != e.blob || source.mode != e.mode {
            return refuse(format!("view source identity mismatch: {}", e.source_path));
        }
    }
    stamp(&mut profile, "validate_source_bindings");
    Ok(())
}

fn make_plan(
    kind: &str,
    focus: &str,
    commit: String,
    tree: String,
    mut entries: Vec<Entry>,
) -> Result<Plan> {
    entries.sort_by(|a, b| a.virtual_path.cmp(&b.virtual_path));
    let plan = Plan {
        kind: kind.into(),
        commit,
        tree,
        focus: focus.into(),
        entries,
    };
    plan.check()?;
    Ok(plan)
}

#[derive(Clone)]
struct Module {
    path: String,
    blob: String,
    mode: String,
    test: bool,
}

fn required(row: &[Option<String>], i: usize) -> Result<&str> {
    row.get(i)
        .and_then(Option::as_deref)
        .ok_or_else(|| "unexpected SQLite NULL".to_owned())
}

fn bfs(seed: &str, graph: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, usize> {
    let mut distance = BTreeMap::from([(seed.to_owned(), 0)]);
    let mut queue = VecDeque::from([seed.to_owned()]);
    while let Some(node) = queue.pop_front() {
        let depth = distance[&node];
        if let Some(neighbors) = graph.get(&node) {
            for next in neighbors {
                if !distance.contains_key(next) {
                    distance.insert(next.clone(), depth + 1);
                    queue.push_back(next.clone());
                }
            }
        }
    }
    distance
}

fn dependency_plan_with_profile(
    db: &Db,
    name: &str,
    revision: &str,
    seed: &str,
    mut profile: Option<&mut PhaseProfile>,
) -> Result<Plan> {
    let id = db
        .scalar(
            "SELECT id FROM repository WHERE name=?",
            &[Param::Text(name)],
        )?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    let repo = PathBuf::from(db.scalar(
        "SELECT git_dir FROM repository WHERE id=?",
        &[Param::Integer(id)],
    )?);
    let commit = git::rev(&repo, revision)?;
    let tree = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(id), Param::Text(&commit)],
    )?;
    stamp(&mut profile, "resolve_revision");
    if tree != git::rev(&repo, &format!("{commit}^{{tree}}"))? {
        return refuse("indexed revision tree mismatch");
    }
    stamp(&mut profile, "check_git_tree");
    if index::reconstruct_tree(db, id, &commit)? != tree {
        return refuse("indexed Git tree reconstruction mismatch");
    }
    stamp(&mut profile, "reconstruct_index_tree");
    let mut modules = BTreeMap::new();
    for row in db.query("SELECT m.name,m.path,m.blob_oid,t.mode,m.is_test FROM module m JOIN tree_entry t ON t.repo_id=m.repo_id AND t.commit_oid=m.commit_oid AND t.path=m.path WHERE m.repo_id=? AND m.commit_oid=? ORDER BY m.name", &[Param::Integer(id),Param::Text(&commit)])? {
        modules.insert(required(&row,0)?.to_owned(), Module {
            path: required(&row,1)?.to_owned(), blob: required(&row,2)?.to_owned(),
            mode: required(&row,3)?.to_owned(), test: required(&row,4)? == "1",
        });
    }
    if !modules.contains_key(seed) {
        return refuse(format!("unknown indexed seed module: {seed}"));
    }
    stamp(&mut profile, "load_modules");
    let mut forward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut backward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in db.query("SELECT source_name,target_name FROM dependency WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name", &[Param::Integer(id), Param::Text(&commit)])? {
        let from = required(&row,0)?.to_owned();
        let to = required(&row,1)?.to_owned();
        if !modules.contains_key(&from) || !modules.contains_key(&to) { return refuse("dependency refers to missing module"); }
        forward.entry(from.clone()).or_default().insert(to.clone());
        backward.entry(to).or_default().insert(from);
    }
    stamp(&mut profile, "load_dependencies");
    let downstream = bfs(seed, &forward);
    let upstream = bfs(seed, &backward);
    let mut entries = Vec::new();
    for (direction, distances) in [("dependencies", downstream), ("dependents", upstream)] {
        for (module, distance) in distances {
            if distance == 0 {
                continue;
            }
            let m = &modules[&module];
            let role = if distance == 1 {
                format!("direct-{direction}")
            } else {
                format!("transitive-{direction}")
            };
            let group = if direction == "dependencies" {
                "10-dependencies"
            } else {
                "20-dependents"
            };
            let tier = if distance == 1 {
                "direct"
            } else {
                "transitive"
            };
            entries.push(Entry {
                virtual_path: format!("views/{group}/{tier}/{}", m.path),
                source_path: m.path.clone(),
                blob: m.blob.clone(),
                mode: m.mode.clone(),
                role,
                detail: format!("module={module};depth={distance}"),
            });
            if m.test {
                // Duplicate references are intentional, but the test-focused projection is distinct.
                entries.push(Entry {
                    virtual_path: format!("views/30-tests/{direction}/{}", m.path),
                    source_path: m.path.clone(),
                    blob: m.blob.clone(),
                    mode: m.mode.clone(),
                    role: "indexed-test".into(),
                    detail: format!("module={module};direction={direction}"),
                });
            }
        }
    }
    let m = &modules[seed];
    entries.push(Entry {
        virtual_path: format!("views/00-focus/{}", m.path),
        source_path: m.path.clone(),
        blob: m.blob.clone(),
        mode: m.mode.clone(),
        role: "focus".into(),
        detail: format!("module={seed}"),
    });
    stamp(&mut profile, "traverse_and_build_entries");
    db.foreign_keys_valid()?;
    let plan = make_plan("dependencies", seed, commit, tree, entries)?;
    stamp(&mut profile, "validate_index_and_plan");
    authenticate_profiled(&plan, &repo, profile)?;
    Ok(plan)
}

fn dependency_plan(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Plan> {
    dependency_plan_with_profile(db, name, revision, seed, None)
}

/// Profile the exact source-bound dependency path without changing the
/// production manifest, view identity, SQL queries or refusal rules.
/// Phase durations are process-local microsecond observations, not guarantees.
pub fn profile_dependencies(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Value> {
    let mut timer = PhaseProfile::new();
    let plan = dependency_plan_with_profile(db, name, revision, seed, Some(&mut timer))?;
    let view_id = plan.payload().sha256();
    timer.mark("canonical_view_id");
    let elapsed_us = timer.elapsed_us();
    Ok(obj(&[
        (
            "schema",
            Value::string("excession.projection-phase-profile.v1"),
        ),
        ("source_commit", Value::string(&plan.commit)),
        ("source_tree", Value::string(&plan.tree)),
        ("view_id", Value::string(view_id)),
        ("seed", Value::string(seed)),
        ("entry_count", Value::number(plan.entries.len())),
        ("total_us", Value::number(elapsed_us)),
        ("phases", timer.observations()),
    ]))
}

/// Generate the complete authenticated, materializable dependency projection.
pub fn dependencies(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Value> {
    Ok(dependency_plan(db, name, revision, seed)?.manifest())
}

/// Return only paths needed by a focused role query. The complete source-bound
/// projection is authenticated first, but its unrelated entries are not emitted.
/// This is a query result, not a view manifest or an independent correctness proof.
pub fn dependency_paths(
    db: &Db,
    name: &str,
    revision: &str,
    seed: &str,
    role: &str,
) -> Result<Value> {
    if !matches!(
        role,
        "focus"
            | "direct-dependencies"
            | "transitive-dependencies"
            | "direct-dependents"
            | "transitive-dependents"
            | "indexed-test"
    ) {
        return refuse("unknown dependency role");
    }
    let plan = dependency_plan(db, name, revision, seed)?;
    let paths = plan
        .entries
        .iter()
        .filter(|entry| entry.role == role)
        .map(|entry| entry.source_path.as_str())
        .collect::<BTreeSet<_>>();
    Ok(obj(&[
        ("schema", Value::string("excession.dependency-paths.v1")),
        ("source_commit", Value::string(&plan.commit)),
        ("source_tree", Value::string(&plan.tree)),
        ("view_id", Value::string(plan.payload().sha256())),
        ("role", Value::string(role)),
        (
            "source_paths",
            Value::array(paths.into_iter().map(Value::string)),
        ),
    ]))
}

pub fn security(db: &Db, repo: &Path, run_id: &str) -> Result<Value> {
    if run_id.len() != 64 || !run_id.bytes().all(|x| x.is_ascii_hexdigit()) {
        return refuse("invalid security run id");
    }
    let rows = db.query("SELECT source_commit,candidate_count,flow_step_count,status FROM security_ingest_run WHERE run_id=?", &[Param::Text(run_id)])?;
    if rows.len() != 1 {
        return refuse("unknown or duplicate security run");
    }
    let r = &rows[0];
    if required(r, 3)? != "UNVERIFIED_STATIC_CANDIDATES" {
        return refuse("not an unverified static-candidate run");
    }
    let commit = git::oid(required(r, 0)?)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let all_sources = source_table(repo, &commit, &tree)?;
    let mut entries = Vec::new();
    let candidates=db.query("SELECT candidate_id,source_path,source_blob,line,rule_id FROM security_ingest_candidate WHERE run_id=? ORDER BY candidate_id", &[Param::Text(run_id)])?;
    let steps=db.query("SELECT candidate_id,flow_index,thread_index,step_index,source_path,source_blob,line FROM security_ingest_flow_step WHERE run_id=? ORDER BY candidate_id,flow_index,thread_index,step_index", &[Param::Text(run_id)])?;
    if candidates.len().to_string() != required(r, 1)? || steps.len().to_string() != required(r, 2)?
    {
        return refuse("security run evidence row count mismatch");
    }
    let mut ids = BTreeSet::new();
    for row in candidates {
        let id = required(&row, 0)?;
        if id.len() != 64 || !id.bytes().all(|x| x.is_ascii_hexdigit()) {
            return refuse("invalid candidate id");
        }
        ids.insert(id.to_owned());
        let path = required(&row, 1)?;
        let source = all_sources
            .get(path)
            .ok_or("candidate source absent from Git")?;
        if source.oid != required(&row, 2)? {
            return refuse("candidate source blob disagreement");
        }
        entries.push(Entry {
            virtual_path: format!("views/00-findings/{id}/{path}"),
            source_path: path.into(),
            blob: source.oid.clone(),
            mode: source.mode.clone(),
            role: "unverified-static-candidate".into(),
            detail: format!("rule={};line={}", required(&row, 4)?, required(&row, 3)?),
        });
    }
    type ScannerFlow = BTreeMap<(String, i64, i64), Vec<(i64, String, String, i64)>>;
    let mut grouped: ScannerFlow = BTreeMap::new();
    for row in steps {
        let id = required(&row, 0)?;
        if !ids.contains(id) {
            return refuse("flow references missing candidate");
        }
        let parse_num = |n: usize| -> Result<i64> {
            required(&row, n)?
                .parse()
                .map_err(|e| format!("invalid flow sequence: {e}"))
        };
        let (flow, thread, step) = (parse_num(1)?, parse_num(2)?, parse_num(3)?);
        if flow < 0 || thread < 0 || step < 0 {
            return refuse("negative flow sequence");
        }
        let path = required(&row, 4)?.to_owned();
        let source = all_sources
            .get(&path)
            .ok_or("flow source absent from Git")?;
        if source.oid != required(&row, 5)? {
            return refuse("flow source blob mismatch");
        }
        grouped
            .entry((id.to_owned(), flow, thread))
            .or_default()
            .push((step, path, source.oid.clone(), parse_num(6)?));
    }
    for ((id, flow, thread), steps) in grouped {
        for (offset, (step, path, blob, line)) in steps.iter().enumerate() {
            if *step != offset as i64 {
                return refuse("noncontiguous ordered scanner flow");
            }
            let (group, role) = if steps.len() == 1 {
                ("10-flow-single", "scanner-single-step")
            } else if offset == 0 {
                ("10-flow-starts", "scanner-flow-start")
            } else if offset == steps.len() - 1 {
                ("30-flow-ends", "scanner-flow-end")
            } else {
                ("20-flow-intermediate", "scanner-flow-intermediate")
            };
            entries.push(Entry {
                virtual_path: format!(
                    "views/{group}/{id}/flow-{flow:03}/thread-{thread:03}/step-{step:04}/{path}"
                ),
                source_path: path.clone(),
                blob: blob.clone(),
                mode: all_sources[path].mode.clone(),
                role: role.into(),
                detail: format!("line={line};flow={flow};thread={thread};step={step}"),
            });
        }
    }
    db.foreign_keys_valid()?;
    let plan = make_plan("security-flow", run_id, commit, tree, entries)?;
    authenticate(&plan, repo)?;
    Ok(plan.manifest())
}

/// Virtual views are immutable selections over the same source tree. The repository
/// remains authoritative; neither composition nor navigation copies source bytes.
fn checked_plan(repo: &Path, manifest: &Path) -> Result<Plan> {
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    Ok(plan)
}

fn read_plan(path: &Path) -> Result<Plan> {
    Plan::from_bytes(&fs::read(path).map_err(|e| e.to_string())?)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> Plan {
        let fake = "a".repeat(40);
        make_plan(
            "dependencies",
            "m",
            fake.clone(),
            fake.clone(),
            vec![Entry {
                virtual_path: "views/00-focus/src/a.py".into(),
                source_path: "src/a.py".into(),
                blob: fake,
                mode: "100644".into(),
                role: "focus".into(),
                detail: "module=m".into(),
            }],
        )
        .unwrap()
    }
    #[test]
    fn manifest_roundtrip_is_stable() {
        let original = example();
        assert_eq!(
            Plan::from_bytes(original.manifest().canonical().as_bytes()).unwrap(),
            original
        );
    }
    #[test]
    fn manifest_rejects_forged_id() {
        let mut value = example().manifest();
        value
            .entries_mut()
            .unwrap()
            .insert("view_id".into(), Value::string("forged"));
        assert!(Plan::from_bytes(value.canonical().as_bytes()).is_err());
    }
    #[test]
    fn manifest_refuses_traversal_and_symlink_sources() {
        let mut p = example();
        p.entries[0].virtual_path = "views/../../outside".into();
        assert!(p.check().is_err());
        p = example();
        p.entries[0].mode = "120000".into();
        assert!(p.check().is_err());
    }
    #[test]
    fn bfs_cycles_terminate_and_choose_shortest_distance() {
        let mut g = BTreeMap::new();
        g.insert("a".into(), BTreeSet::from(["b".into(), "c".into()]));
        g.insert("b".into(), BTreeSet::from(["a".into(), "c".into()]));
        let distances = bfs("a", &g);
        assert_eq!(distances.get("c"), Some(&1));
        assert_eq!(distances.len(), 3);
    }
}
