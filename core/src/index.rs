//! A rebuildable Git tree index. Tree hashes are computed independently from SQL rows.
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use crate::{digest, git};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn index_revision(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
) -> Result<i64> {
    index_revision_with_roots(db, root, name, commit, provenance, &["h11", "tests"])
}

/// Index the complete Git object/tree/revision identities required by a
/// bounded integration claim, without doing unrelated language parsing.
/// The source tree is still independently reconstructed from SQLite rows.
pub fn index_revision_for_evidence(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
) -> Result<i64> {
    index_revision_inner(db, root, name, commit, provenance, &["h11", "tests"], false)
}

pub fn index_revision_with_roots(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
    roots: &[&str],
) -> Result<i64> {
    index_revision_inner(db, root, name, commit, provenance, roots, true)
}

fn index_revision_inner(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
    roots: &[&str],
    include_semantics: bool,
) -> Result<i64> {
    git::clean(root)?;
    let commit = git::rev(root, commit)?;
    if git::text(root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("Git reference is not a commit");
    }
    let tree = git::rev(root, &format!("{commit}^{{tree}}"))?;
    let entries = git::entries(root, &commit)?;
    db.init_schema()?;

    db.transaction(|db| {
        let repo_id = register_repository(db, root, name, provenance)?;
        let mut recorded_objects = BTreeSet::new();

        // Multiple paths may have identical contents. Persist each distinct
        // (object ID, type) once while retaining every tree-entry relationship.
        for (oid, kind) in [(&commit, "commit"), (&tree, "tree")] {
            if recorded_objects.insert((oid.as_str(), kind)) {
                record_object(db, repo_id, oid, kind)?;
            }
        }
        db.execute(
            "INSERT OR IGNORE INTO revision(repo_id,commit_oid,tree_oid) VALUES(?,?,?)",
            &[
                Param::Integer(repo_id),
                Param::Text(&commit),
                Param::Text(&tree),
            ],
        )?;

        for entry in &entries {
            if recorded_objects.insert((entry.oid.as_str(), entry.kind.as_str())) {
                record_object(db, repo_id, &entry.oid, &entry.kind)?;
            }
            db.execute(
                "INSERT OR IGNORE INTO tree_entry(repo_id,commit_oid,path,mode,kind,oid) VALUES(?,?,?,?,?,?)",
                &[
                    Param::Integer(repo_id),
                    Param::Text(&commit),
                    Param::Text(&entry.path),
                    Param::Text(&entry.mode),
                    Param::Text(&entry.kind),
                    Param::Text(&entry.oid),
                ],
            )?;
            if entry.kind == "blob"
                && let Some((module, is_test)) = module_name(&entry.path, roots)
            {
                db.execute(
                    "INSERT OR IGNORE INTO module(repo_id,commit_oid,name,path,blob_oid,is_test) VALUES(?,?,?,?,?,?)",
                    &[
                        Param::Integer(repo_id),
                        Param::Text(&commit),
                        Param::Text(&module),
                        Param::Text(&entry.path),
                        Param::Text(&entry.oid),
                        Param::Integer(i64::from(is_test)),
                    ],
                )?;
            }
        }

        let stored_tree = db.scalar(
            "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
            &[Param::Integer(repo_id), Param::Text(&commit)],
        )?;
        if stored_tree != tree {
            return refuse("existing revision changed root tree");
        }
        let reconstructed = reconstruct_tree(db, repo_id, &commit)?;
        if reconstructed != tree {
            return refuse(format!(
                "Git/SQLite tree mismatch: expected {tree}, got {reconstructed}"
            ));
        }
        if include_semantics {
            crate::semantic::index_sources(db, repo_id, root, &commit, &entries, roots)?;
        }
        db.foreign_keys_valid()?;
        Ok(repo_id)
    })
}

fn register_repository(db: &Db, root: &Path, name: &str, provenance: &str) -> Result<i64> {
    db.execute(
        "INSERT OR IGNORE INTO repository(name,git_dir,provenance) VALUES(?,?,?)",
        &[
            Param::Text(name),
            Param::Text(&root.to_string_lossy()),
            Param::Text(provenance),
        ],
    )?;
    let repo_id = db
        .scalar(
            "SELECT id FROM repository WHERE name=?",
            &[Param::Text(name)],
        )?
        .parse::<i64>()
        .map_err(|err| err.to_string())?;

    // A registered name identifies one logical checkout. It must not be
    // silently rebound to a different repository on a later indexing call.
    let existing_root = db.scalar(
        "SELECT git_dir FROM repository WHERE id=?",
        &[Param::Integer(repo_id)],
    )?;
    let existing = std::fs::canonicalize(existing_root).map_err(|err| err.to_string())?;
    let current = std::fs::canonicalize(root).map_err(|err| err.to_string())?;
    if existing != current {
        return refuse(format!(
            "repository name {name} already registered for another checkout"
        ));
    }
    Ok(repo_id)
}

fn record_object(db: &Db, repo_id: i64, oid: &str, kind: &str) -> Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO git_object(repo_id,oid,kind) VALUES(?,?,?)",
        &[
            Param::Integer(repo_id),
            Param::Text(oid),
            Param::Text(kind),
        ],
    )
}

pub fn module_name(path: &str, roots: &[&str]) -> Option<(String, bool)> {
    if path.contains("/node_modules/") {
        return None;
    }
    for &root in roots {
        let prefix = format!("{}/", root.trim_end_matches('/').replace('.', "/"));
        if !path.starts_with(&prefix) {
            continue;
        }
        let stem = if let Some(p) = path.strip_suffix(".py") {
            p.to_owned()
        } else if let Some(p) = path.strip_suffix(".js") {
            p.to_owned()
        } else if let Some(p) = path.strip_suffix(".mjs") {
            p.to_owned() + ".mjs"
        } else if let Some(p) = path.strip_suffix(".cjs") {
            p.to_owned() + ".cjs"
        } else if let Some(p) = path.strip_suffix(".ts") {
            p.to_owned() + ".ts"
        } else {
            continue;
        };
        let module = stem.replace('/', ".");
        let name = module
            .strip_suffix(".__init__")
            .or_else(|| module.strip_suffix(".index"))
            .unwrap_or(&module);
        let is_test = path.starts_with("tests/")
            || path.starts_with("h11/tests/")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|p| p.starts_with("test_"));
        return Some((name.to_owned(), is_test));
    }
    None
}

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

pub type FileChange = (String, Option<String>, Option<String>);

pub fn changed_paths(db: &Db, repo: i64, old: &str, new: &str) -> Result<Vec<FileChange>> {
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    for (rev, into) in [(old, &mut a), (new, &mut b)] {
        for row in db.query(
            "SELECT path,oid FROM tree_entry WHERE repo_id=? AND commit_oid=? AND kind='blob'",
            &[Param::Integer(repo), Param::Text(rev)],
        )? {
            into.insert(
                row[0].clone().ok_or("path NULL")?,
                row[1].clone().ok_or("oid NULL")?,
            );
        }
    }
    let mut paths = BTreeSet::new();
    paths.extend(a.keys().cloned());
    paths.extend(b.keys().cloned());
    Ok(paths
        .into_iter()
        .filter_map(|p| {
            let x = a.get(&p).cloned();
            let y = b.get(&p).cloned();
            if x == y { None } else { Some((p, x, y)) }
        })
        .collect())
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
