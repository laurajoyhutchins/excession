//! Read-only composition of independently produced source observations.
//! Native predicates and configured-build memberships retain separate meanings.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, git, refuse, scip_evidence};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_SCIP_OBSERVATIONS: usize = 4096;
const MAX_GRAPH_NODES: usize = 4096;
const MAX_GRAPH_EDGES: usize = 4096;

#[derive(Clone, Copy, Default)]
pub struct Selection<'a> {
    pub scip_snapshot: Option<&'a str>,
    pub graph_id: Option<&'a str>,
    pub source_prefix: Option<&'a str>,
}

impl Selection<'_> {
    pub fn is_empty(self) -> bool {
        self.scip_snapshot.is_none() && self.graph_id.is_none()
    }
}

#[derive(Default)]
pub struct Observed {
    pub relationships: Vec<Value>,
    pub configured_observations: Vec<Value>,
    pub coverage: Vec<Value>,
    pub receipt_ids: BTreeSet<String>,
    pub source_paths: BTreeSet<String>,
}

fn source(repo: &Path, commit: &str, tree: &str, path: &str, blob: &str) -> Result<Value> {
    git::validate_path(path)?;
    let entry = git::entry_at(repo, commit, path)?;
    if entry.oid != blob
        || entry.kind != "blob"
        || !matches!(entry.mode.as_str(), "100644" | "100755")
    {
        return refuse("selected producer source differs from pinned regular Git blob");
    }
    Ok(obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(path)),
        ("blob", Value::string(blob)),
        ("mode", Value::string(entry.mode)),
    ]))
}

fn digest_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return refuse("producer selection requires an exact lowercase SHA-256 identity");
    }
    Ok(())
}

fn select_scip(
    db: &Db,
    repo: &Path,
    commit: &str,
    tree: &str,
    snapshot: &str,
    prefix: Option<&str>,
    out: &mut Observed,
) -> Result<()> {
    digest_id(snapshot)?;
    // This replays the retained original index against the selected Git tree.
    // A matching SQLite receipt hash by itself does not authenticate source.
    let audit = scip_evidence::audit_source(repo, commit, prefix, db, snapshot)?;
    if audit.get("source_commit")?.str()? != commit || audit.get("source_tree")?.str()? != tree {
        return refuse("SCIP evidence belongs to a different Git revision");
    }
    let receipts = db.query(
        "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
        &[Param::Text(snapshot)],
    )?;
    if receipts.len() != 1 {
        return refuse("selected SCIP snapshot has no unique native producer receipt");
    }
    let receipt = json::parse(
        receipts[0][0]
            .as_deref()
            .ok_or("missing SCIP native receipt")?
            .as_bytes(),
    )?;
    let producer = receipt.get("producer")?.str()?;
    if !producer.starts_with("scip:") {
        return refuse("selected evidence is not native SCIP");
    }
    let rows = db.query(
        "SELECT document FROM evidence_observation_v1 WHERE snapshot_id=? ORDER BY observation_id",
        &[Param::Text(snapshot)],
    )?;
    if rows.len() > MAX_SCIP_OBSERVATIONS {
        return refuse("selected SCIP observations exceed integrated-query budget");
    }
    let mut coverage = None;
    for row in rows {
        let record = json::parse(
            row[0]
                .as_deref()
                .ok_or("missing SCIP observation")?
                .as_bytes(),
        )?;
        let kind = record.get("native_kind")?.str()?;
        let payload = record.get("payload")?;
        if kind == "SCIP_INDEX_COVERAGE" {
            if coverage.replace(payload.clone()).is_some() {
                return refuse("duplicate selected SCIP coverage record");
            }
            continue;
        }
        if kind != "SCIP_SYMBOL_REFERENCE" {
            return refuse("unexpected native kind in selected SCIP snapshot");
        }
        let from = payload.get("from")?;
        let to = payload.get("to")?;
        let from_path = from.get("path")?.str()?;
        let to_path = to.get("path")?.str()?;
        let from_source = source(repo, commit, tree, from_path, from.get("blob")?.str()?)?;
        let to_source = source(repo, commit, tree, to_path, to.get("blob")?.str()?)?;
        out.source_paths.insert(from_path.to_owned());
        out.source_paths.insert(to_path.to_owned());
        let receipt_id = record.get("receipt_id")?.str()?;
        out.receipt_ids.insert(receipt_id.to_owned());
        out.relationships.push(obj(&[
            ("source", from_source.clone()),
            ("target", to_source),
            ("kind", Value::string("SCIP_REFERENCE")),
            (
                "evidence",
                Value::array([obj(&[
                    ("receipt_id", Value::string(receipt_id)),
                    ("producer", Value::string(producer)),
                    ("observation_kind", Value::string("SCIP_REFERENCE")),
                    ("assessment", Value::string("OBSERVED")),
                    ("source", from_source),
                ])]),
            ),
            (
                "native_observation",
                obj(&[
                    ("snapshot_id", Value::string(snapshot)),
                    ("observation_id", record.get("observation_id")?.clone()),
                    ("native_kind", Value::string(kind)),
                    ("symbol", payload.get("symbol")?.clone()),
                    ("line", payload.get("line")?.clone()),
                ]),
            ),
        ]));
    }
    let coverage = coverage.ok_or("selected SCIP snapshot has no coverage record")?;
    out.coverage.push(obj(&[
        ("producer", Value::string(producer)),
        ("snapshot_id", Value::string(snapshot)),
        ("coverage", coverage),
    ]));
    Ok(())
}

