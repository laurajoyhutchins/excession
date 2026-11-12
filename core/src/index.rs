//! A rebuildable Git tree index. Tree hashes are computed independently from SQL rows.
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use crate::git;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// Compatibility path for existing index consumers.
pub use crate::module_identity::{module_name, module_name_with_import_base};

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
#[derive(Clone, Copy)]
enum SemanticMode<'a> {
    GitOnly,
    Parse { import_base: Option<&'a str> },
}

pub fn index_revision_for_evidence(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
) -> Result<i64> {
    index_revision_inner(
        db,
        root,
        name,
        commit,
        provenance,
        &["h11", "tests"],
        SemanticMode::GitOnly,
    )
}

pub fn index_revision_with_roots(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
    roots: &[&str],
) -> Result<i64> {
    index_revision_inner(
        db,
        root,
        name,
        commit,
        provenance,
        roots,
        SemanticMode::Parse { import_base: None },
    )
}

/// Resolve imports relative to a declared source directory without changing
/// physical Git paths, file modes, or blob identities.
pub fn index_revision_with_import_base(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
    roots: &[&str],
    import_base: &str,
) -> Result<i64> {
    git::validate_path(import_base)?;
    let prefix = format!("{import_base}/");
    if roots
        .iter()
        .any(|root| *root != import_base && !root.starts_with(&prefix))
    {
        return refuse("every module root must be inside --import-base");
    }
    index_revision_inner(
        db,
        root,
        name,
        commit,
        provenance,
        roots,
        SemanticMode::Parse {
            import_base: Some(import_base),
        },
    )
}

fn index_revision_inner(
    db: &Db,
    root: &Path,
    name: &str,
    commit: &str,
    provenance: &str,
    roots: &[&str],
    semantics: SemanticMode<'_>,
) -> Result<i64> {
    let import_base = match semantics {
        SemanticMode::GitOnly => None,
        SemanticMode::Parse { import_base } => import_base,
    };
    // Committed revision reads use immutable Git objects via BlobReader.
    // A dirty working tree cannot change their meaning; trusted-local
    // execution paths retain their own stricter source policies.
    git::verify_repo(root)?;
    let commit = git::rev(root, commit)?;
    if git::text(root, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("Git reference is not a commit");
    }
    let tree = git::rev(root, &format!("{commit}^{{tree}}"))?;
    let entries = git::entries(root, &commit)?;
    // Plan reuse and run language-native adapters before acquiring the writer
    // lock. Only Git-authenticated completed Python observations may be carried.
    db.init_schema()?;
    let python_plan = match semantics {
        SemanticMode::GitOnly => None,
        SemanticMode::Parse { import_base } => {
            crate::incremental_python::plan(db, root, name, &commit, &entries, roots, import_base)?
        }
    };
    let prepared = match semantics {
        SemanticMode::GitOnly => None,
        SemanticMode::Parse { import_base } => Some(crate::semantic::prepare_sources(
            root,
            &entries,
            roots,
            import_base,
            python_plan.as_ref().map(|plan| crate::semantic::ReuseSelection {
                paths: &plan.reused,
                total: plan.total,
            }),
        )?),
    };

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
                && let Some((module, is_test)) = module_name_with_import_base(&entry.path, roots, import_base)
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
        if let Some(prepared) = &prepared {
            // The Git revision is immutable, but the analyzer implementation
            // is not. Replacing derived rows on *every* analysis prevents
            // obsolete imports, symbols and hazards surviving a parser fix.
            // The only permitted reuse copies authenticated rows from the
            // original relational store; no second cache authority is added.
            for table in ["dependency", "symbol", "api_binding", "hazard"] {
                db.execute(
                    &format!("DELETE FROM {table} WHERE repo_id=? AND commit_oid=?"),
                    &[Param::Integer(repo_id), Param::Text(&commit)],
                )?;
            }
            if let Some(plan) = &python_plan {
                crate::incremental_python::carry(db, root, repo_id, &commit, plan)?;
            }
            crate::semantic::index_prepared(db, repo_id, &commit, prepared)?;
            if let Some(plan) = &python_plan {
                crate::incremental_python::finish(db, repo_id, &commit, plan)?;
            }
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
        &[Param::Integer(repo_id), Param::Text(oid), Param::Text(kind)],
    )
}

// Existing public API remains available to downstream index consumers.
pub use crate::indexed_tree::reconstruct_tree;

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
mod import_base_tests {
    use super::*;

    #[test]
    fn whole_repository_root_includes_top_level_modules_but_not_dependencies() {
        assert_eq!(module_name("app.py", &["."]).unwrap().0, "app");
        assert_eq!(module_name("pkg/a.py", &["."]).unwrap().0, "pkg.a");
        assert!(module_name("node_modules/tool.js", &["."]).is_none());
        assert!(module_name(".venv/lib/tool.py", &["."]).is_none());
    }

    #[test]
    fn git_paths_remain_distinct_from_python_import_names() {
        let path = "scripts/gdb/linux/tasks.py";
        let roots = &["scripts/gdb"];
        assert_eq!(
            module_name(path, roots).unwrap().0,
            "scripts.gdb.linux.tasks"
        );
        assert_eq!(
            module_name_with_import_base(path, roots, Some("scripts/gdb"))
                .unwrap()
                .0,
            "linux.tasks"
        );
        assert_eq!(
            module_name_with_import_base(
                "scripts/gdb/linux/__init__.py",
                roots,
                Some("scripts/gdb")
            )
            .unwrap()
            .0,
            "linux"
        );
        assert!(
            module_name_with_import_base("scripts/other/task.py", roots, Some("scripts/gdb"))
                .is_none()
        );
    }
}
