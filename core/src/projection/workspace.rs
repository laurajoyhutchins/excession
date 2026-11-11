//! Read-only navigation and bounded materialization of verified Git view manifests.
//! Materialized files are not executable checkouts.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn get_entry<'a>(plan: &'a Plan, virtual_path: &str) -> Result<&'a Entry> {
    git::validate_path(virtual_path)?;
    plan.entries
        .binary_search_by(|entry| entry.virtual_path.as_str().cmp(virtual_path))
        .map(|index| &plan.entries[index])
        .map_err(|_| format!("no virtual file at path: {virtual_path}"))
}

/// Expose the path/source binding without requiring a filesystem projection.
pub fn describe(repo: &Path, manifest: &Path, virtual_path: &str) -> Result<Value> {
    let plan = checked_plan(repo, manifest)?;
    let entry = get_entry(&plan, virtual_path)?;
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("source_commit", Value::string(&plan.commit)),
        ("entry", entry.to_json()),
        ("status", Value::string("PINNED_GIT_SOURCE_NOT_EXECUTABLE")),
    ]))
}

/// List one level of the virtual hierarchy. Listing verifies the entire
/// manifest, not only the item requested by a potentially stale client.
pub fn list(repo: &Path, manifest: &Path, prefix: &str) -> Result<Value> {
    if !prefix.is_empty() {
        git::validate_path(prefix)?;
    }
    let plan = checked_plan(repo, manifest)?;
    let mut children = BTreeMap::<String, (bool, usize)>::new();
    let mut found = prefix.is_empty();
    let subtree_prefix = format!("{prefix}/");
    for e in &plan.entries {
        let remainder = if prefix.is_empty() {
            e.virtual_path.as_str()
        } else if let Some(rest) = e.virtual_path.strip_prefix(&subtree_prefix) {
            found = true;
            rest
        } else {
            continue;
        };
        let (name, directory) = if let Some((head, _)) = remainder.split_once('/') {
            (head, true)
        } else {
            (remainder, false)
        };
        let item = children.entry(name.to_owned()).or_insert((directory, 0));
        if item.0 != directory {
            return refuse("virtual file/directory name collision");
        }
        item.1 += 1;
    }
    if !found {
        return refuse("unknown virtual directory");
    }
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("directory", Value::string(prefix)),
        (
            "children",
            Value::array(
                children
                    .into_iter()
                    .map(|(name, (dir, count))| {
                        obj(&[
                            ("name", Value::string(name)),
                            (
                                "type",
                                Value::string(if dir { "directory" } else { "file" }),
                            ),
                            ("references", Value::number(count)),
                        ])
                    })
                    .collect::<Vec<_>>(),
            ),
        ),
    ]))
}

/// Read exactly one immutable Git blob on demand, without materialization.
/// The CLI emits the raw bytes to stdout; all metadata goes through describe.
pub fn open(repo: &Path, manifest: &Path, virtual_path: &str) -> Result<Vec<u8>> {
    let plan = checked_plan(repo, manifest)?;
    read_blob(repo, get_entry(&plan, virtual_path)?)
}

/// Check that a previously generated manifest is still the complete output of the
/// exact indexed query, not merely a collection of valid Git blob references.
/// For security runs the scanner report must also be independently audited via
/// `security-ingest-audit` before trusting the unverified flow relationships.
pub fn replay(expected: &Value, manifest: &Path) -> Result<Value> {
    let provided = read_plan(manifest)?;
    if expected.canonical() != provided.manifest().canonical() {
        return refuse("projection is not the complete deterministic query result");
    }
    Ok(obj(&[
        ("view_id", Value::string(provided.payload().sha256())),
        ("entries_replayed", Value::number(provided.entries.len())),
        ("status", Value::string("QUERY_REPLAY_MATCHED")),
    ]))
}

fn verify_parent(repo: &Path, output: &Path) -> Result<()> {
    if !output.is_absolute() {
        return refuse("view output must be an absolute path");
    }
    let parent = output
        .parent()
        .ok_or("view output must have a parent directory")?
        .canonicalize()
        .map_err(|e| format!("output parent must already exist: {e}"))?;
    let root = repo.canonicalize().map_err(|e| e.to_string())?;
    if parent.starts_with(&root) {
        return refuse("view output must be outside source checkout");
    }
    if !parent.is_dir() {
        return refuse("output parent is not a directory");
    }
    Ok(())
}

fn read_blob(repo: &Path, e: &Entry) -> Result<Vec<u8>> {
    let raw = git::command(repo, &["cat-file", "blob", &e.blob])?;
    if raw.len() > MAX_BLOB_BYTES {
        return refuse("source blob larger than 16 MiB cap");
    }
    if digest::git_blob(&raw) != e.blob {
        return refuse("Git source blob content has invalid OID");
    }
    Ok(raw)
}