fn select_graph(
    db: &Db,
    repo: &Path,
    commit: &str,
    tree: &str,
    graph_id: &str,
    out: &mut Observed,
) -> Result<()> {
    digest_id(graph_id)?;
    let rows = db.query(
        "SELECT source_commit,document FROM configured_graph WHERE graph_id=?",
        &[Param::Text(graph_id)],
    )?;
    if rows.len() != 1 || rows[0][0].as_deref() != Some(commit) {
        return refuse("configured graph missing or belongs to a different Git revision");
    }
    let raw = rows[0][1]
        .as_deref()
        .ok_or("configured graph document missing")?;
    let graph = json::parse(raw.as_bytes())?;
    if graph.sha256() != graph_id
        || graph.get("source_commit")?.str()? != commit
        || graph.get("source_tree")?.str()? != tree
    {
        return refuse("configured graph identity or Git revision mismatch");
    }
    if !matches!(
        graph.get("schema")?.str()?,
        "excession.configured-graph.v1" | "excession.kbuild-graph.v1"
    ) {
        return refuse("unsupported configured graph producer");
    }
    let nodes = graph.get("nodes")?.array_ref()?;
    let edges = graph.get("edges")?.array_ref()?;
    if nodes.len() > MAX_GRAPH_NODES || edges.len() > MAX_GRAPH_EDGES {
        return refuse("configured graph exceeds integrated-query budget");
    }
    let stored_nodes = db.query(
        "SELECT id,document FROM configured_entity WHERE graph_id=? ORDER BY id",
        &[Param::Text(graph_id)],
    )?;
    let stored_edges = db.query(
        "SELECT id,kind,source_id,destination_id,status,evidence FROM configured_edge WHERE graph_id=? ORDER BY id",
        &[Param::Text(graph_id)],
    )?;
    if stored_nodes.len() != nodes.len() || stored_edges.len() != edges.len() {
        return refuse("configured graph relational membership differs from immutable body");
    }
    let mut by_id = BTreeMap::new();
    let mut source_ids = BTreeMap::new();
    for node in nodes {
        let id = node.get("id")?.str()?.to_owned();
        if by_id.insert(id.clone(), node).is_some() {
            return refuse("duplicate configured entity identity");
        }
        if node.get("role")?.str()? != "source" {
            continue;
        }
        let payload = node.get("payload")?;
        let blob = match node.get("kind")?.str()? {
            "git-source" => Some(payload.get("blob")?.str()?),
            "kbuild-source" => match payload.get("git_blob")? {
                Value::Null => None,
                other => Some(other.str()?),
            },
            _ => None,
        };
        if let Some(blob) = blob {
            let path = payload.get("path")?.str()?;
            let authenticated = source(repo, commit, tree, path, blob)?;
            out.source_paths.insert(path.to_owned());
            source_ids.insert(id, authenticated);
        }
    }
    for (index, (id, node)) in by_id.iter().enumerate() {
        let row = &stored_nodes[index];
        if row[0].as_deref() != Some(id.as_str())
            || row[1].as_deref() != Some(node.canonical().as_str())
        {
            return refuse("configured entity SQL rows differ from graph body");
        }
    }
    let mut edge_map = BTreeMap::new();
    for edge in edges {
        let id = edge.sha256();
        if edge_map.insert(id, edge).is_some() {
            return refuse("duplicate configured graph edge");
        }
    }
    for (index, (id, edge)) in edge_map.iter().enumerate() {
        let row = &stored_edges[index];
        if row[0].as_deref() != Some(id.as_str())
            || row[1].as_deref() != Some(edge.get("kind")?.str()?)
            || row[2].as_deref() != Some(edge.get("from")?.str()?)
            || row[3].as_deref() != Some(edge.get("to")?.str()?)
            || row[4].as_deref() != Some(edge.get("status")?.str()?)
            || row[5].as_deref() != Some(edge.get("evidence")?.str()?)
        {
            return refuse("configured relationship SQL rows differ from graph body");
        }
        let from = edge.get("from")?.str()?;
        let to = edge.get("to")?.str()?;
        if !by_id.contains_key(from) || !by_id.contains_key(to) {
            return refuse("configured graph edge has missing node");
        }
        for (source_id, other_id, direction) in [
            (from, to, "SOURCE_TO_ENTITY"),
            (to, from, "ENTITY_TO_SOURCE"),
        ] {
            let Some(authenticated) = source_ids.get(source_id) else {
                continue;
            };
            out.configured_observations.push(obj(&[
                ("graph_id", Value::string(graph_id)),
                ("source", authenticated.clone()),
                ("entity", Value::string(other_id)),
                ("entity_role", by_id[other_id].get("role")?.clone()),
                ("direction", Value::string(direction)),
                ("relation", edge.get("relation")?.clone()),
                ("native_kind", edge.get("kind")?.clone()),
                ("status", edge.get("status")?.clone()),
                ("evidence", edge.get("evidence")?.clone()),
            ]));
        }
    }
    out.coverage.push(obj(&[
        ("producer", Value::string(graph.get("schema")?.str()?)),
        ("graph_id", Value::string(graph_id)),
        ("status", graph.get("status")?.clone()),
        (
            "configuration",
            match graph.entries()?.get("configuration") {
                Some(value) => value.clone(),
                None => graph.get("config_sha256")?.clone(),
            },
        ),
        ("source_count", Value::number(source_ids.len())),
        ("limitations", graph.get("limitations")?.clone()),
    ]));
    Ok(())
}

