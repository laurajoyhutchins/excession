//! Shared immutable configured-graph storage. Sources of evidence own their semantics.
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeMap;
const MAX_NODES: usize = 30_000;
const MAX_EDGES: usize = 80_000;
/// Cross-language semantic vocabulary. Native kinds are preserved verbatim,
/// including unknown kinds, so a new adapter never silently implies a stronger
/// relationship than its evidence establishes.
pub fn role(kind: &str) -> &'static str {
    match kind {
        "configured-target" => "target",
        "configured-artifact" | "kbuild-archive" | "kbuild-object" => "artifact",
        "git-source" | "kbuild-source" => "source",
        "compiler-translation-unit" => "analysis",
        "tu-declaration" => "symbol",
        "tu-call-site" => "reference",
        "observed-input" => "input",
        "kbuild-command-record" => "observation",
        "kbuild-compile-action" => "action",
        "kbuild-config" => "configuration",
        _ => "unknown",
    }
}
pub fn relation(kind: &str) -> &'static str {
    match kind {
        "DEPENDS_ON_TARGET" => "depends_on",
        "TARGET_SOURCE" => "uses",
        "PRODUCES_ARTIFACT" => "produces",
        "OBSERVED_AS_TU" => "analyzed_as",
        "PREPROCESSOR_INPUT" | "COMPILES_SOURCE" => "reads",
        "DECLARES" => "declares",
        "CONTAINS_CALL" | "CONTAINS_OBJECT" => "contains",
        "MAY_REFER_TO" => "may_refer_to",
        "CONFIGURATION_SNAPSHOT" => "observed_with",
        "HAS_BUILD_RECORD" | "RECORDS_COMPILER_COMMAND" => "has_evidence",
        _ => "unknown",
    }
}
pub fn push_node(
    nodes: &mut BTreeMap<String, Value>,
    id: String,
    kind: &str,
    payload: Value,
) -> Result<()> {
    push_typed_node(nodes, id, role(kind), kind, payload)
}

/// Direct neutral contract for future adapters, without adding a
/// language- or build-system-specific mapping to this module.
pub fn push_typed_node(
    nodes: &mut BTreeMap<String, Value>,
    id: String,
    neutral_role: &str,
    kind: &str,
    payload: Value,
) -> Result<()> {
    if !matches!(
        neutral_role,
        "target"
            | "artifact"
            | "source"
            | "analysis"
            | "symbol"
            | "reference"
            | "input"
            | "observation"
            | "action"
            | "configuration"
            | "unknown"
    ) {
        return refuse("unrecognized neutral node role");
    }
    if kind.is_empty() || id.is_empty() {
        return refuse("graph entity identity and native kind must be nonempty");
    }
    if nodes.len() >= MAX_NODES && !nodes.contains_key(&id) {
        return refuse("configured graph node budget exceeded");
    }
    let item = obj(&[
        ("id", Value::string(&id)),
        ("kind", Value::string(kind)),
        ("role", Value::string(neutral_role)),
        ("payload", payload),
    ]);
    if let Some(existing) = nodes.insert(id, item.clone())
        && existing != item
    {
        return refuse("conflicting entity identity");
    }
    Ok(())
}
pub fn push_edge(
    edges: &mut BTreeMap<String, Value>,
    from: &str,
    to: &str,
    kind: &str,
    status: &str,
    evidence: &str,
) -> Result<()> {
    push_typed_edge(edges, from, to, relation(kind), kind, status, evidence)
}

/// Language-neutral relationship contract. Original evidence is mandatory.
pub fn push_typed_edge(
    edges: &mut BTreeMap<String, Value>,
    from: &str,
    to: &str,
    neutral_relation: &str,
    kind: &str,
    status: &str,
    evidence: &str,
) -> Result<()> {
    if !matches!(
        neutral_relation,
        "depends_on"
            | "uses"
            | "produces"
            | "analyzed_as"
            | "reads"
            | "declares"
            | "contains"
            | "may_refer_to"
            | "observed_with"
            | "has_evidence"
            | "unknown"
    ) {
        return refuse("unrecognized neutral relationship");
    }
    if from.is_empty()
        || to.is_empty()
        || kind.is_empty()
        || status.is_empty()
        || evidence.is_empty()
    {
        return refuse("graph relationship needs identities, native kind, status, and evidence");
    }
    if edges.len() >= MAX_EDGES {
        return refuse("configured graph edge budget exceeded");
    }
    let item = obj(&[
        ("from", Value::string(from)),
        ("to", Value::string(to)),
        ("kind", Value::string(kind)),
        ("relation", Value::string(neutral_relation)),
        ("status", Value::string(status)),
        ("evidence", Value::string(evidence)),
    ]);
    edges.insert(item.sha256(), item);
    Ok(())
}
pub struct GraphSnapshot<'a> {
    pub db: &'a Db,
    pub id: &'a str,
    pub commit: &'a str,
    pub target: &'a str,
    pub configuration: &'a str,
    pub graph: &'a Value,
    pub nodes: &'a BTreeMap<String, Value>,
    pub edges: &'a BTreeMap<String, Value>,
    pub observations: &'a [(String, String, Value)],
}

