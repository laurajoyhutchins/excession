//! SCIP-first source indexing. A SCIP import-role reference is evidence of a
//! syntactic import, not proof of runtime dependency or complete build closure.
//! Language parsing and symbol resolution belong to the external SCIP indexer.
use crate::json::{Value, obj};
use crate::scip_refactor::{Graph, load};
use crate::sqlite::{Db, Param};
use crate::{Result, index, refuse, views};
use std::collections::BTreeSet;
use std::path::Path;

const RELATION: &str = "scip:import-role";

fn is_test_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|p| matches!(p, "test" | "tests" | "__tests__"))
        || file.starts_with("test_")
        || file.ends_with("_test.go")
        || file.ends_with(".test.ts")
        || file.ends_with(".test.tsx")
        || file.ends_with("_test.rs")
}

fn import_edges(graph: &Graph) -> BTreeSet<(String, String, i64)> {
    graph
        .references
        .iter()
        .filter(|r| r.import_role && r.from != r.to)
        .map(|r| (r.from.clone(), r.to.clone(), (r.line + 1) as i64))
        .collect()
}

fn summary(graph: &Graph) -> Value {
    let edges = import_edges(graph);
    let reference_count = graph.references.len();
    obj(&[
        ("schema", Value::string("excession.scip-index.v1")),
        ("status", Value::string("SCIP_IMPORT_RELATIONS_INDEXED")),
        ("source_commit", Value::string(&graph.commit)),
        ("source_tree", Value::string(&graph.tree)),
        ("index_sha256", Value::string(&graph.index_sha256)),
        (
            "indexer",
            obj(&[
                ("name", Value::string(&graph.indexer_name)),
                ("version", Value::string(&graph.indexer_version)),
            ]),
        ),
        (
            "coverage",
            obj(&[
                ("indexed_documents", Value::number(graph.documents.len())),
                ("indexed_occurrences", Value::number(graph.occurrence_count)),
                (
                    "matched_cross_file_references",
                    Value::number(reference_count),
                ),
                ("indexed_import_role_edges", Value::number(edges.len())),
                (
                    "non_import_references_not_promoted",
                    Value::number(graph.references.iter().filter(|r| !r.import_role).count()),
                ),
                ("unresolved_references", Value::number(graph.unresolved)),
                (
                    "languages",
                    Value::array(graph.languages.iter().cloned().map(Value::string)),
                ),
            ]),
        ),
        (
            "module_identity",
            Value::string("repository-relative Git path"),
        ),
        (
            "test_identification",
            Value::string("conservative filename heuristic; not SCIP test coverage"),
        ),
        (
            "limitations",
            Value::string(
                "Only explicitly role-marked, source-bound, uniquely resolved cross-file SCIP imports become dependency rows. References without SCIP Import role, unresolved/external symbols, runtime imports, unindexed languages and build configurations are unknown. No Python/JS source parser runs in this mode.",
            ),
        ),
    ])
}

/// SCIP supplies all semantic observations for this index. The existing
/// Python/JS import and symbol extractor is deliberately NOT invoked.
pub fn import(
    db: &Db,
    root: &Path,
    name: &str,
    revision: &str,
    scip_index: &Path,
) -> Result<Value> {
    let graph = load(root, revision, scip_index)?;
    let repo_id = index::index_revision_git_only(db, root, name, &graph.commit, "scip-index")?;
    let edges = import_edges(&graph);
    db.transaction(|db| {
        let existing = db.scalar(
            "SELECT COUNT(*) FROM module WHERE repo_id=? AND commit_oid=?",
            &[Param::Integer(repo_id), Param::Text(&graph.commit)],
        )?;
        let already_indexed = db.scalar(
            "SELECT COUNT(*) FROM scip_index_metadata WHERE repo_id=? AND commit_oid=?",
            &[Param::Integer(repo_id), Param::Text(&graph.commit)],
        )?;
        if existing != "0" || already_indexed != "0" {
            return refuse("refusing to combine SCIP with an existing semantic index; choose a new repository name");
        }

        for doc in graph.documents.values() {
            db.execute(
                "INSERT INTO module(repo_id,commit_oid,name,path,blob_oid,is_test) VALUES(?,?,?,?,?,?)",
                &[
                    Param::Integer(repo_id), Param::Text(&graph.commit),
                    Param::Text(&doc.path), Param::Text(&doc.path),
                    Param::Text(&doc.blob), Param::Integer(i64::from(is_test_path(&doc.path))),
                ],
            )?;
        }
        for (from, to, line) in &edges {
            db.execute(
                "INSERT INTO dependency(repo_id,commit_oid,source_name,target_name,line,syntax) VALUES(?,?,?,?,?,?)",
                &[
                    Param::Integer(repo_id), Param::Text(&graph.commit),
                    Param::Text(from), Param::Text(to),
                    Param::Integer(*line), Param::Text(RELATION),
                ],
            )?;
        }
        db.execute(
            "INSERT INTO scip_index_metadata(repo_id,commit_oid,index_sha256,indexer_name,indexer_version,document_count,import_edge_count) VALUES(?,?,?,?,?,?,?)",
            &[
                Param::Integer(repo_id), Param::Text(&graph.commit),
                Param::Text(&graph.index_sha256), Param::Text(&graph.indexer_name),
                Param::Text(&graph.indexer_version), Param::Integer(graph.documents.len() as i64),
                Param::Integer(edges.len() as i64),
            ],
        )?;
        db.foreign_keys_valid()
    })?;
    Ok(summary(&graph))
}

