//! End-user entry point over the existing Git/SQLite static indexer.
//! These are *candidate* dependencies, not complete dynamic semantics.
use crate::discovery::{self, Discovery};
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, doctor, index, refuse, views};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

struct Module {
    path: String,
    blob: String,
    mode: String,
}

fn source(discovered: &Discovery, module: &Module) -> Value {
    obj(&[
        ("source_commit", Value::string(&discovered.commit)),
        ("source_tree", Value::string(&discovered.tree)),
        ("path", Value::string(&module.path)),
        ("blob", Value::string(&module.blob)),
        ("mode", Value::string(&module.mode)),
    ])
}

fn modules(db: &Db, repo_id: i64, commit: &str) -> Result<BTreeMap<String, Module>> {
    let rows = db.query(
        "SELECT m.name,m.path,m.blob_oid,t.mode FROM module m
         JOIN tree_entry t ON m.repo_id=t.repo_id AND m.commit_oid=t.commit_oid
              AND m.path=t.path AND m.blob_oid=t.oid
         WHERE m.repo_id=? AND m.commit_oid=? ORDER BY m.name",
        &[Param::Integer(repo_id), Param::Text(commit)],
    )?;
    let mut modules = BTreeMap::new();
    for row in rows {
        let get = |n: usize| -> Result<String> {
            row.get(n)
                .and_then(|v| v.as_ref())
                .cloned()
                .ok_or_else(|| format!("invalid module catalog column {n}"))
        };
        let name = get(0)?;
        if modules
            .insert(
                name,
                Module {
                    path: get(1)?,
                    blob: get(2)?,
                    mode: get(3)?,
                },
            )
            .is_some()
        {
            return refuse("duplicate indexed module");
        }
    }
    Ok(modules)
}

/// Validate the database destination before creating any directory.
///
/// A lexical starts_with comparison is insufficient: \`..\` components or
/// existing symlinks can route a seemingly external cache path back into
/// the source checkout. Resolve the nearest existing ancestor, then append
/// only normal missing path components. The read-only source boundary is
/// enforced before Db::open can create or modify anything.
fn checked_cache_destination(source_root: &Path, destination: &Path) -> Result<PathBuf> {
    if !destination.is_absolute()
        || destination
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return refuse(
            "SQLite destination must be an absolute, normalized path outside the source repository",
        );
    }
    let canonical_root = source_root
        .canonicalize()
        .map_err(|e| format!("resolve source checkout: {e}"))?;
    let mut ancestor = destination
        .parent()
        .ok_or("SQLite destination has no parent")?;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or("cannot resolve SQLite destination parent")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("cannot resolve SQLite destination ancestor")?;
            }
            Err(error) => return Err(format!("inspect SQLite destination parent: {error}")),
        }
    }
    let mut resolved_parent = ancestor
        .canonicalize()
        .map_err(|e| format!("resolve SQLite destination parent: {e}"))?;
    if !resolved_parent.is_dir() {
        return refuse("SQLite destination parent is not a directory");
    }
    for component in missing.into_iter().rev() {
        resolved_parent.push(component);
    }
    if resolved_parent.starts_with(&canonical_root) {
        return refuse("SQLite destination resolves inside the source repository");
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return refuse("SQLite destination cannot be a symbolic link");
        }
        Ok(metadata) if !metadata.is_file() => {
            return refuse("SQLite destination is not a regular file");
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect SQLite destination: {error}")),
    }
    Ok(destination.to_path_buf())
}

