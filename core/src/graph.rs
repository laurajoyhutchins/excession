//! Language-neutral graph import, inspection and evidence-preserving queries.
//! Build systems and languages are adapters. This layer never parses source.
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, configured_store, graph_assembly, kbuild_graph, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

const MAX_GRAPHS: usize = 1024;
const MAX_DEPTH: usize = 8;
const MAX_QUERY_EDGES: usize = 256;

pub struct Import<'a> {
    pub repo: &'a Path,
    pub revision: &'a str,
    pub build: &'a Path,
    pub target: &'a str,
    pub provider: &'a str,
    pub observations: Option<&'a Path>,
    pub trusted_local: &'a str,
    pub archiver: &'a str,
}

pub fn database_path(db: Option<&Path>, build: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = db {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = build {
        return Ok(path.join("excession.sqlite"));
    }
    refuse("provide --db SQLITE or --build BUILD (defaults to BUILD/excession.sqlite)")
}

fn provider_for(build: &Path, target: &str, requested: &str) -> Result<&'static str> {
    match requested {
        "cmake" => Ok("cmake"),
        "kbuild" => Ok("kbuild"),
        "auto" => {
            let cmake = build.join(".cmake/api/v1/reply").is_dir();
            let kbuild = build.join(".config").is_file() && target.ends_with(".a");
            match (cmake, kbuild) {
                (true, false) => Ok("cmake"),
                (false, true) => Ok("kbuild"),
                (false, false) => refuse(
                    "build provider not detected: expected CMake File API reply or a Kbuild .config and .a target; use --provider"
                ),
                (true, true) => refuse(
                    "ambiguous build provider; specify --provider cmake|kbuild"
                ),
            }
        }
        _ => refuse("unknown build provider: choose auto, cmake, or kbuild"),
    }
}
pub fn import(db: &Db, input: &Import<'_>) -> Result<Value> {
    if input.target.is_empty() { return refuse("select a nonempty configured build target"); }
    let provider = provider_for(input.build, input.target, input.provider)?;
    let result = match provider {
        "cmake" => graph_assembly::assemble(input.repo, input.revision,
            input.build, input.target, input.observations, db)?,
        "kbuild" => {
            if input.observations.is_some() {
                return refuse("--observations applies to the cmake adapter, not Kbuild");
            }
            kbuild_graph::assemble(input.repo, input.revision, input.build,
                input.target, input.archiver, input.trusted_local, db)?
        }
        _ => return refuse("unsupported configured build provider"),
    };
    let id = result.get("graph_id")?.str()?;
    // Small interactive response. The complete raw adapter report is stored
    // immutably in the graph table and queryable without console flooding.
    let mut summary = summarize(&result, id)?;
    summary.entries_mut()?.insert("provider".to_owned(), Value::string(provider));
    Ok(summary)
}

