//! Read-only, source-bound refactoring hypotheses over the existing module index.
//! These are structural observations, not proofs of equivalent behavior.
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, git, index, refuse, views};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_MODULES: usize = 10_000;
const MAX_EDGES: usize = 50_000;
const MAX_SYMBOLS: usize = 100_000;
const MAX_CANDIDATES: usize = 128;
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct Module {
    path: String,
    blob: String,
    is_test: bool,
}

#[derive(Clone)]
struct Edge {
    source: String,
    target: String,
    line: i64,
    syntax: String,
}

#[derive(Clone)]
struct Symbol {
    module: String,
    name: String,
    kind: String,
    signature: String,
    fingerprint: String,
    line: i64,
}

fn field(row: &[Option<String>], column: usize) -> Result<&str> {
    row.get(column)
        .and_then(Option::as_deref)
        .ok_or_else(|| "unexpected NULL in indexed refactor evidence".to_owned())
}

fn number(row: &[Option<String>], column: usize) -> Result<i64> {
    field(row, column)?
        .parse::<i64>()
        .map_err(|_| "invalid indexed integer".to_owned())
}

fn module_evidence(name: &str, modules: &BTreeMap<String, Module>) -> Result<Value> {
    let m = modules
        .get(name)
        .ok_or_else(|| format!("missing indexed module {name}"))?;
    Ok(obj(&[
        ("name", Value::string(name)),
        ("path", Value::string(&m.path)),
        ("blob", Value::string(&m.blob)),
        ("is_test", Value::Bool(m.is_test)),
    ]))
}

fn edge_evidence(edge: &Edge) -> Value {
    obj(&[
        ("source", Value::string(&edge.source)),
        ("target", Value::string(&edge.target)),
        ("line", Value::number(edge.line)),
        ("syntax", Value::string(&edge.syntax)),
    ])
}

fn symbol_evidence(symbol: &Symbol, modules: &BTreeMap<String, Module>) -> Result<Value> {
    let m = modules
        .get(&symbol.module)
        .ok_or("symbol refers to absent indexed module")?;
    Ok(obj(&[
        ("module", Value::string(&symbol.module)),
        ("path", Value::string(&m.path)),
        ("blob", Value::string(&m.blob)),
        ("name", Value::string(&symbol.name)),
        ("line", Value::number(symbol.line)),
        ("kind", Value::string(&symbol.kind)),
    ]))
}

fn identify(mut proposal: Value) -> Result<Value> {
    let id = proposal.sha256();
    proposal
        .entries_mut()?
        .insert("candidate_id".to_owned(), Value::string(id));
    Ok(proposal)
}

/// Kosaraju with explicit stacks: a long dependency chain cannot overflow
/// the process stack. Every node and edge is visited a bounded number of times.
fn cycles(nodes: &BTreeMap<String, Module>, edges: &[Edge]) -> Vec<Vec<String>> {
    let mut forward: BTreeMap<String, Vec<String>> = nodes
        .keys()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    let mut reverse = forward.clone();
    for e in edges {
        forward
            .get_mut(&e.source)
            .expect("edge checked")
            .push(e.target.clone());
        reverse
            .get_mut(&e.target)
            .expect("edge checked")
            .push(e.source.clone());
    }
    for neighbors in forward.values_mut().chain(reverse.values_mut()) {
        neighbors.sort();
        neighbors.dedup();
    }

    let mut visited = BTreeSet::new();
    let mut finish = Vec::with_capacity(nodes.len());
    for start in nodes.keys() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut stack = vec![(start.clone(), 0_usize)];
        while let Some((node, index)) = stack.pop() {
            let neighbors = &forward[&node];
            if let Some(next) = neighbors.get(index) {
                stack.push((node, index + 1));
                if visited.insert(next.clone()) {
                    stack.push((next.clone(), 0));
                }
            } else {
                finish.push(node);
            }
        }
    }

    visited.clear();
    let mut result = Vec::new();
    for start in finish.into_iter().rev() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for next in &reverse[&node] {
                if visited.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
            members.push(node);
        }
        if members.len() > 1 {
            members.sort();
            result.push(members);
        }
    }
    result.sort();
    result
}

