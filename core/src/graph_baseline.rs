//! Native graph-only retrieval baseline for controlled comparisons.
//! No virtual paths, projection manifest, or view-role amplification.
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, git, index, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

fn required(row: &[Option<String>], index: usize) -> Result<&str> {
    row.get(index)
        .and_then(Option::as_deref)
        .ok_or_else(|| "unexpected SQLite NULL in graph baseline".to_owned())
}

fn distances(seed: &str, edges: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::from([(seed.to_owned(), 0)]);
    let mut queue = VecDeque::from([seed.to_owned()]);
    while let Some(current) = queue.pop_front() {
        let distance = result[&current];
        if let Some(neighbors) = edges.get(&current) {
            for neighbor in neighbors {
                if !result.contains_key(neighbor) {
                    result.insert(neighbor.clone(), distance + 1);
                    queue.push_back(neighbor.clone());
                }
            }
        }
    }
    result
}

/// Run a native, direct SQL dependency traversal against exactly one pinned
/// Git revision. This intentionally does not generate or authenticate a view.
pub fn paths(db: &Db, name: &str, revision: &str, seed: &str, kind: &str) -> Result<Value> {
    if !matches!(
        kind,
        "focus"
            | "dependencies_direct"
            | "dependencies_all"
            | "dependents_direct"
            | "dependents_all"
            | "tests"
    ) {
        return refuse("unknown graph baseline query kind");
    }

    let repo_id = db
        .scalar(
            "SELECT id FROM repository WHERE name=?",
            &[Param::Text(name)],
        )?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    let repo = PathBuf::from(db.scalar(
        "SELECT git_dir FROM repository WHERE id=?",
        &[Param::Integer(repo_id)],
    )?);
    let commit = git::rev(&repo, revision)?;
    let tree = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&commit)],
    )?;
    if tree != git::rev(&repo, &format!("{commit}^{{tree}}"))?
        || tree != index::reconstruct_tree(db, repo_id, &commit)?
    {
        return refuse("graph baseline indexed Git tree mismatch");
    }

    let mut modules = BTreeMap::new();
    for row in db.query(
        "SELECT name,path,is_test FROM module WHERE repo_id=? AND commit_oid=? ORDER BY name",
        &[Param::Integer(repo_id), Param::Text(&commit)],
    )? {
        let name = required(&row, 0)?.to_owned();
        let path = required(&row, 1)?.to_owned();
        let test = match required(&row, 2)? {
            "0" => false,
            "1" => true,
            _ => return refuse("invalid indexed test flag"),
        };
        if modules.insert(name, (path, test)).is_some() {
            return refuse("duplicate indexed module");
        }
    }
    if !modules.contains_key(seed) {
        return refuse(format!("unknown indexed seed module: {seed}"));
    }

    let mut forward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut backward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in db.query(
        "SELECT source_name,target_name FROM dependency WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name",
        &[Param::Integer(repo_id), Param::Text(&commit)],
    )? {
        let from = required(&row, 0)?.to_owned();
        let to = required(&row, 1)?.to_owned();
        if !modules.contains_key(&from) || !modules.contains_key(&to) {
            return refuse("dependency refers to missing module");
        }
        forward.entry(from.clone()).or_default().insert(to.clone());
        backward.entry(to).or_default().insert(from);
    }

    let chosen: BTreeSet<String> = match kind {
        "focus" => BTreeSet::from([modules[seed].0.clone()]),
        "tests" => {
            let mut reachable = distances(seed, &forward)
                .into_keys()
                .chain(distances(seed, &backward).into_keys())
                .collect::<BTreeSet<_>>();
            reachable.remove(seed);
            reachable
                .into_iter()
                .filter(|module| modules[module].1)
                .map(|module| modules[&module].0.clone())
                .collect()
        }
        _ => {
            let use_forward = kind.starts_with("dependencies");
            let direct_only = kind.ends_with("_direct");
            distances(seed, if use_forward { &forward } else { &backward })
                .into_iter()
                .filter(|(_, distance)| *distance > 0 && (!direct_only || *distance == 1))
                .map(|(module, _)| modules[&module].0.clone())
                .collect()
        }
    };
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("schema", Value::string("excession.graph-baseline.v1")),
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("kind", Value::string(kind)),
        (
            "source_paths",
            Value::array(chosen.into_iter().map(Value::string)),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_cycles_keep_shortest_distances() {
        let edges = BTreeMap::from([
            (
                "a".to_owned(),
                BTreeSet::from(["b".to_owned(), "c".to_owned()]),
            ),
            ("b".to_owned(), BTreeSet::from(["a".to_owned()])),
        ]);
        let found = distances("a", &edges);
        assert_eq!(found["a"], 0);
        assert_eq!(found["b"], 1);
        assert_eq!(found["c"], 1);
        assert_eq!(found.len(), 3);
    }
}