/// Capability preflight must follow the selected roots, not every language
/// found elsewhere in the repository. Only explicit overrides require a
/// second Git tree listing; the auto-discovered case reuses its inventory.
fn scoped_languages(
    discovered: &Discovery,
    roots: &[String],
    explicit: bool,
) -> Result<BTreeSet<&'static str>> {
    if !explicit {
        return Ok(discovered.language_counts.keys().copied().collect());
    }
    let roots: Vec<&str> = roots.iter().map(String::as_str).collect();
    Ok(crate::git::entries(&discovered.root, &discovered.commit)?
        .into_iter()
        .filter(|entry| {
            entry.kind == "blob"
                && matches!(entry.mode.as_str(), "100644" | "100755")
                && index::module_name(&entry.path, &roots).is_some()
        })
        .filter_map(|entry| {
            if entry.path.ends_with(".py") {
                Some("python")
            } else if [".js", ".cjs", ".mjs", ".ts"]
                .iter()
                .any(|extension| entry.path.ends_with(extension))
            {
                Some("javascript")
            } else {
                None
            }
        })
        .collect())
}

/// User-facing analysis requires at least one discovered supported source root.
pub fn run(
    path: &Path,
    rev: &str,
    explicit_roots: Option<&str>,
    db_override: Option<&str>,
) -> Result<(Value, PathBuf)> {
    run_if_supported(path, rev, explicit_roots, db_override)?.ok_or_else(|| {
        "no supported Python/JavaScript source directories detected; provide --roots or use configured native graph commands".to_owned()
    })
}

