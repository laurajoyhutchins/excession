//! Read-only source construction and independently recomputed filesystem audit.
use super::PrivateDir;
use super::planning::{validate, validate_path};
use crate::json::{Value, obj};
use crate::{Result, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

fn external_destination(repo: &Path, destination: &Path) -> Result<(PathBuf, PathBuf)> {
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        return refuse("slice destination already exists");
    }
    let parent = destination
        .parent()
        .ok_or("destination has no parent")?
        .canonicalize()
        .map_err(|e| format!("destination parent inaccessible: {e}"))?;
    let name = destination.file_name().ok_or("destination name missing")?;
    let repo = repo.canonicalize().map_err(|e| e.to_string())?;
    let absolute = parent.join(name);
    if absolute.starts_with(&repo) || absolute == repo {
        return refuse("slice destination must be outside source checkout");
    }
    Ok((parent, absolute))
}
fn save_file(root: &Path, path: &str, data: &[u8]) -> Result<()> {
    validate_path(path)?;
    let dest = root.join(path);
    let parent = dest.parent().ok_or("slice file has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("cannot create slice directories: {e}"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&dest)
        .map_err(|e| format!("cannot construct slice source: {e}"))?;
    file.write_all(data)
        .map_err(|e| format!("cannot materialize slice Git blob: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o444))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn materialize(repo: &Path, manifest: &Value, dest: &Path) -> Result<Value> {
    let (_commit, sources) = validate(repo, manifest)?;
    let (parent, absolute) = external_destination(repo, dest)?;
    let work = PrivateDir::new(&parent, "excession-slice")?;
    for item in sources {
        let data = git::command(repo, &["cat-file", "blob", &item.blob])?;
        if data.len() != item.bytes as usize || digest::sha256(&data) != item.sha256 {
            return refuse("slice materialization source differs from pinned Git blob");
        }
        save_file(work.path(), &item.path, &data)?;
    }
    fs::rename(work.path(), &absolute)
        .map_err(|e| format!("cannot publish slice directory: {e}"))?;
    Ok(obj(&[
        ("status", Value::string("MATERIALIZED")),
        ("directory", Value::string(dest.to_string_lossy().as_ref())),
    ]))
}

pub fn audit(repo: &Path, manifest: &Value, root: &Path) -> Result<Value> {
    let (_, sources) = validate(repo, manifest)?;
    let metadata =
        fs::symlink_metadata(root).map_err(|e| format!("missing slice directory: {e}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return refuse("slice root is not a regular directory");
    }
    let expected = sources
        .iter()
        .map(|x| (x.path.as_str(), x))
        .collect::<BTreeMap<_, _>>();
    let mut found = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for item in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return refuse("slice contains symlink");
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                return refuse("slice contains nonregular file");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o222 != 0 {
                    return refuse("slice contains writable source file");
                }
            }
            let key = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("non-UTF8 slice pathname")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let source = expected
                .get(key.as_str())
                .ok_or("unexpected file in slice")?;
            if !found.insert(key) {
                return refuse("duplicate slice file");
            }
            if meta.len() != source.bytes as u64 {
                return refuse("slice source size changed");
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            if digest::sha256(&bytes) != source.sha256 {
                return refuse("slice source bytes changed");
            }
        }
    }
    if found.len() != expected.len() {
        return refuse("slice is missing source files");
    }
    Ok(obj(&[("status", Value::string("AUDITED"))]))
}
