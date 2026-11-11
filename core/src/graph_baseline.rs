//! A deliberately small, view-free graph query for benchmark comparisons.
//!
//! Shares the indexed Git revision and SQL relations with `projection`, but
//! neither constructs virtual paths nor calls any projection/view code.
use crate::git;
use crate::index;
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

const KINDS: &[&str] = &[
    "focus",
    "dependencies_direct",
    "dependencies_all",
    "dependents_direct",
    "dependents_all",
    "tests",
];

fn required(row: &[Option<String>], idx: usize) -> Result<&str> {
    row.get(idx)
        .and_then(Option::as_deref)
        .ok_or_else(|| "NULL indexed graph value".into())
}

fn distances(seed: &str, edges: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, usize> {
    let mut found = BTreeMap::from([(seed.to_owned(), 0)]);
    let mut pending = VecDeque::from([seed.to_owned()]);
    while let Some(name) = pending.pop_front() {
        let depth = found[&name];
        if let Some(neighbors) = edges.get(&name) {
            for target in neighbors {
                if !found.contains_key(target) {
                    found.insert(target.clone(), depth + 1);
                    pending.push_back(target.clone());
                }
            }
        }
    }
    found
}

/// Return only original source paths. Unlike a derived view this does not
/// create aliases, roles or a manifest. We still validate the pinned Git tree,
/// so the timing includes revision and index-integrity work.
pub fn query(db: &Db, name: &str, revision: &str, seed: &str, kind: &str) -> Result<Value> {
    if !KINDS.contains(&kind) {
        return refuse(format!("unknown graph query kind: {kind}"));
    }
    let rows = db.query(
        "SELECT id,git_dir FROM repository WHERE name=?",
        &[Param::Text(name)],
    )?;
    if rows.len() != 1 {
        return refuse("unknown indexed repository");
    }
    let id: i64 = required(&rows[0], 0)?
        .parse()
        .map_err(|_| "invalid repository ID")?;
    let root = Path::new(required(&rows[0], 1)?);
    let commit = git::rev(root, revision)?;
    let tree = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(id), Param::Text(&commit)],
    )?;
    if tree != git::rev(root, &format!("{commit}^{{tree}}"))? {
        return refuse("indexed revision tree mismatch");
    }
    if index::reconstruct_tree(db, id, &commit)? != tree {
        return refuse("indexed Git tree reconstruction mismatch");
    }

    let mut modules: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for row in db.query(
        "SELECT name,path,is_test FROM module WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(id), Param::Text(&commit)],
    )? {
        let is_test = match required(&row, 2)? {
            "0" => false,
            "1" => true,
            _ => return refuse("invalid indexed test flag"),
        };
        modules.insert(
            required(&row, 0)?.to_owned(),
            (required(&row, 1)?.to_owned(), is_test),
        );
    }
    if !modules.contains_key(seed) {
        return refuse("unknown indexed seed module");
    }
    let mut forward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in db.query(
        "SELECT source_name,target_name FROM dependency WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(id), Param::Text(&commit)],
    )? {
        let source = required(&row, 0)?;
        let target = required(&row, 1)?;
        if !modules.contains_key(source) || !modules.contains_key(target) {
            return refuse("dependency references unknown indexed module");
        }
        forward
            .entry(source.to_owned())
            .or_default()
            .insert(target.to_owned());
        reverse
            .entry(target.to_owned())
            .or_default()
            .insert(source.to_owned());
    }

    let mut paths = BTreeSet::new();
    match kind {
        "focus" => {
            paths.insert(modules[seed].0.clone());
        }
        "tests" => {
            let mut scope = distances(seed, &forward);
            scope.extend(distances(seed, &reverse));
            for module in scope.keys().filter(|m| m.as_str() != seed) {
                if modules[module].1 {
                    paths.insert(modules[module].0.clone());
                }
            }
        }
        other => {
            let direct = other.ends_with("_direct");
            let graph = if other.starts_with("dependencies_") {
                &forward
            } else {
                &reverse
            };
            for (module, depth) in distances(seed, graph) {
                if depth > 0 && (!direct || depth == 1) {
                    paths.insert(modules[&module].0.clone());
                }
            }
        }
    }
    Ok(obj(&[(
        "source_paths",
        Value::array(paths.into_iter().map(Value::string)),
    )]))
}