/// Compose selected evidence in a consistent read transaction; selected
/// producer receipts must replay against exactly the requested Git revision.
pub fn read(db: &Db, repo: &Path, revision: &str, selection: Selection<'_>) -> Result<Observed> {
    if selection.source_prefix.is_some() && selection.scip_snapshot.is_none() {
        return refuse("--source-prefix requires --scip-snapshot");
    }
    if selection.is_empty() {
        return Ok(Observed::default());
    }
    let commit = git::rev(repo, revision)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    db.read_transaction(|db| {
        let mut out = Observed::default();
        if let Some(snapshot) = selection.scip_snapshot {
            select_scip(
                db,
                repo,
                &commit,
                &tree,
                snapshot,
                selection.source_prefix,
                &mut out,
            )?;
        }
        if let Some(id) = selection.graph_id {
            select_graph(db, repo, &commit, &tree, id, &mut out)?;
        }
        Ok(out)
    })
}

const MAX_DISCOVERY_ROWS: usize = 128;

fn has_table(db: &Db, name: &str) -> Result<bool> {
    Ok(!db
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            &[Param::Text(name)],
        )?
        .is_empty())
}

/// Recover the source prefix from the same SCIP project_root declaration
/// already authenticated by scip_refactor during native evidence ingestion.
/// Absence is allowed only for root-scoped indexes; no project root is guessed.
fn scip_prefix(repo: &Path, raw: &str) -> Result<Option<String>> {
    let parsed = json::parse(raw.as_bytes())?;
    let meta = parsed.get("metadata")?;
    let project_root = meta
        .entries()?
        .get("project_root")
        .or_else(|| meta.entries().ok()?.get("projectRoot"));
    let Some(root) = project_root else {
        return Ok(None);
    };
    let root = root.str()?;
    let physical = root
        .strip_prefix("file://")
        .ok_or("SCIP project root is not a file URI")?;
    let canonical = repo
        .canonicalize()
        .map_err(|e| format!("Git source root: {e}"))?;
    let prefix = Path::new(physical)
        .strip_prefix(&canonical)
        .map_err(|_| "SCIP project root is outside selected Git checkout")?;
    if prefix.as_os_str().is_empty() {
        return Ok(None);
    }
    let path = prefix.to_str().ok_or("SCIP source prefix is not UTF-8")?;
    git::validate_path(path)?;
    Ok(Some(path.to_owned()))
}