/// Impact analysis may investigate a revision without supported roots.
/// Treat this as an explicit absence of a static graph, not an analyzer failure.
pub fn run_if_supported(
    path: &Path,
    rev: &str,
    explicit_roots: Option<&str>,
    db_override: Option<&str>,
) -> Result<Option<(Value, PathBuf)>> {
    let discovered = discovery::inspect(path, rev)?;
    let roots: Vec<String> = if let Some(spec) = explicit_roots {
        let roots: Vec<String> = spec
            .split(',')
            .map(str::to_owned)
            .filter(|x| !x.is_empty())
            .collect();
        if roots.is_empty() {
            return refuse("at least one source root is required");
        }
        for root in &roots {
            crate::git::validate_path(root)?;
        }
        roots
    } else {
        discovered.roots.clone()
    };
    if roots.is_empty() {
        return Ok(None);
    }
    // Revisions indexed with different module roots must never silently share
    // a module catalog. The roots digest is part of the logical index name.
    let mut normalized_roots = roots;
    normalized_roots.sort();
    normalized_roots.dedup();
    let languages = scoped_languages(&discovered, &normalized_roots, explicit_roots.is_some())?;
    let capabilities = doctor::report();
    let cap = capabilities.get("capabilities")?;
    for (lang, key) in [
        ("python", "python_index"),
        ("javascript", "javascript_index"),
    ] {
        if languages.contains(lang) && !cap.get(key)?.bool()? {
            return refuse(format!(
                "required {lang} analyzer unavailable; run excession doctor --json"
            ));
        }
    }
    let root_spec = normalized_roots.join(",");
    let digest = digest::sha256(root_spec.as_bytes());
    let project = discovered
        .root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repository");
    let name = format!("{project}-{}", &digest[..12]);
    let destination = db_override
        .map(PathBuf::from)
        .unwrap_or_else(|| discovered.index_path.clone());
    let db_path = checked_cache_destination(&discovered.root, &destination)?;
    // No directory is created until all path and symlink checks have passed.
    let parent = db_path.parent().ok_or("SQLite destination has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("create cache directory: {e}"))?;
    let db = Db::open(&db_path)?;
    let roots_borrowed: Vec<&str> = normalized_roots.iter().map(String::as_str).collect();
    let repo_id = index::index_revision_with_roots(
        &db,
        &discovered.root,
        &name,
        &discovered.commit,
        "git-object-static-candidates",
        &roots_borrowed,
    )?;
    index::reconstruct_tree(&db, repo_id, &discovered.commit)?;
    let catalog = modules(&db, repo_id, &discovered.commit)?;
    let raw = views::dependencies(&db, &name, &discovered.commit)?;
    let mut relationships = Vec::new();
    let mut receipt_ids = BTreeSet::new();
    for item in raw.array_ref()? {
        let from = item.get("source_name")?.str()?;
        let to = item.get("target_name")?.str()?;
        let source_module = catalog.get(from).ok_or("missing indexed source module")?;
        let target_module = catalog.get(to).ok_or("missing indexed target module")?;
        let from_source = source(&discovered, source_module);
        let to_source = source(&discovered, target_module);
        let observation = obj(&[
            ("source", from_source.clone()),
            ("target", to_source.clone()),
            ("line", item.get("line")?.clone()),
            ("syntax", item.get("syntax")?.clone()),
            ("claim", Value::string("STATIC_IMPORT_CANDIDATE")),
        ]);
        let receipt_id = observation.sha256();
        receipt_ids.insert(receipt_id.clone());
        relationships.push(obj(&[
            ("source", from_source.clone()),
            ("target", to_source),
            ("kind", Value::string("STATIC_IMPORT")),
            (
                "evidence",
                Value::array([obj(&[
                    ("receipt_id", Value::string(receipt_id)),
                    ("producer", Value::string("static-module-index")),
                    ("observation_kind", Value::string("STATIC_IMPORT_CANDIDATE")),
                    ("assessment", Value::string("CANDIDATE")),
                    ("source", from_source),
                ])]),
            ),
        ]));
    }
    let mut gaps = discovered
        .report()?
        .get("coverage")?
        .get("gaps")?
        .array_ref()?
        .to_vec();
    for row in db.query(
        "SELECT module_name,line,reason FROM hazard
         WHERE repo_id=? AND commit_oid=? ORDER BY module_name,line,reason LIMIT 9999",
        &[Param::Integer(repo_id), Param::Text(&discovered.commit)],
    )? {
        let name = row
            .first()
            .and_then(|x| x.as_deref())
            .ok_or("hazard module missing")?;
        let reason = row
            .get(2)
            .and_then(|x| x.as_deref())
            .ok_or("hazard reason missing")?;
        let path = catalog.get(name).ok_or("hazard source module missing")?;
        gaps.push(obj(&[
            ("reason", Value::string("UNRESOLVED_RELATIONSHIP")),
            ("path", Value::string(&path.path)),
            (
                "detail",
                Value::string(reason.chars().take(1024).collect::<String>()),
            ),
        ]));
    }
    if gaps.len() > 10000 {
        gaps.truncate(9999);
        gaps.push(obj(&[("reason", Value::string("TRUNCATED"))]));
    }
    let mut report = obj(&[
        ("schema", Value::string("excession.analysis.v1")),
        ("source_commit", Value::string(&discovered.commit)),
        ("source_tree", Value::string(&discovered.tree)),
        ("status", Value::string("PARTIAL")),
        (
            "observation_receipts",
            Value::array(receipt_ids.into_iter().map(Value::string)),
        ),
        ("relationships", Value::array(relationships)),
        (
            "coverage",
            obj(&[
                ("status", Value::string("PARTIAL")),
                ("analyzed_sources", Value::number(catalog.len())),
                ("gaps", Value::array(gaps)),
            ]),
        ),
    ]);
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok(Some((report, db_path)))
}

pub fn human(report: &Value, db_path: &Path) -> Result<String> {
    let count = report.get("relationships")?.array_ref()?.len();
    let modules = report.get("coverage")?.get("analyzed_sources")?.integer()?;
    let gaps = report.get("coverage")?.get("gaps")?.array_ref()?.len();
    Ok(format!(
        "Excession indexed {modules} supported modules, {count} static candidate relationships.\n\
         Source revision: {}\nUnresolved/excluded observations: {gaps}\n\
         SQLite evidence: {}\nCoverage: PARTIAL (absence of an edge is not absence of a dependency)\n\
         Use --json for a versioned report.",
        report.get("source_commit")?.str()?,
        db_path.display()
    ))
}