fn optional<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.entries().ok()?.get(key)
}
fn provider_name(body: &Value) -> &'static str {
    match body.get("schema").and_then(Value::str).ok() {
        Some("excession.configured-graph.v1") => "cmake",
        Some("excession.kbuild-graph.v1") => "kbuild",
        _ => "unknown",
    }
}
fn summarize(body: &Value, id: &str) -> Result<Value> {
    let nodes = body.get("nodes")?.array_ref()?;
    let edges = body.get("edges")?.array_ref()?;
    let target = optional(body, "target").or_else(|| optional(body, "archive"))
        .ok_or("graph target missing")?.str()?;
    let status = body.get("status")?.str()?;
    let config = optional(body, "configuration").or_else(|| optional(body, "config_sha256"))
        .ok_or("graph configuration missing")?.str()?;
    Ok(obj(&[
        ("schema", Value::string("excession.graph-summary.v1")),
        ("graph_id", Value::string(id)),
        ("source_commit", body.get("source_commit")?.clone()),
        ("target", Value::string(target)),
        ("configuration", Value::string(config)),
        ("provider", Value::string(provider_name(body))),
        ("status", Value::string(status)),
        ("node_count", Value::number(nodes.len())),
        ("edge_count", Value::number(edges.len())),
        ("coverage_note", Value::string(
            "Configured observations only; not proof of build execution, global symbols, or runtime reachability"
        )),
    ]))
}
fn graphs(db: &Db) -> Result<Vec<(String, String, String)>> {
    let rows = db.query(
        "SELECT graph_id, target, document FROM configured_graph ORDER BY target,graph_id",
        &[]
    )?;
    if rows.len() > MAX_GRAPHS { return refuse("more than 1024 graph snapshots; select a smaller database"); }
    rows.into_iter().map(|row| {
        let id = row[0].clone().ok_or("missing graph id")?;
        let target = row[1].clone().ok_or("missing graph target")?;
        let document = row[2].clone().ok_or("missing graph body")?;
        Ok((id, target, document))
    }).collect()
}
fn find(db: &Db, graph: Option<&str>, target: Option<&str>) -> Result<(String, Value)> {
    let mut choices = Vec::new();
    for (id, name, text) in graphs(db)? {
        if graph.is_none_or(|needle| id == needle || id.starts_with(needle))
            && target.is_none_or(|needle| name == needle) {
            choices.push((id, text));
        }
    }
    if choices.len() != 1 {
        return refuse(format!(
            "expected one graph, found {}; use 'excession graph ls' then --id GRAPH_ID or --target TARGET",
            choices.len()
        ));
    }
    let (id, text) = choices.pop().ok_or("graph snapshot missing")?;
    let parsed = crate::json::parse(text.as_bytes())?;
    if parsed.sha256() != id { return refuse("stored graph body digest mismatch"); }
    Ok((id, parsed))
}
pub fn list(db: &Db) -> Result<Value> {
    let entries = graphs(db)?.into_iter().map(|(id, _, body)| {
        let parsed = crate::json::parse(body.as_bytes())?;
        if parsed.sha256() != id { return refuse("stored graph body digest mismatch"); }
        summarize(&parsed, &id)
    }).collect::<Result<Vec<_>>>()?;
    Ok(obj(&[
        ("schema", Value::string("excession.graph-list.v1")),
        ("count", Value::number(entries.len())),
        ("graphs", Value::array(entries)),
    ]))
}
pub fn show(db: &Db, id: Option<&str>, target: Option<&str>) -> Result<Value> {
    let (id, body) = find(db, id, target)?;
    summarize(&body, &id)
}
fn resolve_node(db: &Db, graph: &str, name: &str) -> Result<String> {
    let rows = db.query("SELECT id FROM configured_entity WHERE graph_id=? ORDER BY id",
        &[Param::Text(graph)])?;
    let names = rows.into_iter().filter_map(|r| r[0].clone()).collect::<Vec<_>>();
    if names.iter().any(|id| id == name) { return Ok(name.to_owned()); }
    let candidates = names.iter()
        .filter(|id| id.rsplit_once(':').is_some_and(|(_, tail)| tail == name)
            || id.strip_prefix("source:").is_some_and(|tail| tail == name)
            || id.strip_prefix("target:").is_some_and(|tail| tail == name)
            || id.strip_prefix("archive:").is_some_and(|tail| tail == name))
        .collect::<Vec<_>>();
    if candidates.len() == 1 { return Ok((*candidates[0]).clone()); }
    refuse(format!(
        "node {name:?} has {} matches; use the exact ID from the graph or a unique suffix",
        candidates.len()
    ))
}
#[derive(Clone)]
struct Arc {
    from: String, to: String, kind: String, status: String, evidence: String,
}
fn arcs(db: &Db, graph: &str) -> Result<Vec<Arc>> {
    let rows = db.query(
        "SELECT source_id,destination_id,kind,status,evidence FROM configured_edge WHERE graph_id=? ORDER BY id",
        &[Param::Text(graph)]
    )?;
    rows.into_iter().map(|r| Ok(Arc {
        from: r[0].clone().ok_or("missing relationship source")?,
        to: r[1].clone().ok_or("missing relationship destination")?,
        kind: r[2].clone().ok_or("missing relationship kind")?,
        status: r[3].clone().ok_or("missing relationship status")?,
        evidence: r[4].clone().ok_or("missing relationship evidence")?,
    })).collect()
}
fn arc_value(arc: &Arc) -> Value {
    obj(&[
        ("from", Value::string(&arc.from)), ("to", Value::string(&arc.to)),
        ("kind", Value::string(&arc.kind)),
        ("relation", Value::string(configured_store::relation(&arc.kind))),
        ("status", Value::string(&arc.status)),
        ("evidence", Value::string(&arc.evidence)),
    ])
}
fn node_value(db: &Db, graph: &str, name: &str) -> Result<Value> {
    let rows = db.query(
        "SELECT document FROM configured_entity WHERE graph_id=? AND id=?",
        &[Param::Text(graph), Param::Text(name)]
    )?;
    if rows.len() != 1 { return refuse("graph node missing"); }
    let encoded = rows[0][0].as_deref().ok_or("missing graph node document")?;
    let mut node = crate::json::parse(encoded.as_bytes())?;
    if optional(&node, "role").is_none() {
        let kind = node.get("kind")?.str()?.to_owned();
        node.entries_mut()?.insert("role".to_owned(),
            Value::string(configured_store::role(&kind)));
    }
    Ok(node)
}
fn presentation(db: &Db, graph: &str, nodes: &BTreeSet<String>, edges: &[Arc],
                status: &str) -> Result<Value> {
    let mut facts = Vec::new();
    for name in nodes { facts.push(node_value(db, graph, name)?); }
    Ok(obj(&[
        ("schema", Value::string("excession.graph-query.v1")),
        ("graph_id", Value::string(graph)),
        ("status", Value::string(status)),
        ("nodes", Value::array(facts)),
        ("edges", Value::array(edges.iter().map(arc_value))),
        ("semantics", Value::string(
            "Evidence relationships are not automatically runtime reachability or proven causality"
        )),
    ]))
}
pub fn neighbors(db: &Db, graph_id: Option<&str>, target: Option<&str>,
                 node: &str, direction: &str, depth: usize) -> Result<Value> {
    if depth == 0 || depth > MAX_DEPTH { return refuse("graph depth must be 1..8"); }
    if !matches!(direction, "out" | "in" | "both") {
        return refuse("graph direction must be out, in, or both");
    }
    let (id, _) = find(db, graph_id, target)?;
    let start = resolve_node(db, &id, node)?;
    let all = arcs(db, &id)?;
    let mut visited = BTreeSet::from([start.clone()]);
    let mut queue = VecDeque::from([(start, 0usize)]);
    let mut shown = BTreeMap::<(String, String, String, String, String), Arc>::new();
    let mut truncated = false;
    while let Some((current, distance)) = queue.pop_front() {
        if distance == depth { continue; }
        for arc in &all {
            let other = if (direction == "out" || direction == "both") && arc.from == current {
                Some(&arc.to)
            } else if (direction == "in" || direction == "both") && arc.to == current {
                Some(&arc.from)
            } else { None };
            if let Some(other) = other {
                if shown.len() >= MAX_QUERY_EDGES {
                    truncated = true;
                    break;
                }
                shown.insert((
                    arc.from.clone(), arc.to.clone(), arc.kind.clone(),
                    arc.status.clone(), arc.evidence.clone(),
                ), arc.clone());
                if visited.insert(other.clone()) {
                    queue.push_back((other.clone(), distance + 1));
                }
            }
        }
        if truncated { break; }
    }
    presentation(db, &id, &visited, &shown.into_values().collect::<Vec<_>>(),
                 if truncated { "PARTIAL_EDGE_LIMIT" } else { "OBSERVED_RELATIONSHIPS" })
}
pub fn path(db: &Db, graph_id: Option<&str>, target: Option<&str>,
            from: &str, to: &str, depth: usize) -> Result<Value> {
    if depth == 0 || depth > MAX_DEPTH { return refuse("graph depth must be 1..8"); }
    let (id, _) = find(db, graph_id, target)?;
    let start = resolve_node(db, &id, from)?;
    let goal = resolve_node(db, &id, to)?;
    let mut all = arcs(db, &id)?;
    all.sort_by(|a,b| (&a.from,&a.to,&a.kind).cmp(&(&b.from,&b.to,&b.kind)));
    let mut visited = BTreeSet::from([start.clone()]);
    let mut queue = VecDeque::from([(start.clone(), 0usize)]);
    let mut parents = BTreeMap::<String, Arc>::new();
    while let Some((here, dist)) = queue.pop_front() {
        if here == goal { break; }
        if dist == depth { continue; }
        for arc in all.iter().filter(|arc| arc.from == here) {
            if visited.insert(arc.to.clone()) {
                parents.insert(arc.to.clone(), arc.clone());
                queue.push_back((arc.to.clone(), dist + 1));
            }
        }
    }
    if !visited.contains(&goal) {
        return Ok(obj(&[
            ("schema", Value::string("excession.graph-query.v1")),
            ("graph_id", Value::string(id)),
            ("status", Value::string("NO_RECORDED_PATH_IN_SELECTED_DIRECTION_AND_DEPTH")),
            ("from", Value::string(start)),
            ("to", Value::string(goal)),
            ("max_depth", Value::number(depth)),
            ("warning", Value::string("Absence of a recorded path is not evidence of absence of a dependency")),
        ]));
    }
    let mut current = goal;
    let mut selected = Vec::new();
    let mut nodes = BTreeSet::from([current.clone()]);
    while current != start {
        let arc = parents.get(&current).ok_or("missing BFS predecessor")?.clone();
        current = arc.from.clone();
        nodes.insert(current.clone());
        selected.push(arc);
    }
    selected.reverse();
    presentation(db, &id, &nodes, &selected, "RECORDED_EVIDENCE_PATH")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_vocabulary_is_not_tied_to_one_build_system() {
        assert_eq!(configured_store::role("git-source"), configured_store::role("kbuild-source"));
        assert_eq!(configured_store::role("configured-artifact"), configured_store::role("kbuild-object"));
        assert_eq!(configured_store::relation("PREPROCESSOR_INPUT"), configured_store::relation("COMPILES_SOURCE"));
        assert_eq!(configured_store::relation("unknown-new-edge"), "unknown");
    }
    #[test]
    fn provider_detection_is_fail_closed() {
        assert!(provider_for(Path::new("/does-not-exist"), "lib", "auto").is_err());
        assert_eq!(provider_for(Path::new("."), "lib", "cmake").unwrap(), "cmake");
        assert!(provider_for(Path::new("."), "lib", "ninja").is_err());
    }
}
