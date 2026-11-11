//! Assemble configured build observations with Git-bound compiler receipts.
//! CMake File API supplies target structure; Clang supplies TU facts.
//! Neither is promoted into observed build execution or whole-program proof.
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_JSON: usize = 48 * 1024 * 1024;
const MAX_TUS: usize = 128;
const MAX_NODES: usize = 30_000;
const MAX_EDGES: usize = 80_000;

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() > MAX_JSON {
        return refuse(format!("JSON input exceeds limit: {}", path.display()));
    }
    crate::json::parse(&bytes)
}
fn optional<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.entries().ok()?.get(key)
}
fn values<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    match optional(value, key) {
        None => Ok(&[]),
        Some(v) => v.array_ref(),
    }
}
fn read_reply(reply: &Path, filename: &str) -> Result<Value> {
    if filename.is_empty()
        || !Path::new(filename).components().all(|c| matches!(c, Component::Normal(_)))
        || filename.contains('/') || filename.contains('\\')
        || !filename.ends_with(".json")
    {
        return refuse("unsafe CMake File API reply filename");
    }
    read_json(&reply.join(filename))
}
fn sha_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|e| format!("read dependency {}: {e}", path.display()))?;
    if bytes.len() > MAX_JSON {
        return refuse("dependency file exceeds hash limit");
    }
    Ok(digest::sha256(&bytes))
}
fn as_path(source: &Path, build: &Path, name: &str) -> Result<PathBuf> {
    let original = Path::new(name);
    if name.contains('\0') {
        return refuse("NUL in source path");
    }
    if original.is_absolute() {
        return Ok(original.to_path_buf());
    }
    let from_source = source.join(original);
    if from_source.is_file() {
        return Ok(from_source);
    }
    Ok(build.join(original))
}
fn push_node(nodes: &mut BTreeMap<String, Value>, id: String, kind: &str, payload: Value) -> Result<()> {
    if nodes.len() >= MAX_NODES && !nodes.contains_key(&id) {
        return refuse("graph node budget exceeded");
    }
    let entry = obj(&[("id", Value::string(&id)), ("kind", Value::string(kind)),
                      ("payload", payload)]);
    if let Some(old) = nodes.insert(id, entry.clone()) {
        if old != entry {
            return refuse("conflicting graph entity identity");
        }
    }
    Ok(())
}
fn push_edge(edges: &mut BTreeMap<String, Value>, from: &str, to: &str,
             kind: &str, status: &str, evidence: &str) -> Result<()> {
    if edges.len() >= MAX_EDGES {
        return refuse("graph edge budget exceeded");
    }
    let entry = obj(&[
        ("from", Value::string(from)), ("to", Value::string(to)),
        ("kind", Value::string(kind)), ("status", Value::string(status)),
        ("evidence", Value::string(evidence)),
    ]);
    edges.insert(entry.sha256(), entry);
    Ok(())
}
fn source_suffix(path: &str) -> bool {
    [".c", ".cc", ".cpp", ".cxx", ".C"].iter().any(|s| path.ends_with(s))
}
fn validate_receipt(raw: &Value, commit: &str, tree: &str, repo: &Path,
                    db_sha: &str) -> Result<(String, String, Value)> {
    if raw.get("schema")?.str()? != "excession.clang-tu.v1"
        || raw.get("source_commit")?.str()? != commit
        || raw.get("source_tree")?.str()? != tree
    {
        return refuse("compiler receipt has incompatible source revision or schema");
    }
    let id = raw.get("report_id")?.str()?.to_owned();
    let mut body = raw.entries()?.clone();
    body.remove("report_id");
    if Value::Object(body).sha256() != id {
        return refuse("compiler receipt report_id does not match canonical payload");
    }
    let obs = raw.get("observation")?;
    if obs.get("status")?.str()? != "CLANG_AST_OBSERVED_NOT_BUILD_VERIFIED"
        || obs.get("compilation_database")?.get("sha256")?.str()? != db_sha
    {
        return refuse("compiler observation not bound to this build compilation database");
    }
    let path = obs.get("source_path")?.str()?.to_owned();
    git::validate_path(&path)?;
    let blob = git::blob(repo, commit, &path)?;
    if obs.get("source_blob")?.str()? != blob {
        return refuse("compiler receipt source blob mismatch");
    }
    for dep in obs.get("dependencies")?.array_ref()? {
        let name = dep.get("path")?.str()?;
        if sha_file(Path::new(name))? != dep.get("sha256")?.str()? {
            return refuse(format!("compiler dependency changed after observation: {name}"));
        }
    }
    Ok((path, id, obs.clone()))
}