/// Enumerate *all* relevant snapshots for a pinned revision. Matching a
/// commit is only a scope filter; chosen evidence is reauthenticated before
/// use. No ranking by recency, filename, producer popularity or configuration.
pub fn discover(db: &Db, repo: &Path, revision: &str) -> Result<Value> {
    let commit = git::rev(repo, revision)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    db.read_transaction(|db| {
        let mut scip = Vec::new();
        let mut graphs = Vec::new();
        if has_table(db, "evidence_snapshot_v1")? {
            let rows = db.query(
                "SELECT snapshot_id FROM evidence_snapshot_v1 WHERE source_commit=? ORDER BY snapshot_id LIMIT 129",
                &[Param::Text(&commit)],
            )?;
            if rows.len() > MAX_DISCOVERY_ROWS {
                return refuse("more than 128 evidence snapshots for revision; narrow the database");
            }
            for row in rows {
                let snapshot = row[0].as_deref().ok_or("snapshot ID missing")?;
                digest_id(snapshot)?;
                let receipts = db.query(
                    "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=? ORDER BY receipt_id LIMIT 2",
                    &[Param::Text(snapshot)],
                )?;
                if receipts.len() != 1 {
                    continue;
                }
                let receipt = json::parse(
                    receipts[0][0].as_deref().ok_or("native receipt missing")?.as_bytes(),
                )?;
                let producer = receipt.get("producer")?.str()?;
                if !producer.starts_with("scip:") {
                    continue;
                }
                // Verify the immutable manifest and its exact receipt/observation
                // membership before offering this snapshot as a candidate.
                crate::evidence_store::audit(db, snapshot)?;
                let prefix = scip_prefix(repo, receipt.get("raw_json")?.str()?)?;
                scip.push(obj(&[
                    ("snapshot_id", Value::string(snapshot)),
                    ("producer", Value::string(producer)),
                    ("version", receipt.get("version")?.clone()),
                    ("source_prefix", prefix.as_deref().map_or(Value::Null, Value::string)),
                    ("source_authentication", Value::string("REPLAY_REQUIRED_ON_SELECTION")),
                ]));
            }
        }
        if has_table(db, "configured_graph")? {
            let rows = db.query(
                "SELECT graph_id,document FROM configured_graph WHERE source_commit=? ORDER BY graph_id LIMIT 129",
                &[Param::Text(&commit)],
            )?;
            if rows.len() > MAX_DISCOVERY_ROWS {
                return refuse("more than 128 configured graphs for revision; narrow the database");
            }
            for row in rows {
                let id = row[0].as_deref().ok_or("configured graph ID missing")?;
                digest_id(id)?;
                let body = json::parse(
                    row[1].as_deref().ok_or("configured graph body missing")?.as_bytes(),
                )?;
                if body.sha256() != id
                    || body.get("source_tree")?.str()? != tree
                    || body.get("source_commit")?.str()? != commit
                {
                    return refuse("configured graph discovery found invalid Git-bound identity");
                }
                let schema = body.get("schema")?.str()?;
                if !matches!(schema, "excession.configured-graph.v1" | "excession.kbuild-graph.v1") {
                    return refuse("unrecognized configured graph producer");
                }
                let (target_key, configuration_key) = if schema == "excession.configured-graph.v1" {
                    ("target", "configuration")
                } else {
                    ("archive", "config_sha256")
                };
                graphs.push(obj(&[
                    ("graph_id", Value::string(id)),
                    ("producer", Value::string(schema)),
                    ("target", body.get(target_key)?.clone()),
                    ("configuration", body.get(configuration_key)?.clone()),
                    ("source_authentication", Value::string("REPLAY_REQUIRED_ON_SELECTION")),
                ]));
            }
        }
        let ambiguous = scip.len() > 1 || graphs.len() > 1;
        let chosen_scip = if scip.len() == 1 {
            scip[0].get("snapshot_id")?.clone()
        } else {
            Value::Null
        };
        let chosen_prefix = if scip.len() == 1 {
            scip[0].get("source_prefix")?.clone()
        } else {
            Value::Null
        };
        let chosen_graph = if graphs.len() == 1 {
            graphs[0].get("graph_id")?.clone()
        } else {
            Value::Null
        };
        let result = obj(&[
            ("schema", Value::string("excession.producer-discovery.v1")),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("status", Value::string(if ambiguous { "AMBIGUOUS" }
                else if scip.is_empty() && graphs.is_empty() { "NO_IMPORTED_EVIDENCE" }
                else { "UNAMBIGUOUS" })),
            ("scip_candidates", Value::array(scip)),
            ("graph_candidates", Value::array(graphs)),
            ("selection", obj(&[
                ("scip_snapshot", chosen_scip),
                ("graph_id", chosen_graph),
                ("source_prefix", chosen_prefix),
            ])),
            ("selection_policy", Value::string("UNIQUE_ID_HINTS_WITH_CONFLICT_ISOLATED_COMPOSITION")),
            ("source_authentication", Value::string("REQUIRES_GIT_REPLAY_ON_USE")),
            ("complete_semantics", Value::Bool(false)),
        ]);
        Ok(result)
    })
}