fn propose(
    modules: &BTreeMap<String, Module>,
    edges: &[Edge],
    symbols: &[Symbol],
) -> Result<Vec<Value>> {
    let mut candidates = Vec::new();
    for members in cycles(modules, edges) {
        let names: BTreeSet<_> = members.iter().map(String::as_str).collect();
        let internal = edges
            .iter()
            .filter(|e| names.contains(e.source.as_str()) && names.contains(e.target.as_str()))
            .map(edge_evidence)
            .collect::<Vec<_>>();
        candidates.push(identify(obj(&[
            ("kind", Value::string("dependency_cycle")),
            ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESIS")),
            ("action", Value::string("Examine module boundaries and break an import cycle; do not assume an edge is removable.")),
            ("modules", Value::array(members.iter().map(|n| module_evidence(n, modules)).collect::<Result<Vec<_>>>()?)),
            ("edges", Value::array(internal)),
            ("verification", Value::array([
                Value::string("Inspect runtime import and initialization behavior."),
                Value::string("Run affected module and integration tests before and after a proposed change."),
                Value::string("Reindex and confirm the intended dependency cycle is absent."),
            ])),
        ]))?);
    }

    // Parser fingerprints only compare within the same language. Python AST
    // fingerprints include identifiers; JS printer fingerprints are likewise
    // syntactic. Neither establishes semantic or behavioral equivalence.
    type Key = (String, String, String, String);
    let mut groups: BTreeMap<Key, Vec<&Symbol>> = BTreeMap::new();
    for symbol in symbols {
        if !matches!(symbol.kind.as_str(), "function" | "async_function") {
            continue;
        }
        let path = &modules[&symbol.module].path;
        let language = if path.ends_with(".py") {
            "python"
        } else {
            "javascript"
        };
        groups
            .entry((
                language.to_owned(),
                symbol.kind.clone(),
                symbol.signature.clone(),
                symbol.fingerprint.clone(),
            ))
            .or_default()
            .push(symbol);
    }
    for ((language, kind, signature, fingerprint), group) in groups {
        if group.len() < 2 {
            continue;
        }
        let distinct_modules: BTreeSet<_> = group.iter().map(|s| s.module.as_str()).collect();
        if distinct_modules.len() < 2 {
            continue;
        }
        candidates.push(identify(obj(&[
            ("kind", Value::string("duplicate_body_fingerprint")),
            ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESIS")),
            ("action", Value::string("Review repeated parser-normalized function bodies for possible consolidation or intentional duplication.")),
            ("language", Value::string(language)),
            ("symbol_kind", Value::string(kind)),
            ("signature", Value::string(signature)),
            ("body_fingerprint", Value::string(fingerprint)),
            ("symbols", Value::array(group.iter().map(|s| symbol_evidence(s, modules)).collect::<Result<Vec<_>>>()?)),
            ("verification", Value::array([
                Value::string("Check public contracts, callers, side effects and intended independence."),
                Value::string("Prove equivalence for relevant inputs or retain distinct implementations."),
                Value::string("Run language-native type checks and affected tests."),
            ])),
        ]))?);
    }
    if candidates.len() > MAX_CANDIDATES {
        return refuse(
            "refactor candidates exceed the bounded report limit; narrow indexing roots",
        );
    }
    candidates.sort_by_key(Value::canonical);
    Ok(candidates)
}