pub fn materialize(repo: &Path, manifest: &Path, target: &Path) -> Result<Value> {
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    verify_parent(repo, target)?;
    if target.exists() || fs::symlink_metadata(target).is_ok() {
        return refuse("view destination already exists");
    }
    // Validate/read all content before creating output; limit total bytes including repeated refs.
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for e in &plan.entries {
        let data = read_blob(repo, e)?;
        total = total.checked_add(data.len()).ok_or("view size overflow")?;
        if total > MAX_TOTAL_BYTES {
            return refuse("view exceeds 128 MiB copy cap");
        }
        bytes.push(data);
    }
    fs::create_dir(target).map_err(|e| e.to_string())?;
    let result = (|| {
        for (e, data) in plan.entries.iter().zip(bytes.iter()) {
            let path = target.join(&e.virtual_path);
            fs::create_dir_all(path.parent().ok_or("invalid view path")?)
                .map_err(|e| e.to_string())?;
            fs::write(&path, data).map_err(|e| e.to_string())?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
                .map_err(|e| e.to_string())?;
        }
        let receipt = plan.manifest().canonical();
        fs::write(target.join(MANIFEST_NAME), receipt.as_bytes()).map_err(|e| e.to_string())?;
        audit(repo, manifest, target)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(target);
    }
    result
}

fn walk(root: &Path, prefix: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let typ = entry.file_type().map_err(|e| e.to_string())?;
        if typ.is_symlink() {
            return refuse("symbolic link in materialized view");
        }
        if typ.is_dir() {
            walk(&entry.path(), prefix, output)?;
        } else if typ.is_file() {
            let entry_path = entry.path();
            let rel = entry_path.strip_prefix(prefix).map_err(|e| e.to_string())?;
            let name = rel
                .to_str()
                .ok_or("non-UTF8 output filename")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !output.insert(name) {
                return refuse("duplicate output file");
            }
        } else {
            return refuse("unexpected device or special file in view");
        }
    }
    Ok(())
}

pub fn audit(repo: &Path, manifest: &Path, target: &Path) -> Result<Value> {
    if !target
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return refuse("view root must be an actual directory, not a symlink");
    }
    let plan = read_plan(manifest)?;
    authenticate(&plan, repo)?;
    let receipt = fs::read(target.join(MANIFEST_NAME)).map_err(|e| e.to_string())?;
    if receipt != plan.manifest().canonical().as_bytes() {
        return refuse("view receipt mismatch");
    }
    let mut observed = BTreeSet::new();
    walk(target, target, &mut observed)?;
    let expected = plan
        .entries
        .iter()
        .map(|e| e.virtual_path.clone())
        .chain(std::iter::once(MANIFEST_NAME.into()))
        .collect::<BTreeSet<_>>();
    if observed != expected {
        return refuse("view contains extra or missing files");
    }
    let mut total = 0usize;
    for entry in &plan.entries {
        let contents = fs::read(target.join(&entry.virtual_path)).map_err(|e| e.to_string())?;
        if contents.len() > MAX_BLOB_BYTES {
            return refuse("view output exceeds blob cap");
        }
        total = total
            .checked_add(contents.len())
            .ok_or("view byte count overflow")?;
        if total > MAX_TOTAL_BYTES {
            return refuse("view output exceeds total cap");
        }
        if digest::git_blob(&contents) != entry.blob {
            return refuse("view contents differ from source blob");
        }
    }
    Ok(obj(&[
        ("view_id", Value::string(plan.payload().sha256())),
        ("kind", Value::string(plan.kind)),
        ("source_commit", Value::string(plan.commit)),
        ("entries_verified", Value::number(plan.entries.len())),
        ("bytes_verified", Value::number(total)),
        (
            "status",
            Value::string("SOURCE_BLOBS_VERIFIED_NOT_EXECUTABLE"),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_uses_canonical_order_without_losing_path_validation() {
        let oid = "a".repeat(40);
        let make_entry = |path: &str| Entry {
            virtual_path: path.to_owned(),
            source_path: "src/source.rs".into(),
            blob: oid.clone(),
            mode: "100644".into(),
            role: "focus".into(),
            detail: "from source".into(),
        };
        let plan = make_plan(
            "dependencies",
            "src/source",
            oid.clone(),
            oid.clone(),
            vec![
                make_entry("views/20-last/src/source.rs"),
                make_entry("views/00-first/src/source.rs"),
                make_entry("views/10-middle/src/source.rs"),
            ],
        )
        .unwrap();

        assert_eq!(
            get_entry(&plan, "views/00-first/src/source.rs")
                .unwrap()
                .virtual_path,
            "views/00-first/src/source.rs"
        );
        assert_eq!(
            get_entry(&plan, "views/20-last/src/source.rs")
                .unwrap()
                .virtual_path,
            "views/20-last/src/source.rs"
        );
        assert!(get_entry(&plan, "views/19-missing/src/source.rs").is_err());
        assert!(get_entry(&plan, "views/../escape").is_err());
    }
}
