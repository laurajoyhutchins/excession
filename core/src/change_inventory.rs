//! Exact two-revision Git blob inventory, distinct from inferred impact.
//! Tree entries are authoritative; the checkout is never read or mutated.
use crate::git::{self, Entry};
use crate::json::{Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn committed(root: &Path, rev: &str) -> Result<(String, String)> {
    let commit = git::rev(root, rev)?;
    if git::text(root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("change inventory requires Git commits");
    }
    let tree = git::rev(root, &format!("{commit}^{{tree}}"))?;
    Ok((commit, tree))
}

fn blobs(entries: &[Entry]) -> BTreeMap<&str, &Entry> {
    entries
        .iter()
        .filter(|e| e.kind == "blob")
        .map(|e| (e.path.as_str(), e))
        .collect()
}

fn source(e: &Entry, commit: &str, tree: &str) -> Value {
    obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(&e.path)),
        ("blob", Value::string(&e.oid)),
        ("mode", Value::string(&e.mode)),
    ])
}

/// Reports exact tracked blob path and mode changes. Renames are deliberately
/// left as add+delete until an independent candidate heuristic is specified.
fn compare(
    old: &[Entry],
    new: &[Entry],
    old_commit: &str,
    old_tree: &str,
    new_commit: &str,
    new_tree: &str,
) -> Vec<Value> {
    let a = blobs(old);
    let b = blobs(new);
    let paths: BTreeSet<_> = a.keys().chain(b.keys()).copied().collect();
    paths
        .into_iter()
        .filter_map(|p| {
            let before = a.get(p).copied();
            let after = b.get(p).copied();
            let kind = match (before, after) {
                (None, Some(_)) => "ADDED",
                (Some(_), None) => "DELETED",
                (Some(a), Some(b)) if a.oid != b.oid => "MODIFIED",
                (Some(a), Some(b)) if a.mode != b.mode => "MODE_CHANGED",
                _ => return None,
            };
            let mut change = obj(&[
                ("kind", Value::string(kind)),
                (
                    "before",
                    before
                        .map(|e| source(e, old_commit, old_tree))
                        .unwrap_or(Value::Null),
                ),
                (
                    "after",
                    after
                        .map(|e| source(e, new_commit, new_tree))
                        .unwrap_or(Value::Null),
                ),
            ]);
            let id = change.sha256();
            // The digest identifies the observation, not a verified semantic result.
            change
                .entries_mut()
                .expect("internal object")
                .insert("change_id".into(), Value::string(id));
            Some(change)
        })
        .collect()
}

pub fn inventory(path: &Path, base: &str, head: &str) -> Result<Value> {
    let root = PathBuf::from(git::text(path, &["rev-parse", "--show-toplevel"])?);
    git::verify_repo(&root)?;
    let (base_commit, base_tree) = committed(&root, base)?;
    let (head_commit, head_tree) = committed(&root, head)?;
    let old = git::entries(&root, &base_commit)?;
    let new = git::entries(&root, &head_commit)?;
    let changes = compare(
        &old,
        &new,
        &base_commit,
        &base_tree,
        &head_commit,
        &head_tree,
    );
    let mut report = obj(&[
        ("schema", Value::string("excession.change-inventory.v1")),
        ("base_commit", Value::string(&base_commit)),
        ("base_tree", Value::string(&base_tree)),
        ("head_commit", Value::string(&head_commit)),
        ("head_tree", Value::string(&head_tree)),
        ("changes", Value::array(changes)),
    ]);
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(id));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, oid: &str, mode: &str) -> Entry {
        Entry {
            path: path.into(),
            oid: oid.into(),
            mode: mode.into(),
            kind: "blob".into(),
        }
    }
    #[test]
    fn exact_git_changes_detect_contents_and_mode_without_guessing_renames() {
        let original = [
            e("a.py", "0", "100644"),
            e("script.sh", "2", "100644"),
            e("gone.py", "3", "100644"),
        ];
        let changed = [
            e("a.py", "1", "100644"),
            e("script.sh", "2", "100755"),
            e("new.py", "3", "100644"),
        ];
        let report = compare(&original, &changed, "a", "b", "c", "d");
        let kinds: Vec<_> = report
            .iter()
            .map(|x| x.get("kind").unwrap().str().unwrap())
            .collect();
        assert_eq!(kinds, ["MODIFIED", "DELETED", "ADDED", "MODE_CHANGED"]);
        assert_eq!(report[1].get("after").unwrap(), &Value::Null);
        assert_eq!(report[2].get("before").unwrap(), &Value::Null);
    }
    #[test]
    fn no_change_for_identical_path_mode_and_blob() {
        let a = [e("src/a.rs", "0", "100644")];
        assert!(compare(&a, &a, "a", "b", "a", "b").is_empty());
    }
}