/// Ingest an existing CMake File API codemodel and Clang JSON receipts into a
/// configuration-scoped, immutable relational graph. The build and source
/// checkout must be local; no build or compiler command is executed here.
pub fn assemble(repo: &Path, rev: &str, build: &Path, target_name: &str,
                observations: &Path, db: &Db) -> Result<Value> {
    git::verify_repo(repo)?;
    let repo = repo.canonicalize().map_err(|e| e.to_string())?;
    let build = build.canonicalize().map_err(|e| format!("build directory: {e}"))?;
    let commit = git::rev(&repo, rev)?;
    if git::text(&repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("revision must resolve to commit");
    }
    let tree = git::rev(&repo, &format!("{commit}^{{tree}}"))?;
    let reply = build.join(".cmake/api/v1/reply");
    let mut indexes = fs::read_dir(&reply)
        .map_err(|e| format!("CMake File API reply unavailable: {e}"))?
        .filter_map(|x| x.ok().map(|y| y.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("index-") && n.ends_with(".json")))
        .collect::<Vec<_>>();
    indexes.sort();
    let index = indexes.last().ok_or("CMake File API index is missing")?;
    let header = read_json(index)?;
    let model_ref = values(&header, "objects")?.iter().find(|o| {
        o.get("kind").and_then(Value::str).ok() == Some("codemodel")
            && o.get("version").and_then(|v| v.get("major"))
                .and_then(Value::integer).ok() == Some(2)
    }).ok_or("CMake codemodel v2 object unavailable")?;
    let model_path = model_ref.get("jsonFile")?.str()?;
    let model = read_reply(&reply, model_path)?;
    if model.get("kind")?.str()? != "codemodel"
        || model.get("paths")?.get("source")?.str()? != repo.to_string_lossy()
        || model.get("paths")?.get("build")?.str()? != build.to_string_lossy()
    {
        return refuse("CMake codemodel paths disagree with selected Git source/build");
    }
    let configurations = model.get("configurations")?.array_ref()?;
    if configurations.len() != 1 {
        return refuse("select a single CMake build configuration");
    }
    let config = &configurations[0];
    let config_name = config.get("name")?.str()?;
    let targets = config.get("targets")?.array_ref()?;
    let entry = targets.iter().find(|t| t.get("name").and_then(Value::str).ok()
                              == Some(target_name))
        .ok_or("requested target not in CMake codemodel")?;
    let selected_id = entry.get("id")?.str()?;
    let target_json = read_reply(&reply, entry.get("jsonFile")?.str()?)?;
    if target_json.get("id")?.str()? != selected_id
        || target_json.get("name")?.str()? != target_name {
        return refuse("CMake target identity mismatch");
    }
    let db_path = build.join("compile_commands.json");
    let db_sha = sha_file(&db_path)?;
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let target_id = format!("target:{target_name}");
    push_node(&mut nodes, target_id.clone(), "configured-target",
              obj(&[("name", Value::string(target_name)),
                    ("cmake_id", Value::string(selected_id)),
                    ("configuration", Value::string(config_name))]))?;
    let names: BTreeMap<String, String> = targets.iter().map(|t| {
        Ok((t.get("id")?.str()?.to_owned(), t.get("name")?.str()?.to_owned()))
    }).collect::<Result<_>>()?;
    for dep in values(&target_json, "dependencies")? {
        let id = dep.get("id")?.str()?;
        let name = names.get(id).ok_or("target depends on unknown CMake target")?;
        let node = format!("target:{name}");
        push_node(&mut nodes, node.clone(), "configured-target",
                  obj(&[("name", Value::string(name)),
                        ("cmake_id", Value::string(id)),
                        ("configuration", Value::string(config_name))]))?;
        push_edge(&mut edges, &target_id, &node, "DEPENDS_ON_TARGET",
                  "CMAKE_CONFIGURED", model_path)?;
    }
    for artifact in values(&target_json, "artifacts")? {
        let path = artifact.get("path")?.str()?;
        if path.is_empty() { return refuse("empty target artifact path"); }
        let node = format!("artifact:{path}");
        push_node(&mut nodes, node.clone(), "configured-artifact",
                  obj(&[("path", Value::string(path)),
                        ("configuration", Value::string(config_name))]))?;
        push_edge(&mut edges, &target_id, &node, "PRODUCES_ARTIFACT",
                  "CMAKE_CONFIGURED_NOT_EXECUTION_PROOF", model_path)?;
    }
    let mut source_paths = BTreeSet::new();
    let mut non_git_sources = 0usize;
    for item in values(&target_json, "sources")? {
        let path = item.get("path")?.str()?;
        if !source_suffix(path) { continue; }
        let physical = as_path(&repo, &build, path)?
            .canonicalize().map_err(|e| format!("CMake source {path}: {e}"))?;
        if !physical.starts_with(&repo) {
            non_git_sources += 1;
            continue;
        }
        let relative = physical.strip_prefix(&repo).map_err(|e| e.to_string())?
            .to_string_lossy().replace('\\', "/");
        git::validate_path(&relative)?;
        let blob = git::blob(&repo, &commit, &relative)?;
        let bytes = fs::read(&physical).map_err(|e| e.to_string())?;
        if digest::git_blob(&bytes) != blob {
            return refuse("CMake target source differs from pinned Git blob");
        }
        if !source_paths.insert(relative.clone()) { continue; }
        let node = format!("source:{relative}");
        push_node(&mut nodes, node.clone(), "git-source",
                  obj(&[("path", Value::string(&relative)), ("blob", Value::string(blob))]))?;
        push_edge(&mut edges, &target_id, &node, "TARGET_SOURCE",
                  "CMAKE_CONFIGURED_GIT_VERIFIED", model_path)?;
    }
    if source_paths.is_empty() || source_paths.len() > MAX_TUS {
        return refuse("selected target has no bounded tracked C/C++ source set");
    }
    let mut observed = BTreeMap::<String, (String, Value)>::new();
    let entries = fs::read_dir(observations)
        .map_err(|e| format!("observation directory unavailable: {e}"))?;
    let mut count = 0;
    for file in entries {
        let path = file.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") { continue; }
        count += 1;
        if count > MAX_TUS { return refuse("too many compiler receipts"); }
        let raw = read_json(&path)?;
        let (source, id, obs) = validate_receipt(&raw, &commit, &tree, &repo, &db_sha)?;
        if !source_paths.contains(&source) {
            return refuse("compiler receipt not a selected target source");
        }
        if observed.insert(source, (id, obs)).is_some() {
            return refuse("ambiguous duplicate compiler observations for selected target");
        }
    }
    let unobserved = source_paths.iter().filter(|s| !observed.contains_key(*s))
        .cloned().collect::<Vec<_>>();
    for (source, (receipt, obs)) in &observed {
        let source_node = format!("source:{source}");
        let tu = format!("tu:{source}:{receipt}");
        push_node(&mut nodes, tu.clone(), "compiler-translation-unit",
                  obj(&[("source", Value::string(source)),
                        ("receipt", Value::string(receipt)),
                        ("compilation_database_sha256", Value::string(&db_sha))]))?;
        push_edge(&mut edges, &source_node, &tu, "OBSERVED_AS_TU",
                  "GIT_AND_CLANG_RECEIPT_VERIFIED", receipt)?;
        for dep in obs.get("dependencies")?.array_ref()? {
            let full = dep.get("path")?.str()?;
            let hash = dep.get("sha256")?.str()?;
            let node = format!("dependency:{full}:{hash}");
            push_node(&mut nodes, node.clone(), "observed-input",
                      obj(&[("path", Value::string(full)), ("sha256", Value::string(hash)),
                            ("git_path", optional(dep, "repo_path").cloned().unwrap_or(Value::Null))]))?;
            push_edge(&mut edges, &tu, &node, "PREPROCESSOR_INPUT",
                      "CLANG_OBSERVED_BYTES_MATCH", receipt)?;
        }
        let mut definitions = BTreeMap::<String, Vec<String>>::new();
        for (index, symbol) in obs.get("symbols")?.array_ref()?.iter().enumerate() {
            let name = symbol.get("name")?.str()?;
            let node = format!("symbol:{source}:{index}");
            push_node(&mut nodes, node.clone(), "tu-declaration", symbol.clone())?;
            push_edge(&mut edges, &tu, &node, "DECLARES",
                      "CLANG_AST_OBSERVED", receipt)?;
            if symbol.get("definition")?.bool()? {
                definitions.entry(name.to_owned()).or_default().push(node);
            }
        }
        for (index, call) in obs.get("calls")?.array_ref()?.iter().enumerate() {
            let node = format!("call:{source}:{index}");
            push_node(&mut nodes, node.clone(), "tu-call-site", call.clone())?;
            push_edge(&mut edges, &tu, &node, "CONTAINS_CALL",
                      "CLANG_AST_OBSERVED", receipt)?;
            let callee = call.get("callee")?.str()?;
            if let Some(candidates) = definitions.get(callee) {
                if candidates.len() == 1 {
                    push_edge(&mut edges, &node, &candidates[0], "MAY_REFER_TO",
                              "NAME_MATCH_CANDIDATE_ONLY", receipt)?;
                }
            }
        }
    }
    let status = if unobserved.is_empty() && non_git_sources == 0 {
        "TARGET_TUS_OBSERVED"
    } else { "PARTIAL_TARGET_OBSERVATIONS" };
    let target_sha = sha_file(&reply.join(entry.get("jsonFile")?.str()?))?;
    let model_sha = sha_file(&reply.join(model_path))?;
    let body = obj(&[
        ("schema", Value::string("excession.configured-graph.v1")),
        ("source_commit", Value::string(&commit)),
        ("source_tree", Value::string(&tree)),
        ("build_directory", Value::string(build.to_string_lossy())),
        ("configuration", Value::string(config_name)),
        ("target", Value::string(target_name)),
        ("codemodel_sha256", Value::string(model_sha)),
        ("target_reply_sha256", Value::string(target_sha)),
        ("compilation_database_sha256", Value::string(db_sha)),
        ("status", Value::string(status)),
        ("expected_source_count", Value::number(source_paths.len())),
        ("observed_source_count", Value::number(observed.len())),
        ("unobserved_sources", Value::array(unobserved.iter().map(Value::string))),
        ("non_git_source_count", Value::number(non_git_sources)),
        ("nodes", Value::array(nodes.values().cloned())),
        ("edges", Value::array(edges.values().cloned())),
        ("limitations", Value::array([
            Value::string("CMake configured target graph, not proof of build execution"),
            Value::string("Clang reparses are scoped to source and compilation command"),
            Value::string("Calls and same-name matches do not establish cross-TU symbol resolution"),
            Value::string("Generated and system headers are hashed, not Git-authenticated"),
            Value::string("Target dependency edges are direct CMake codemodel relationships"),
        ])),
    ]);
    let id = body.sha256();
    db.exec("CREATE TABLE IF NOT EXISTS configured_graph(\
        graph_id TEXT PRIMARY KEY, source_commit TEXT NOT NULL,\
        target TEXT NOT NULL, configuration TEXT NOT NULL, document TEXT NOT NULL);\
        CREATE TABLE IF NOT EXISTS configured_entity(\
        graph_id TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL, document TEXT NOT NULL,\
        PRIMARY KEY(graph_id,id), FOREIGN KEY(graph_id) REFERENCES configured_graph(graph_id));\
        CREATE TABLE IF NOT EXISTS configured_edge(\
        graph_id TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL,\
        source_id TEXT NOT NULL, destination_id TEXT NOT NULL, status TEXT NOT NULL,\
        evidence TEXT NOT NULL, PRIMARY KEY(graph_id,id),\
        FOREIGN KEY(graph_id,source_id) REFERENCES configured_entity(graph_id,id),\
        FOREIGN KEY(graph_id,destination_id) REFERENCES configured_entity(graph_id,id));")?;
    db.transaction(|db| {
        db.execute("INSERT OR IGNORE INTO configured_graph VALUES(?,?,?,?,?)",
                   &[Param::Text(&id), Param::Text(&commit), Param::Text(target_name),
                     Param::Text(config_name), Param::Text(&body.canonical())])?;
        for (node_id, node) in &nodes {
            db.execute("INSERT OR IGNORE INTO configured_entity VALUES(?,?,?,?)",
                       &[Param::Text(&id), Param::Text(node_id),
                         Param::Text(node.get("kind")?.str()?),
                         Param::Text(&node.canonical())])?;
        }
        for (edge_id, edge) in &edges {
            db.execute("INSERT OR IGNORE INTO configured_edge VALUES(?,?,?,?,?,?,?)",
                       &[Param::Text(&id), Param::Text(edge_id),
                         Param::Text(edge.get("kind")?.str()?),
                         Param::Text(edge.get("from")?.str()?),
                         Param::Text(edge.get("to")?.str()?),
                         Param::Text(edge.get("status")?.str()?),
                         Param::Text(edge.get("evidence")?.str()?)])?;
        }
        Ok(())
    })?;
    let mut result = body.entries()?.clone();
    result.insert("graph_id".to_owned(), Value::string(id));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_reply_path_traversal() {
        let path = Path::new("/tmp/replies");
        for name in ["../secret.json", "nested/secret.json", "", "target.txt"] {
            assert!(read_reply(path, name).is_err(), "{name}");
        }
    }
    #[test]
    fn c_cpp_recognition() {
        assert!(source_suffix("x.cc"));
        assert!(source_suffix("x.c"));
        assert!(!source_suffix("x.rs"));
    }
}
