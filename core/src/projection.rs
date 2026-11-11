//! Deterministic, read-only repository projections over existing Git/SQLite relations.
//! A view is not a source rewrite, executable checkout, authorization proof, or CVE verdict.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const FORMAT: &str = "excession.repository-view.v1";
const MANIFEST_NAME: &str = "view-manifest.json";
const MAX_ENTRIES: usize = 5_000;
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

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
    plan.check()?;
    let actual = source_table(repo, &plan.commit, &plan.tree)?;
    for e in &plan.entries {
        let source = actual
            .get(&e.source_path)
            .ok_or("view references an absent source")?;
        if source.oid != e.blob || source.mode != e.mode {
            return refuse(format!("view source identity mismatch: {}", e.source_path));
        }
    }
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

pub fn dependencies(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Value> {
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
    if tree != git::rev(&repo, &format!("{commit}^{{tree}}"))? {
        return refuse("indexed revision tree mismatch");
    }
    if index::reconstruct_tree(db, id, &commit)? != tree {
        return refuse("indexed Git tree reconstruction mismatch");
    }
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
    let mut forward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut backward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in db.query("SELECT source_name,target_name FROM dependency WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name", &[Param::Integer(id), Param::Text(&commit)])? {
        let from = required(&row,0)?.to_owned();
        let to = required(&row,1)?.to_owned();
        if !modules.contains_key(&from) || !modules.contains_key(&to) { return refuse("dependency refers to missing module"); }
        forward.entry(from.clone()).or_default().insert(to.clone());
        backward.entry(to).or_default().insert(from);
    }
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
    db.foreign_keys_valid()?;
    let plan = make_plan("dependencies", seed, commit, tree, entries)?;
    authenticate(&plan, &repo)?;
    Ok(plan.manifest())
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

fn same_revision(left: &Plan, right: &Plan) -> Result<()> {
    if left.commit != right.commit || left.tree != right.tree {
        return refuse("cannot compose projections from different Git source revisions");
    }
    Ok(())
}

fn source_identity(e: &Entry) -> (&str, &str, &str) {
    (&e.source_path, &e.blob, &e.mode)
}

/// Compose two independently checked view manifests by *source identity*, never
/// by substring matching their display names. Multiple virtual references to
/// one source blob are meaningful and retained when selected.
pub fn compose(repo: &Path, operator: &str, left: &Path, right: &Path) -> Result<Value> {
    if !matches!(operator, "union" | "intersection" | "difference") {
        return refuse("view composition supports union, intersection, difference only");
    }
    let a = checked_plan(repo, left)?;
    let b = checked_plan(repo, right)?;
    same_revision(&a, &b)?;
    let a_id = a.payload().sha256();
    let b_id = b.payload().sha256();
    let left_sources = a
        .entries
        .iter()
        .map(source_identity)
        .collect::<BTreeSet<_>>();
    let right_sources = b
        .entries
        .iter()
        .map(source_identity)
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeMap::<String, Entry>::new();
    for e in &a.entries {
        let keep = match operator {
            "union" => true,
            "intersection" => right_sources.contains(&source_identity(e)),
            "difference" => !right_sources.contains(&source_identity(e)),
            _ => unreachable!(),
        };
        if keep {
            selected.insert(e.virtual_path.clone(), e.clone());
        }
    }
    if operator == "union" {
        for e in &b.entries {
            // Same virtual pathname with different provenance/meaning cannot be
            // silently overwritten, even when the two blobs are byte-identical.
            if let Some(previous) = selected.get(&e.virtual_path) {
                if previous != e {
                    return refuse(format!("conflicting virtual view path: {}", e.virtual_path));
                }
            } else {
                selected.insert(e.virtual_path.clone(), e.clone());
            }
        }
    }
    // Sanity check ensures we have a source-based operation, rather than a
    // fortuitous equal count of unrelated directory entries.
    if operator == "intersection" && left_sources.is_disjoint(&right_sources) {
        return refuse("view intersection has no common source identity");
    }
    let query = obj(&[
        ("operator", Value::string(operator)),
        ("left_view_id", Value::string(a_id)),
        ("right_view_id", Value::string(b_id)),
        ("selection", Value::string("source-identity-v1")),
    ]);
    let plan = make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        a.commit,
        a.tree,
        selected.into_values().collect(),
    )?;
    Ok(plan.manifest())
}

/// Narrow an existing projection by a precise role or path-subtree. The
/// filter is deterministic and does not rely on a language parser.
pub fn select(repo: &Path, manifest: &Path, axis: &str, needle: &str) -> Result<Value> {
    if needle.is_empty() || needle.len() > 512 {
        return refuse("view filter value must contain 1..512 characters");
    }
    if !matches!(axis, "role" | "source-prefix" | "virtual-prefix") {
        return refuse("supported view filter axes: role, source-prefix, virtual-prefix");
    }
    if axis != "role" {
        git::validate_path(needle)?;
    }
    let input = checked_plan(repo, manifest)?;
    let input_id = input.payload().sha256();
    let entries = input
        .entries
        .into_iter()
        .filter(|e| match axis {
            "role" => e.role == needle,
            "source-prefix" => {
                e.source_path == needle || e.source_path.starts_with(&format!("{needle}/"))
            }
            "virtual-prefix" => {
                e.virtual_path == needle || e.virtual_path.starts_with(&format!("{needle}/"))
            }
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    let query = obj(&[
        ("operator", Value::string("select")),
        ("input_view_id", Value::string(input_id)),
        ("axis", Value::string(axis)),
        ("value", Value::string(needle)),
    ]);
    Ok(make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        input.commit,
        input.tree,
        entries,
    )?
    .manifest())
}

/// Place a view below a separate named virtual subtree. This permits two
/// independently focused layouts to coexist even if their original paths clash.
pub fn mount(repo: &Path, manifest: &Path, namespace: &str) -> Result<Value> {
    if namespace.is_empty()
        || namespace.len() > 64
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return refuse("view namespace must be 1..64 ASCII letters, digits, '-' or '_'");
    }
    let input = checked_plan(repo, manifest)?;
    let query = obj(&[
        ("operator", Value::string("mount")),
        ("input_view_id", Value::string(input.payload().sha256())),
        ("namespace", Value::string(namespace)),
    ]);
    let mut entries = input.entries;
    for e in &mut entries {
        let tail = e
            .virtual_path
            .strip_prefix("views/")
            .ok_or("virtual entry missing views/ root")?;
        e.virtual_path = format!("views/{namespace}/{tail}");
    }
    Ok(make_plan(
        "composed",
        &format!("query:{}", query.sha256()),
        input.commit,
        input.tree,
        entries,
    )?
    .manifest())
}

pub fn mount_replay(repo: &Path, input: &Path, namespace: &str, output: &Path) -> Result<Value> {
    replay(&mount(repo, input, namespace)?, output)
}

pub fn compose_replay(
    repo: &Path,
    operator: &str,
    left: &Path,
    right: &Path,
    output: &Path,
) -> Result<Value> {
    replay(&compose(repo, operator, left, right)?, output)
}

pub fn select_replay(
    repo: &Path,
    input: &Path,
    axis: &str,
    value: &str,
    output: &Path,
) -> Result<Value> {
    replay(&select(repo, input, axis, value)?, output)
}

fn get_entry<'a>(plan: &'a Plan, virtual_path: &str) -> Result<&'a Entry> {
    git::validate_path(virtual_path)?;
    plan.entries
        .iter()
        .find(|e| e.virtual_path == virtual_path)
        .ok_or_else(|| format!("no virtual file at path: {virtual_path}"))
}

