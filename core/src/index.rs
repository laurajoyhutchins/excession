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
        db.execute("INSERT OR IGNORE INTO repository(name,git_dir,provenance) VALUES(?,?,?)",&[Param::Text(name),Param::Text(&root.to_string_lossy()),Param::Text(provenance)])?;
        let id=db.scalar("SELECT id FROM repository WHERE name=?",&[Param::Text(name)])?.parse::<i64>().map_err(|e|e.to_string())?;
        // A registered name is one logical checkout. Moving it requires a new name.
        let old_root=db.scalar("SELECT git_dir FROM repository WHERE id=?",&[Param::Integer(id)])?;
        if std::fs::canonicalize(&old_root).map_err(|e|e.to_string())?!=std::fs::canonicalize(root).map_err(|e|e.to_string())?{
            return refuse(format!("repository name {name} already registered for another checkout"));
        }
        for (oid,kind) in [(&commit,"commit"),(&tree,"tree")] {
            db.execute("INSERT OR IGNORE INTO git_object(repo_id,oid,kind) VALUES(?,?,?)",&[Param::Integer(id),Param::Text(oid),Param::Text(kind)])?;
        }
        db.execute("INSERT OR IGNORE INTO revision(repo_id,commit_oid,tree_oid) VALUES(?,?,?)",&[Param::Integer(id),Param::Text(&commit),Param::Text(&tree)])?;
        for e in &entries {
            db.execute("INSERT OR IGNORE INTO git_object(repo_id,oid,kind) VALUES(?,?,?)",&[Param::Integer(id),Param::Text(&e.oid),Param::Text(&e.kind)])?;
            db.execute("INSERT OR IGNORE INTO tree_entry(repo_id,commit_oid,path,mode,kind,oid) VALUES(?,?,?,?,?,?)",&[Param::Integer(id),Param::Text(&commit),Param::Text(&e.path),Param::Text(&e.mode),Param::Text(&e.kind),Param::Text(&e.oid)])?;
            if e.kind=="blob" && let Some((module,is_test))=module_name(&e.path,roots) {
                db.execute("INSERT OR IGNORE INTO module(repo_id,commit_oid,name,path,blob_oid,is_test) VALUES(?,?,?,?,?,?)",&[Param::Integer(id),Param::Text(&commit),Param::Text(&module),Param::Text(&e.path),Param::Text(&e.oid),Param::Integer(i64::from(is_test))])?;
            }
        }
        if db.scalar("SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",&[Param::Integer(id),Param::Text(&commit)])?!=tree{
            return refuse("existing revision changed root tree");
        }
        let computed=reconstruct_tree(db,id,&commit)?;
        if computed!=tree{return refuse(format!("Git/SQLite tree mismatch: expected {tree}, got {computed}"));}
        if include_semantics {
            crate::semantic::index_sources(db,id,root,&commit,&entries,roots)?;
        }
        db.foreign_keys_valid()?;
        Ok(id)
    })
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