const MAX_PLAN_ITEMS: usize = 24;
const MAX_COMPOSED_EDGES: usize = 20_000;

/// One independently sourced SCIP index. The prefix is declared by the
/// producer, never inferred from a file suffix or checkout convention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScipInput {
    pub snapshot_id: String,
    pub source_prefix: Option<String>,
}

#[derive(Clone, Default)]
pub struct Plan {
    pub scip: Vec<ScipInput>,
    pub graphs: Vec<String>,
    pub blocked_paths: BTreeSet<String>,
    pub gaps: Vec<Value>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.scip.is_empty() && self.graphs.is_empty()
    }

    pub fn document(&self) -> Value {
        obj(&[
            ("scip", Value::array(self.scip.iter().map(|x| obj(&[
                ("snapshot_id", Value::string(&x.snapshot_id)),
                ("source_prefix", x.source_prefix.as_deref().map_or(Value::Null, Value::string)),
            ])))),
            ("graphs", Value::array(self.graphs.iter().map(Value::string))),
            ("blocked_paths", Value::array(self.blocked_paths.iter().map(Value::string))),
            ("gaps", Value::array(self.gaps.clone())),
        ])
    }
}

/// Construct a pinned plan from supplied identities, recomputing the precise
/// conflicting source set from independently retained native SCIP documents.
/// Overlapping sources are quarantined from impact traversal, while unrelated
/// source references and separately scoped build membership can still be used.
fn plan_from_ids(
    db: &Db,
    repo: &Path,
    revision: &str,
    scip: Vec<ScipInput>,
    graphs: Vec<String>,
) -> Result<Plan> {
    if scip.len() > MAX_PLAN_ITEMS || graphs.len() > MAX_PLAN_ITEMS {
        return refuse("more than 24 source evidence producers for one revision");
    }
    let commit = git::rev(repo, revision)?;
    let mut seen_scip = BTreeSet::new();
    let mut seen_graphs = BTreeSet::new();
    let mut by_path = BTreeMap::<String, BTreeSet<String>>::new();
    for choice in &scip {
        digest_id(&choice.snapshot_id)?;
        if !seen_scip.insert(choice.snapshot_id.clone()) {
            return refuse("duplicate SCIP snapshot identity in source evidence plan");
        }
        if let Some(prefix) = &choice.source_prefix {
            git::validate_path(prefix)?;
        }
        let manifest = crate::evidence_store::audit(db, &choice.snapshot_id)?;
        if manifest.get("source_commit")?.str()? != commit {
            return refuse("SCIP plan references evidence from a different Git revision");
        }
        let rows = db.query(
            "SELECT document FROM evidence_receipt_v1 WHERE snapshot_id=?",
            &[Param::Text(&choice.snapshot_id)],
        )?;
        if rows.len() != 1 {
            return refuse("SCIP plan needs exactly one retained native receipt");
        }
        let receipt = json::parse(
            rows[0][0].as_deref().ok_or("missing native receipt")?.as_bytes(),
        )?;
        if !receipt.get("producer")?.str()?.starts_with("scip:") {
            return refuse("non-SCIP receipt was supplied as a SCIP source index");
        }
        let raw = receipt.get("raw_json")?.str()?;
        if scip_prefix(repo, raw)? != choice.source_prefix {
            return refuse("SCIP source prefix differs from its producer declaration");
        }
        let parsed = json::parse(raw.as_bytes())?;
        let documents = parsed.get("documents")?.array_ref()?;
        for item in documents {
            let relative = item.entries()?.get("relative_path")
                .or_else(|| item.entries().ok()?.get("relativePath"))
                .ok_or("SCIP document lacks a source path")?.str()?;
            git::validate_path(relative)?;
            let path = choice.source_prefix.as_ref().map_or_else(
                || relative.to_owned(),
                |prefix| format!("{prefix}/{relative}"),
            );
            git::validate_path(&path)?;
            by_path.entry(path).or_default().insert(choice.snapshot_id.clone());
        }
    }
    let mut blocked_paths = BTreeSet::new();
    let mut gaps = Vec::new();
    for (path, index_ids) in &by_path {
        if index_ids.len() > 1 {
            blocked_paths.insert(path.to_owned());
            gaps.push(obj(&[
                ("reason", Value::string("UNRESOLVED_RELATIONSHIP")),
                ("path", Value::string(path)),
                ("detail", Value::string(
                    "Overlapping native SCIP indexes; paths touching this source are excluded from composed impact traversal",
                )),
            ]));
        }
    }
    let mut by_target = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for id in &graphs {
        digest_id(id)?;
        if !seen_graphs.insert(id.clone()) {
            return refuse("duplicate configured graph identity in source evidence plan");
        }
        let rows = db.query(
            "SELECT document FROM configured_graph WHERE graph_id=?",
            &[Param::Text(id)],
        )?;
        if rows.len() != 1 {
            return refuse("configured graph plan identity is not in SQLite");
        }
        let document = json::parse(
            rows[0][0].as_deref().ok_or("configured graph document missing")?.as_bytes(),
        )?;
        if document.sha256() != *id || document.get("source_commit")?.str()? != commit {
            return refuse("configured graph plan identity or Git revision differs");
        }
        let schema = document.get("schema")?.str()?;
        let target = match schema {
            "excession.configured-graph.v1" => document.get("target")?.str()?,
            "excession.kbuild-graph.v1" => document.get("archive")?.str()?,
            _ => return refuse("unsupported configured graph producer in plan"),
        };
        by_target.entry((schema.to_owned(), target.to_owned()))
            .or_default().insert(id.to_owned());
    }
    for ((producer, target), ids) in by_target {
        if ids.len() > 1 {
            gaps.push(obj(&[
                ("reason", Value::string("MISSING_CONFIGURATION")),
                ("detail", Value::string(format!(
                    "Multiple {producer} snapshots of target {} retain separate configuration-scoped observations",
                    target.chars().take(180).collect::<String>()
                ))),
            ]));
        }
    }
    if gaps.len() > 9999 {
        return refuse("native evidence conflicts exceed coverage budget");
    }
    Ok(Plan { scip, graphs, blocked_paths, gaps })
}