/// Expose the path/source binding without requiring a filesystem projection.
pub fn describe(repo: &Path, manifest: &Path, virtual_path: &str) -> Result<Value> {
    let plan = checked_plan(repo, manifest)?;
    let entry = get_entry(&plan, virtual_path)?;
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("source_commit", Value::string(&plan.commit)),
        ("entry", entry.to_json()),
        ("status", Value::string("PINNED_GIT_SOURCE_NOT_EXECUTABLE")),
    ]))
}

/// List one level of the virtual hierarchy. Listing verifies the entire
/// manifest, not only the item requested by a potentially stale client.
pub fn list(repo: &Path, manifest: &Path, prefix: &str) -> Result<Value> {
    if !prefix.is_empty() {
        git::validate_path(prefix)?;
    }
    let plan = checked_plan(repo, manifest)?;
    let mut children = BTreeMap::<String, (bool, usize)>::new();
    let mut found = prefix.is_empty();
    for e in &plan.entries {
        let remainder = if prefix.is_empty() {
            e.virtual_path.as_str()
        } else if let Some(rest) = e.virtual_path.strip_prefix(&format!("{prefix}/")) {
            found = true;
            rest
        } else {
            continue;
        };
        let (name, directory) = if let Some((head, _)) = remainder.split_once('/') {
            (head, true)
        } else {
            (remainder, false)
        };
        let item = children.entry(name.to_owned()).or_insert((directory, 0));
        if item.0 != directory {
            return refuse("virtual file/directory name collision");
        }
        item.1 += 1;
    }
    if !found {
        return refuse("unknown virtual directory");
    }
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("directory", Value::string(prefix)),
        (
            "children",
            Value::array(
                children
                    .into_iter()
                    .map(|(name, (dir, count))| {
                        obj(&[
                            ("name", Value::string(name)),
                            (
                                "type",
                                Value::string(if dir { "directory" } else { "file" }),
                            ),
                            ("references", Value::number(count)),
                        ])
                    })
                    .collect::<Vec<_>>(),
            ),
        ),
    ]))
}

