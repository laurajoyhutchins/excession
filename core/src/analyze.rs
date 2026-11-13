//! End-user entry point over the existing Git/SQLite static indexer.
//! These are *candidate* dependencies, not complete dynamic semantics.
use crate::discovery::{self, Discovery};
use crate::json::{Value, obj};
use crate::observed_relations::{self, Observed, Plan, Selection};
use crate::source_capabilities;
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

/// Validate an external output destination before creating anything.
/// Reject relative paths, traversal and symlinked parents that route back into
/// the checkout. The same boundary applies to caches and exported reports.
pub fn checked_external_destination(
    source_root: &Path,
    destination: &Path,
    label: &str,
) -> Result<PathBuf> {
    if !destination.is_absolute()
        || destination
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return refuse(format!(
            "{label} destination must be an absolute, normalized path outside the source repository"
        ));
    }
    let canonical_root = source_root
        .canonicalize()
        .map_err(|e| format!("resolve source checkout: {e}"))?;
    let mut ancestor = destination
        .parent()
        .ok_or_else(|| format!("{label} destination has no parent"))?;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| format!("cannot resolve {label} destination parent"))?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| format!("cannot resolve {label} destination ancestor"))?;
            }
            Err(error) => return Err(format!("inspect {label} destination parent: {error}")),
        }
    }
    let mut resolved_parent = ancestor
        .canonicalize()
        .map_err(|e| format!("resolve {label} destination parent: {e}"))?;
    if !resolved_parent.is_dir() {
        return refuse(format!("{label} destination parent is not a directory"));
    }
    for component in missing.into_iter().rev() {
        resolved_parent.push(component);
    }
    if resolved_parent.starts_with(&canonical_root) {
        return refuse(format!(
            "{label} destination resolves inside the source repository"
        ));
    }
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return refuse(format!("{label} destination cannot be a symbolic link"));
        }
        Ok(metadata) if !metadata.is_file() => {
            return refuse(format!("{label} destination is not a regular file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect {label} destination: {error}")),
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
            source_capabilities::static_module_producer(&entry.path).map(|producer| producer.name)
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
            if root != "." {
                crate::git::validate_path(root)?;
            }
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
    for producer in source_capabilities::STATIC_MODULE_PRODUCERS {
        if languages.contains(producer.name) && !cap.get(producer.capability)?.bool()? {
            return refuse(format!(
                "required {} analyzer unavailable; run excession doctor --json",
                producer.name
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
    let db_path = checked_external_destination(&discovered.root, &destination, "SQLite")?;
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

/// Compose already authenticated native observations with native static
/// candidates. This is the single report assembly point for explicit and
/// default selection, so coverage and receipt handling cannot drift.
struct Compose<'a> {
    path: &'a Path,
    rev: &'a str,
    roots: Option<&'a str>,
    database: Option<&'a str>,
    discovered: &'a Discovery,
    selected: Observed,
    selection_key: &'a str,
    selection_record: Value,
    native_gaps: &'a [Value],
}

fn compose(request: Compose<'_>) -> Result<(Value, PathBuf)> {
    let Compose {
        path, rev, roots, database, discovered, selected,
        selection_key, selection_record, native_gaps,
    } = request;
    let db_path = checked_external_destination(
        &discovered.root,
        database.map(Path::new).unwrap_or(&discovered.index_path),
        "SQLite",
    )?;
    let (mut report, _) = match run_if_supported(path, rev, roots, database)? {
        Some(current) => current,
        None => {
            let gaps = discovered.report()?.get("coverage")?.get("gaps")?.clone();
            (
                obj(&[
                    ("schema", Value::string("excession.analysis.v1")),
                    ("source_commit", Value::string(&discovered.commit)),
                    ("source_tree", Value::string(&discovered.tree)),
                    ("status", Value::string("PARTIAL")),
                    ("observation_receipts", Value::array([])),
                    ("relationships", Value::array([])),
                    (
                        "coverage",
                        obj(&[
                            ("status", Value::string("PARTIAL")),
                            ("analyzed_sources", Value::number(0)),
                            ("gaps", gaps),
                        ]),
                    ),
                ]),
                db_path.clone(),
            )
        }
    };
    let mut relations = report.get("relationships")?.array_ref()?.to_vec();
    relations.extend(selected.relationships);
    let mut receipts = report
        .get("observation_receipts")?
        .array_ref()?
        .iter()
        .map(|value| Ok(value.str()?.to_owned()))
        .collect::<Result<BTreeSet<_>>>()?;
    receipts.extend(selected.receipt_ids);
    let covered = usize::try_from(report.get("coverage")?.get("analyzed_sources")?.integer()?)
        .map_err(|_| "invalid indexed source count")?;
    let count = covered.max(selected.source_paths.len());
    let coverage = report
        .entries_mut()?
        .get_mut("coverage")
        .ok_or("analysis report missing coverage")?;
    let gaps = coverage.get("gaps")?.array_ref()?.to_vec();
    let mut gaps = [gaps, native_gaps.to_vec()].concat();
    if gaps.len() > 10000 {
        gaps.truncate(9999);
        gaps.push(obj(&[("reason", Value::string("TRUNCATED"))]));
    }
    coverage
        .entries_mut()?
        .insert("gaps".into(), Value::array(gaps));
    coverage
        .entries_mut()?
        .insert("analyzed_sources".into(), Value::number(count));
    let body = report.entries_mut()?;
    body.insert("relationships".into(), Value::array(relations));
    body.insert(
        "observation_receipts".into(),
        Value::array(receipts.into_iter().map(Value::string)),
    );
    body.insert(
        "configured_observations".into(),
        Value::array(selected.configured_observations),
    );
    body.insert("producer_coverage".into(), Value::array(selected.coverage));
    body.insert(selection_key.into(), selection_record);
    body.remove("report_id");
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".into(), Value::string(report_id));
    Ok((report, db_path))
}

/// Explicit identities continue to narrow source evidence, even if multiple
/// other snapshots are present in the store. Do not automatically add anything
/// to this explicit scope.
pub fn run_with_evidence(
    path: &Path,
    rev: &str,
    roots: Option<&str>,
    db_override: Option<&str>,
    selection: Selection<'_>,
) -> Result<(Value, PathBuf)> {
    if selection.is_empty() && selection.source_prefix.is_some() {
        return refuse("--source-prefix requires --scip-snapshot");
    }
    let discovered = discovery::inspect(path, rev)?;
    let selected = if selection.is_empty() {
        Observed::default()
    } else {
        let database = db_override.ok_or("selected native evidence requires --db")?;
        let dest = checked_external_destination(&discovered.root, Path::new(database), "SQLite")?;
        if !dest.is_file() {
            return refuse("selected source evidence database does not exist");
        }
        observed_relations::read(
            &Db::open(&dest)?,
            &discovered.root,
            &discovered.commit,
            selection,
        )?
    };
    compose(Compose {
        path,
        rev,
        roots,
        database: db_override,
        discovered: &discovered,
        selected,
        selection_key: "producer_selections",
        selection_record: obj(&[
            ("scip_snapshot", selection.scip_snapshot.map_or(Value::Null, Value::string)),
            ("graph_id", selection.graph_id.map_or(Value::Null, Value::string)),
            ("source_prefix", selection.source_prefix.map_or(Value::Null, Value::string)),
        ]),
        native_gaps: &[],
    })
}

/// Generate an analysis from an immutable, already selected producer plan.
pub fn run_with_plan(
    path: &Path,
    rev: &str,
    roots: Option<&str>,
    db_override: Option<&str>,
    plan: &Plan,
) -> Result<(Value, PathBuf)> {
    let discovered = discovery::inspect(path, rev)?;
    let selected = if plan.is_empty() {
        Observed::default()
    } else {
        let dest = checked_external_destination(
            &discovered.root,
            db_override.map(Path::new).unwrap_or(&discovered.index_path),
            "SQLite",
        )?;
        if !dest.is_file() {
            return refuse("pinned native evidence database does not exist");
        }
        observed_relations::read_plan(
            &Db::open(&dest)?,
            &discovered.root,
            &discovered.commit,
            plan,
        )?
    };
    compose(Compose {
        path,
        rev,
        roots,
        database: db_override,
        discovered: &discovered,
        selected,
        selection_key: "producer_selection_plan",
        selection_record: plan.document(),
        native_gaps: &plan.gaps,
    })
}

/// Default: select everything compatible from the existing evidence store.
/// If there is no store, preserve existing static analysis and report its
/// unsupported-language coverage rather than treating discovery as an opt-in.
pub fn run_default(
    path: &Path,
    rev: &str,
    roots: Option<&str>,
    db_override: Option<&str>,
) -> Result<(Value, PathBuf)> {
    let discovered = discovery::inspect(path, rev)?;
    let dest = checked_external_destination(
        &discovered.root,
        db_override.map(Path::new).unwrap_or(&discovered.index_path),
        "SQLite",
    )?;
    let plan = if dest.is_file() {
        observed_relations::default_plan(&Db::open(&dest)?, &discovered.root, &discovered.commit)?
    } else {
        Plan::default()
    };
    run_with_plan(path, rev, roots, db_override, &plan)
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