fn required(row: &[Option<String>], column: usize) -> Result<&str> {
    row.get(column)
        .and_then(Option::as_deref)
        .ok_or_else(|| "missing SCIP SQL field".to_owned())
}

/// Recompute the expected indexed relationships from the same pinned SCIP
/// input; reject missing, forged, or altered SQL edges. This does not rerun
/// the external indexer or prove its coverage.
pub fn audit(db: &Db, root: &Path, name: &str, revision: &str, scip_index: &Path) -> Result<Value> {
    let graph = load(root, revision, scip_index)?;
    let repo_id = views::repo_id(db, name)?;
    let stored = db.scalar(
        "SELECT git_dir FROM repository WHERE id=?",
        &[Param::Integer(repo_id)],
    )?;
    let actual_root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let recorded_root = std::fs::canonicalize(stored).map_err(|e| e.to_string())?;
    if actual_root != recorded_root {
        return refuse("SCIP audit repository name is bound to another Git checkout");
    }
    let recorded_tree = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&graph.commit)],
    )?;
    if recorded_tree != graph.tree
        || index::reconstruct_tree(db, repo_id, &graph.commit)? != graph.tree
    {
        return refuse("SCIP indexed Git tree differs from the pinned revision");
    }
    let metadata = db.query(
        "SELECT index_sha256,indexer_name,indexer_version,document_count,import_edge_count FROM scip_index_metadata WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&graph.commit)],
    )?;
    if metadata.len() != 1 {
        return refuse("no SCIP import metadata for indexed revision");
    }
    let m = &metadata[0];
    if required(m, 0)? != graph.index_sha256
        || required(m, 1)? != graph.indexer_name
        || required(m, 2)? != graph.indexer_version
        || required(m, 3)? != graph.documents.len().to_string()
        || required(m, 4)? != import_edges(&graph).len().to_string()
    {
        return refuse("SCIP index provenance differs from supplied source evidence");
    }
    let modules = db.query(
        "SELECT name,path,blob_oid,is_test FROM module WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&graph.commit)],
    )?;
    if modules.len() != graph.documents.len() {
        return refuse("SCIP indexed document count differs");
    }
    for row in &modules {
        let path = required(row, 1)?;
        let doc = graph
            .documents
            .get(path)
            .ok_or("SCIP indexed module not present in source evidence")?;
        if required(row, 0)? != path
            || required(row, 2)? != doc.blob
            || required(row, 3)? != if is_test_path(path) { "1" } else { "0" }
        {
            return refuse("SCIP indexed module differs from source evidence");
        }
    }
    let actual = db.query(
        "SELECT source_name,target_name,line,syntax FROM dependency WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo_id), Param::Text(&graph.commit)],
    )?;
    let mut recorded = BTreeSet::new();
    for row in actual {
        if required(&row, 3)? != RELATION {
            return refuse("unexpected non-SCIP dependency mixed into indexed revision");
        }
        let line = required(&row, 2)?
            .parse::<i64>()
            .map_err(|_| "invalid SCIP indexed dependency line")?;
        recorded.insert((
            required(&row, 0)?.to_owned(),
            required(&row, 1)?.to_owned(),
            line,
        ));
    }
    if recorded != import_edges(&graph) {
        return refuse("SCIP indexed import edges differ from source evidence");
    }
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("status", Value::string("REPLAYED_SCIP_INDEXED_RELATIONS")),
        ("source_commit", Value::string(&graph.commit)),
        ("source_tree", Value::string(&graph.tree)),
        ("index_sha256", Value::string(&graph.index_sha256)),
        ("semantic_indexer_rerun", Value::Bool(false)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_identity_is_heuristic_only() {
        assert!(is_test_path("pkg/tests/test_math.py"));
        assert!(is_test_path("src/api_test.go"));
        assert!(!is_test_path("src/business.rs"));
        assert!(!is_test_path("src/latest.ts"));
    }
}
