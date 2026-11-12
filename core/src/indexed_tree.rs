//! Reconstruct and authenticate Git tree identities from SQLite index rows.
//! Shared by revision publication and incremental parent-snapshot verification.
//! The Git object format remains authoritative; SQLite only records identities.
use crate::digest;
use crate::git;
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Node {
    name: String,
    mode: String,
    oid: String,
    kind: String,
}

/// Verify a recorded tree using only the recorded Git entries. Subtrees are
/// checked bottom-up, and each computed directory is handed to its parent once.
pub fn reconstruct_tree(db: &Db, repo: i64, commit: &str) -> Result<String> {
    let expected = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo), Param::Text(commit)],
    )?;
    let entries = db
        .query(
            "SELECT path,mode,kind,oid FROM tree_entry WHERE repo_id=? AND commit_oid=? ORDER BY path",
            &[Param::Integer(repo), Param::Text(commit)],
        )?
        .into_iter()
        .map(|row| {
            let [path, mode, kind, oid]: [Option<String>; 4] =
                row.try_into().map_err(|_| "invalid indexed tree row")?;
            Ok(git::Entry {
                path: path.ok_or("NULL tree path")?,
                mode: mode.ok_or("NULL tree mode")?,
                kind: kind.ok_or("NULL tree kind")?,
                oid: oid.ok_or("NULL tree OID")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    reconstruct_entries(&entries, &expected)
}

fn reconstruct_entries(entries: &[git::Entry], expected: &str) -> Result<String> {
    git::oid(expected)?;

    // The tree rows already enumerate every directory. Retaining the declared
    // object IDs also removes a SQLite round-trip for each computed subtree.
    let mut registered = BTreeMap::new();
    let mut declared_trees = BTreeMap::new();
    let mut children: BTreeMap<&str, Vec<Node>> = BTreeMap::new();
    let mut directories = vec![""];

    for entry in entries {
        git::validate_path(&entry.path)?;
        git::oid(&entry.oid)?;
        if registered
            .insert(entry.path.as_str(), entry.kind.as_str())
            .is_some()
        {
            return refuse("duplicate tree entry");
        }

        let (parent, name) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
        if entry.kind == "tree" {
            if entry.mode != "040000" {
                return refuse("invalid directory mode");
            }
            declared_trees.insert(entry.path.as_str(), entry.oid.as_str());
            directories.push(&entry.path);
        } else {
            if !matches!(
                (entry.kind.as_str(), entry.mode.as_str()),
                ("blob", "100644" | "100755" | "120000") | ("commit", "160000")
            ) {
                return refuse("invalid indexed Git mode/kind");
            }
            children.entry(parent).or_default().push(Node {
                name: name.to_owned(),
                mode: entry.mode.clone(),
                oid: entry.oid.clone(),
                kind: entry.kind.clone(),
            });
        }
    }

    // A path's parent must exist as a tree, not merely as an inferred prefix.
    // In particular, a file may not also serve as a parent directory.
    for path in registered.keys() {
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        if !parent.is_empty() && registered.get(parent) != Some(&"tree") {
            return refuse(format!("missing parent tree entry: {parent}"));
        }
    }

    if entries.is_empty() {
        let actual = digest::git_tree(&[]);
        if actual != expected {
            return refuse("empty tree OID mismatch");
        }
        return Ok(actual);
    }

    directories.sort_by_key(|path| {
        std::cmp::Reverse(if path.is_empty() {
            0
        } else {
            1 + path.bytes().filter(|byte| *byte == b'/').count()
        })
    });

    for directory in directories {
        let mut nodes = children.remove(directory).unwrap_or_default();
        // Git compares a tree's name as if it has a trailing slash. Compare
        // byte iterators instead of allocating a sort key at each comparison.
        nodes.sort_unstable_by(|a, b| {
            a.name
                .bytes()
                .chain((a.kind == "tree").then_some(b'/'))
                .cmp(b.name.bytes().chain((b.kind == "tree").then_some(b'/')))
        });
        if nodes.windows(2).any(|pair| pair[0].name == pair[1].name) {
            return refuse("duplicate tree basename");
        }

        let mut payload = Vec::new();
        for node in &nodes {
            payload.extend_from_slice(node.mode.trim_start_matches('0').as_bytes());
            payload.push(b' ');
            payload.extend_from_slice(node.name.as_bytes());
            payload.push(0);
            let raw = digest::unhex(&node.oid)?;
            if raw.len() != 20 {
                return refuse("invalid Git object byte length");
            }
            payload.extend_from_slice(&raw);
        }

        let actual = digest::git_tree(&payload);
        if let Some(&declared) = declared_trees.get(directory)
            && actual != declared
        {
            return refuse(format!("subtree mismatch: {directory}"));
        }
        if directory.is_empty() {
            if actual != expected {
                return refuse(format!("root mismatch: {actual} vs {expected}"));
            }
            return Ok(actual);
        }

        let (parent, name) = directory.rsplit_once('/').unwrap_or(("", directory));
        children.entry(parent).or_default().push(Node {
            name: name.to_owned(),
            mode: "40000".to_owned(),
            oid: actual,
            kind: "tree".to_owned(),
        });
    }
    refuse("missing computed root")
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    // These tree and blob IDs were obtained independently with git hash-object
    // and git mktree (including an executable and a symlink).
    const ROOT: &str = "fb358e2c6a35e1550abfe881f1bfa2f67ae99359";

    fn entry(path: &str, mode: &str, kind: &str, oid: &str) -> git::Entry {
        git::Entry {
            path: path.to_owned(),
            mode: mode.to_owned(),
            kind: kind.to_owned(),
            oid: oid.to_owned(),
        }
    }

    fn fixture() -> Vec<git::Entry> {
        vec![
            entry(
                "src/sub/run.sh",
                "100755",
                "blob",
                "e37f89b3b76e73e0d000552c897006f0b8ba1b76",
            ),
            entry(
                "src/link",
                "120000",
                "blob",
                "7444fe0be13f52655ac0ae0b39e50bc6d24562e0",
            ),
            entry(
                "README.md",
                "100644",
                "blob",
                "e845566c06f9bf557d35e8292c37cf05d97a9769",
            ),
            entry(
                "src",
                "040000",
                "tree",
                "ca8514f19a977b33644c22919f1dbd8f144e8507",
            ),
            entry(
                "src/lib.rs",
                "100644",
                "blob",
                "f0d6ccedb89e7e7c1e49694421c1c4530e62ec94",
            ),
            entry(
                "src/sub",
                "040000",
                "tree",
                "a80548db97f17b12b82cfc7880580bdc4f41ab05",
            ),
        ]
    }

    #[test]
    fn reconstructs_independent_git_tree_regardless_of_row_order() {
        let mut rows = fixture();
        assert_eq!(reconstruct_entries(&rows, ROOT).unwrap(), ROOT);
        rows.reverse();
        assert_eq!(reconstruct_entries(&rows, ROOT).unwrap(), ROOT);
        assert_eq!(
            reconstruct_entries(&[], "4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap(),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
    }

    #[test]
    fn refuses_tampering_and_invalid_tree_structures() {
        let mut rows = fixture();
        rows.iter_mut()
            .find(|e| e.path == "src/lib.rs")
            .unwrap()
            .mode = "100755".into();
        assert!(reconstruct_entries(&rows, ROOT).is_err());

        let mut rows = fixture();
        rows.iter_mut().find(|e| e.path == "src/sub").unwrap().oid = "0".repeat(40);
        assert!(
            reconstruct_entries(&rows, ROOT)
                .unwrap_err()
                .contains("subtree mismatch")
        );

        let mut rows = fixture();
        rows.retain(|e| e.path != "src/sub");
        assert!(
            reconstruct_entries(&rows, ROOT)
                .unwrap_err()
                .contains("missing parent tree")
        );

        let mut rows = fixture();
        rows.push(rows[0].clone());
        assert!(
            reconstruct_entries(&rows, ROOT)
                .unwrap_err()
                .contains("duplicate")
        );

        let mut rows = fixture();
        rows[0].path = "src//bad".into();
        assert!(reconstruct_entries(&rows, ROOT).is_err());

        assert!(reconstruct_entries(&[], ROOT).is_err());
    }
}