/// Read exactly one immutable Git blob on demand, without materialization.
/// The CLI emits the raw bytes to stdout; all metadata goes through describe.
pub fn open(repo: &Path, manifest: &Path, virtual_path: &str) -> Result<Vec<u8>> {
    let plan = checked_plan(repo, manifest)?;
    read_blob(repo, get_entry(&plan, virtual_path)?)
}

/// Check that a previously generated manifest is still the complete output of the
/// exact indexed query, not merely a collection of valid Git blob references.
/// For security runs the scanner report must also be independently audited via
/// `security-ingest-audit` before trusting the unverified flow relationships.
pub fn replay(expected: &Value, manifest: &Path) -> Result<Value> {
    let provided = read_plan(manifest)?;
    if expected.canonical() != provided.manifest().canonical() {
        return refuse("projection is not the complete deterministic query result");
    }
    Ok(obj(&[
        ("view_id", Value::string(provided.payload().sha256())),
        ("entries_replayed", Value::number(provided.entries.len())),
        ("status", Value::string("QUERY_REPLAY_MATCHED")),
    ]))
}

fn read_plan(path: &Path) -> Result<Plan> {
    Plan::from_bytes(&fs::read(path).map_err(|e| e.to_string())?)
}
fn verify_parent(repo: &Path, output: &Path) -> Result<()> {
    if !output.is_absolute() {
        return refuse("view output must be an absolute path");
    }
    let parent = output
        .parent()
        .ok_or("view output must have a parent directory")?
        .canonicalize()
        .map_err(|e| format!("output parent must already exist: {e}"))?;
    let root = repo.canonicalize().map_err(|e| e.to_string())?;
    if parent.starts_with(&root) {
        return refuse("view output must be outside source checkout");
    }
    if !parent.is_dir() {
        return refuse("output parent is not a directory");
    }
    Ok(())
}