pub fn persist(snapshot: GraphSnapshot<'_>) -> Result<()> {
    let GraphSnapshot {
        db, id, commit, target, configuration, graph, nodes, edges, observations,
    } = snapshot;
    if graph.sha256() != id {
        return refuse("graph ID differs from canonical body");
    }
    for edge in edges.values() {
        if !nodes.contains_key(edge.get("from")?.str()?)
            || !nodes.contains_key(edge.get("to")?.str()?)
        {
            return refuse("configured graph edge references an absent entity");
        }
    }
    db.exec(
        "CREATE TABLE IF NOT EXISTS configured_graph(\
        graph_id TEXT PRIMARY KEY, source_commit TEXT NOT NULL,\
        target TEXT NOT NULL, configuration TEXT NOT NULL, document TEXT NOT NULL);\
        CREATE TABLE IF NOT EXISTS configured_observation(\
        graph_id TEXT NOT NULL, report_id TEXT NOT NULL, source_path TEXT NOT NULL,\
        document TEXT NOT NULL, PRIMARY KEY(graph_id,report_id),\
        FOREIGN KEY(graph_id) REFERENCES configured_graph(graph_id));\
        CREATE TABLE IF NOT EXISTS configured_entity(\
        graph_id TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL, document TEXT NOT NULL,\
        PRIMARY KEY(graph_id,id), FOREIGN KEY(graph_id) REFERENCES configured_graph(graph_id));\
        CREATE TABLE IF NOT EXISTS configured_edge(\
        graph_id TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL,\
        source_id TEXT NOT NULL, destination_id TEXT NOT NULL, status TEXT NOT NULL,\
        evidence TEXT NOT NULL, relation TEXT NOT NULL DEFAULT '__legacy__',\
        PRIMARY KEY(graph_id,id),\
        FOREIGN KEY(graph_id,source_id) REFERENCES configured_entity(graph_id,id),\
        FOREIGN KEY(graph_id,destination_id) REFERENCES configured_entity(graph_id,id));",
    )?;
    // v1 graph databases have no neutral relation column. Add it without
    // changing immutable graph bodies or graph IDs.
    let columns = db.query("PRAGMA table_info(configured_edge)", &[])?;
    if !columns
        .iter()
        .any(|row| row.get(1).and_then(|value| value.as_deref()) == Some("relation"))
    {
        db.exec(
            "ALTER TABLE configured_edge ADD COLUMN relation TEXT NOT NULL DEFAULT '__legacy__'",
        )?;
    }
    db.transaction(|db| {
        db.execute(
            "INSERT OR IGNORE INTO configured_graph VALUES(?,?,?,?,?)",
            &[
                Param::Text(id),
                Param::Text(commit),
                Param::Text(target),
                Param::Text(configuration),
                Param::Text(&graph.canonical()),
            ],
        )?;
        for (source, receipt_id, receipt) in observations {
            db.execute(
                "INSERT OR IGNORE INTO configured_observation VALUES(?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(receipt_id),
                    Param::Text(source),
                    Param::Text(&receipt.canonical()),
                ],
            )?;
        }
        for (node_id, node) in nodes {
            db.execute(
                "INSERT OR IGNORE INTO configured_entity VALUES(?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(node_id),
                    Param::Text(node.get("kind")?.str()?),
                    Param::Text(&node.canonical()),
                ],
            )?;
        }
        for (edge_id, edge) in edges {
            db.execute(
                "INSERT OR IGNORE INTO configured_edge(graph_id,id,kind,source_id,destination_id,status,evidence,relation) VALUES(?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text(edge_id),
                    Param::Text(edge.get("kind")?.str()?),
                    Param::Text(edge.get("from")?.str()?),
                    Param::Text(edge.get("to")?.str()?),
                    Param::Text(edge.get("status")?.str()?),
                    Param::Text(edge.get("evidence")?.str()?),
                    Param::Text(edge.get("relation")?.str()?),
                ],
            )?;
        }
        Ok(())
    })
}