/// Discover every source-compatible index and every independently labeled
/// configured graph. Only colliding SCIP source paths are excluded; build
/// configurations remain distinct rather than being silently unioned.
pub fn default_plan(db: &Db, repo: &Path, revision: &str) -> Result<Plan> {
    let candidates = discover(db, repo, revision)?;
    let scip = candidates.get("scip_candidates")?.array_ref()?.iter()
        .map(|entry| {
            let source_prefix = match entry.get("source_prefix")? {
                Value::Null => None,
                value => Some(value.str()?.to_owned()),
            };
            Ok(ScipInput {
                snapshot_id: entry.get("snapshot_id")?.str()?.to_owned(),
                source_prefix,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let graphs = candidates.get("graph_candidates")?.array_ref()?.iter()
        .map(|entry| Ok(entry.get("graph_id")?.str()?.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    plan_from_ids(db, repo, revision, scip, graphs)
}

/// Replay only the identities sealed in a saved report. Newly imported
/// indexes or graphs must not silently change an existing impact claim.
pub fn pinned_plan(db: &Db, repo: &Path, revision: &str, record: &Value) -> Result<Plan> {
    let scip = record.get("scip")?.array_ref()?.iter()
        .map(|entry| {
            let source_prefix = match entry.get("source_prefix")? {
                Value::Null => None,
                value => Some(value.str()?.to_owned()),
            };
            Ok(ScipInput {
                snapshot_id: entry.get("snapshot_id")?.str()?.to_owned(),
                source_prefix,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let graphs = record.get("graphs")?.array_ref()?.iter()
        .map(|item| Ok(item.str()?.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    let reconstructed = plan_from_ids(db, repo, revision, scip, graphs)?;
    if reconstructed.document() != *record {
        return refuse("recorded native evidence conflicts or selection identities differ on replay");
    }
    Ok(reconstructed)
}

/// Reauthenticate all selected sources and build graphs. Cross-index overlap
/// is quarantined by source path before any impact traversal sees it.
pub fn read_plan(db: &Db, repo: &Path, revision: &str, plan: &Plan) -> Result<Observed> {
    let commit = git::rev(repo, revision)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    db.read_transaction(|db| {
        let mut out = Observed::default();
        for choice in &plan.scip {
            let mut native = Observed::default();
            select_scip(
                db, repo, &commit, &tree, &choice.snapshot_id,
                choice.source_prefix.as_deref(), &mut native,
            )?;
            for relation in native.relationships {
                let from = relation.get("source")?.get("path")?.str()?;
                let to = relation.get("target")?.get("path")?.str()?;
                if !plan.blocked_paths.contains(from) && !plan.blocked_paths.contains(to) {
                    out.relationships.push(relation);
                }
            }
            out.receipt_ids.extend(native.receipt_ids);
            out.source_paths.extend(native.source_paths);
            out.coverage.extend(native.coverage);
        }
        for id in &plan.graphs {
            select_graph(db, repo, &commit, &tree, id, &mut out)?;
        }
        if out.relationships.len() > MAX_COMPOSED_EDGES
            || out.configured_observations.len() > MAX_COMPOSED_EDGES
        {
            return refuse("composed native source observations exceed evidence budget");
        }
        Ok(out)
    })
}