pub fn reconstruct_tree(db: &Db, repo: i64, commit: &str) -> Result<String> {
    let expected = db.scalar(
        "SELECT tree_oid FROM revision WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo), Param::Text(commit)],
    )?;
    let rows = db.query(
        "SELECT path,mode,kind,oid FROM tree_entry WHERE repo_id=? AND commit_oid=? ORDER BY path",
        &[Param::Integer(repo), Param::Text(commit)],
    )?;
    let mut directories = BTreeSet::from([String::new()]);
    let mut leaves: BTreeMap<String, Vec<Node>> = BTreeMap::new();
    let mut registered: BTreeMap<String, String> = BTreeMap::new();
    let mut count = 0usize;
    for r in rows {
        let get = |i: usize| r[i].clone().ok_or("NULL tree entry");
        let path = get(0)?;
        let mode = get(1)?;
        let kind = get(2)?;
        let oid = get(3)?;
        git::validate_path(&path)?;
        git::oid(&oid)?;
        if registered.insert(path.clone(), kind.clone()).is_some() {
            return refuse("duplicate tree entry");
        }
        count += 1;
        if kind == "tree" {
            if mode != "040000" {
                return refuse("invalid directory mode");
            }
            directories.insert(path.clone());
        } else if !matches!(
            (kind.as_str(), mode.as_str()),
            ("blob", "100644" | "100755" | "120000") | ("commit", "160000")
        ) {
            return refuse("invalid indexed Git mode/kind");
        }
        let pieces = path.split('/').collect::<Vec<_>>();
        let parent = pieces[..pieces.len() - 1].join("/");
        leaves.entry(parent.clone()).or_default();
        directories.insert(parent);
        for k in 1..pieces.len() {
            directories.insert(pieces[..k].join("/"));
        }
        if kind != "tree" {
            leaves
                .entry(pieces[..pieces.len() - 1].join("/"))
                .or_default()
                .push(Node {
                    name: pieces[pieces.len() - 1].into(),
                    mode,
                    oid,
                    kind,
                });
        }
    }
    if count == 0 {
        // An empty Git tree is a valid tree: Git stores no entries.
        let actual = digest::git_tree(&[]);
        if actual != expected {
            return refuse("empty tree OID mismatch");
        }
        return Ok(actual);
    }
    for d in directories.iter().filter(|s| !s.is_empty()) {
        if registered.get(d).map(String::as_str) != Some("tree") {
            return refuse(format!("missing parent tree entry: {d}"));
        }
    }
    let mut sorted = directories.iter().cloned().collect::<Vec<_>>();
    sorted.sort_by_key(|p| {
        std::cmp::Reverse(if p.is_empty() {
            0
        } else {
            1 + p.bytes().filter(|b| *b == b'/').count()
        })
    });
    let mut computed: BTreeMap<String, String> = BTreeMap::new();
    for dir in sorted {
        let mut children = leaves.get(&dir).cloned().unwrap_or_default();
        for (child_path, child_oid) in &computed {
            let parent = child_path.rsplit_once('/').map(|x| x.0).unwrap_or("");
            if parent == dir {
                let name = child_path.rsplit('/').next().ok_or("bad child")?.to_owned();
                children.push(Node {
                    name,
                    mode: "40000".into(),
                    oid: child_oid.clone(),
                    kind: "tree".into(),
                });
            }
        }
        children.sort_by(|a, b| {
            let akey = if a.kind == "tree" {
                format!("{}/", a.name)
            } else {
                a.name.clone()
            };
            let bkey = if b.kind == "tree" {
                format!("{}/", b.name)
            } else {
                b.name.clone()
            };
            akey.as_bytes().cmp(bkey.as_bytes())
        });
        if children.windows(2).any(|p| p[0].name == p[1].name) {
            return refuse("duplicate tree basename");
        }
        let mut payload = Vec::new();
        for child in &children {
            payload.extend_from_slice(child.mode.trim_start_matches('0').as_bytes());
            payload.push(b' ');
            payload.extend_from_slice(child.name.as_bytes());
            payload.push(0);
            let raw = digest::unhex(&child.oid)?;
            if raw.len() != 20 {
                return refuse("invalid Git object byte length");
            }
            payload.extend(raw);
        }
        let actual = digest::git_tree(&payload);
        if let Some(stored) = registered.get(&dir) {
            if stored != "tree" {
                return refuse("directory collides with file");
            }
            let declared = db.scalar(
                "SELECT oid FROM tree_entry WHERE repo_id=? AND commit_oid=? AND path=?",
                &[Param::Integer(repo), Param::Text(commit), Param::Text(&dir)],
            )?;
            if actual != declared {
                return refuse(format!("subtree mismatch: {dir}"));
            }
        }
        computed.insert(dir, actual);
    }
    let root = computed.remove("").ok_or("missing computed root")?;
    if root != expected {
        return refuse(format!("root mismatch: {root} vs {expected}"));
    }
    Ok(root)
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