/// Produce deterministic, read-only hypotheses for a previously indexed Git
/// revision. Reconstructing the tree authenticates source identities, but
/// indexed language relations are not independently reparsed by this command.
pub fn suggest(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let repo_id = views::repo_id(db, name)?;
    let root = PathBuf::from(db.scalar(
        "SELECT git_dir FROM repository WHERE id=?",
        &[Param::Integer(repo_id)],
    )?);
    let commit = git::rev(&root, revision)?;
    let tree = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&commit)],
    )?;
    if tree != git::rev(&root, &format!("{commit}^{{tree}}"))?
        || tree != index::reconstruct_tree(db, repo_id, &commit)?
    {
        return refuse("indexed refactor source tree does not match Git");
    }

    let args = &[Param::Integer(repo_id), Param::Text(&commit)];
    let module_rows = db.query(
        "SELECT name,path,blob_oid,is_test FROM module WHERE repo_id=? AND commit_oid=? ORDER BY name",
        args,
    )?;
    if module_rows.len() > MAX_MODULES {
        return refuse("refactor module limit exceeded");
    }
    let mut modules = BTreeMap::new();
    for row in module_rows {
        let name = field(&row, 0)?.to_owned();
        let test = number(&row, 3)?;
        if !matches!(test, 0 | 1) {
            return refuse("invalid indexed test flag");
        }
        if modules
            .insert(
                name,
                Module {
                    path: field(&row, 1)?.to_owned(),
                    blob: field(&row, 2)?.to_owned(),
                    is_test: test == 1,
                },
            )
            .is_some()
        {
            return refuse("duplicate indexed module");
        }
    }

    let edge_rows = db.query(
        "SELECT source_name,target_name,line,syntax FROM dependency WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name,line,syntax",
        args,
    )?;
    if edge_rows.len() > MAX_EDGES {
        return refuse("refactor dependency limit exceeded");
    }
    let mut edges = Vec::with_capacity(edge_rows.len());
    for row in edge_rows {
        let edge = Edge {
            source: field(&row, 0)?.to_owned(),
            target: field(&row, 1)?.to_owned(),
            line: number(&row, 2)?,
            syntax: field(&row, 3)?.to_owned(),
        };
        if !modules.contains_key(&edge.source)
            || !modules.contains_key(&edge.target)
            || edge.line < 1
        {
            return refuse("invalid indexed refactor dependency");
        }
        edges.push(edge);
    }

    let symbol_rows = db.query(
        "SELECT module_name,qualname,kind,signature,body_fingerprint,line FROM symbol WHERE repo_id=? AND commit_oid=? ORDER BY module_name,qualname",
        args,
    )?;
    if symbol_rows.len() > MAX_SYMBOLS {
        return refuse("refactor symbol limit exceeded");
    }
    let mut symbols = Vec::with_capacity(symbol_rows.len());
    for row in symbol_rows {
        let symbol = Symbol {
            module: field(&row, 0)?.to_owned(),
            name: field(&row, 1)?.to_owned(),
            kind: field(&row, 2)?.to_owned(),
            signature: field(&row, 3)?.to_owned(),
            fingerprint: field(&row, 4)?.to_owned(),
            line: number(&row, 5)?,
        };
        if !modules.contains_key(&symbol.module) || symbol.line < 1 {
            return refuse("invalid indexed refactor symbol");
        }
        symbols.push(symbol);
    }

    let hazards = db
        .scalar(
            "SELECT COUNT(*) FROM hazard WHERE repo_id=? AND commit_oid=?",
            args,
        )?
        .parse::<u64>()
        .map_err(|_| "invalid indexed hazard count")?;
    db.foreign_keys_valid()?;

    let mut report = obj(&[
        ("schema", Value::string("excession.refactor-candidates.v1")),
        ("status", Value::string("UNVERIFIED_REFACTOR_HYPOTHESES")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("repository", Value::string(name)),
        (
            "coverage",
            obj(&[
                (
                    "scope",
                    Value::string("indexed Python/JavaScript modules only"),
                ),
                ("indexed_modules", Value::number(modules.len())),
                ("indexed_dependencies", Value::number(edges.len())),
                ("indexed_symbols", Value::number(symbols.len())),
                ("indexed_hazards", Value::number(hazards)),
                (
                    "limitations",
                    Value::string(
                        "Missing dynamic, unresolved, runtime, cross-language or non-indexed relationships are unknown, not absent. Source identity is Git-checked; semantic observations are not re-extracted by this command.",
                    ),
                ),
            ]),
        ),
        (
            "candidates",
            Value::array(propose(&modules, &edges, &symbols)?),
        ),
    ]);
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(report_id));
    if report.canonical().len() > MAX_REPORT_BYTES {
        return refuse("refactor report exceeds 8 MiB");
    }
    Ok(report)
}

/// Replay against the current indexed relations and pinned Git tree. This
/// detects altered reports or changed SQL relations, not a forged parser index.
pub fn audit(db: &Db, name: &str, revision: &str, file: &Path) -> Result<Value> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REPORT_BYTES {
        return refuse("refactor report exceeds 8 MiB");
    }
    let supplied = json::parse(&bytes)?;
    let regenerated = suggest(db, name, revision)?;
    if supplied != regenerated {
        return refuse("refactor report differs from independently regenerated index query");
    }
    Ok(obj(&[
        ("report_id", regenerated.get("report_id")?.clone()),
        ("source_commit", regenerated.get("source_commit")?.clone()),
        ("status", Value::string("REPLAYED_INDEXED_RELATIONS")),
        ("semantic_reparse_performed", Value::Bool(false)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Module {
        Module {
            path: "pkg/mod.py".to_owned(),
            blob: "0000000000000000000000000000000000000000".to_owned(),
            is_test: false,
        }
    }

    fn e(source: &str, target: &str) -> Edge {
        Edge {
            source: source.to_owned(),
            target: target.to_owned(),
            line: 1,
            syntax: "import".to_owned(),
        }
    }

    #[test]
    fn cycle_detection_distinguishes_chains_and_sccs() {
        let modules = ["a", "b", "c", "d"]
            .into_iter()
            .map(|x| (x.to_owned(), m()))
            .collect();
        assert_eq!(
            cycles(
                &modules,
                &[e("a", "b"), e("b", "a"), e("b", "c"), e("c", "d")]
            ),
            vec![vec!["a".to_owned(), "b".to_owned()]]
        );
        assert!(cycles(&modules, &[e("a", "b"), e("b", "c")]).is_empty());
    }

    #[test]
    fn duplicate_fingerprints_do_not_cross_languages() {
        let modules = BTreeMap::from([
            (
                "a".to_owned(),
                Module {
                    path: "src/a.py".into(),
                    ..m()
                },
            ),
            (
                "b".to_owned(),
                Module {
                    path: "src/b.js".into(),
                    ..m()
                },
            ),
        ]);
        let symbols = ["a", "b"]
            .into_iter()
            .map(|name| Symbol {
                module: name.to_owned(),
                name: "same".to_owned(),
                kind: "function".to_owned(),
                signature: String::new(),
                fingerprint: "same".to_owned(),
                line: 1,
            })
            .collect::<Vec<_>>();
        assert!(propose(&modules, &[], &symbols).unwrap().is_empty());
    }
}