fn read_blob(repo: &Path, e: &Entry) -> Result<Vec<u8>> {
    let raw = git::command(repo, &["cat-file", "blob", &e.blob])?;
    if raw.len() > MAX_BLOB_BYTES {
        return refuse("source blob larger than 16 MiB cap");
    }
    if digest::git_blob(&raw) != e.blob {
        return refuse("Git source blob content has invalid OID");
    }
    Ok(raw)
}

pub fn materialize(repo: &Path, manifest: &Path, target: &Path) -> Result<Value> {
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    verify_parent(repo, target)?;
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        return refuse("view destination already exists");
    }
    // Validate/read all content before creating output; limit total bytes including repeated refs.
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for e in &plan.entries {
        let data = read_blob(repo, e)?;
        total = total.checked_add(data.len()).ok_or("view size overflow")?;
        if total > MAX_TOTAL_BYTES {
            return refuse("view exceeds 128 MiB copy cap");
        }
        bytes.push(data);
    }
    fs::create_dir(target).map_err(|e| e.to_string())?;
    let result = (|| {
        for (e, data) in plan.entries.iter().zip(bytes.iter()) {
            let path = target.join(&e.virtual_path);
            fs::create_dir_all(path.parent().ok_or("invalid view path")?)
                .map_err(|e| e.to_string())?;
            fs::write(&path, data).map_err(|e| e.to_string())?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
                .map_err(|e| e.to_string())?;
        }
        let receipt = plan.manifest().canonical();
        fs::write(target.join(MANIFEST_NAME), receipt.as_bytes()).map_err(|e| e.to_string())?;
        audit(repo, manifest, target)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    result
}

fn walk(root: &Path, prefix: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let typ = entry.file_type().map_err(|e| e.to_string())?;
        if typ.is_symlink() {
            return refuse("symbolic link in materialized view");
        }
        if typ.is_dir() {
            walk(&entry.path(), prefix, output)?;
        } else if typ.is_file() {
            let entry_path = entry.path();
            let rel = entry_path.strip_prefix(prefix).map_err(|e| e.to_string())?;
            let name = rel
                .to_str()
                .ok_or("non-UTF8 output filename")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !output.insert(name) {
                return refuse("duplicate output file");
            }
        } else {
            return refuse("unexpected device or special file in view");
        }
    }
    Ok(())
}

pub fn audit(repo: &Path, manifest: &Path, target: &Path) -> Result<Value> {
    if !target
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return refuse("view root must be an actual directory, not a symlink");
    }
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    let receipt = fs::read(target.join(MANIFEST_NAME)).map_err(|e| e.to_string())?;
    if receipt != plan.manifest().canonical().as_bytes() {
        return refuse("view receipt mismatch");
    }
    let mut observed = BTreeSet::new();
    walk(target, target, &mut observed)?;
    let expected = plan
        .entries
        .iter()
        .map(|e| e.virtual_path.clone())
        .chain(std::iter::once(MANIFEST_NAME.into()))
        .collect::<BTreeSet<_>>();
    if observed != expected {
        return refuse("view contains extra or missing files");
    }
    let mut total = 0usize;
    for entry in &plan.entries {
        let contents = fs::read(target.join(&entry.virtual_path)).map_err(|e| e.to_string())?;
        if contents.len() > MAX_BLOB_BYTES {
            return refuse("view output exceeds blob cap");
        }
        total = total
            .checked_add(contents.len())
            .ok_or("view byte count overflow")?;
        if total > MAX_TOTAL_BYTES {
            return refuse("view output exceeds total cap");
        }
        if digest::git_blob(&contents) != entry.blob {
            return refuse("view contents differ from source blob");
        }
    }
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("kind", Value::string(plan.kind)),
        ("source_commit", Value::string(plan.commit)),
        ("entries_verified", Value::number(plan.entries.len())),
        ("bytes_verified", Value::number(total)),
        (
            "status",
            Value::string("SOURCE_BLOBS_VERIFIED_NOT_EXECUTABLE"),
        ),
    ]))
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
