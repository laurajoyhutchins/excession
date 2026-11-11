//! Rust-owned executable-slice orchestration. CPython supplies only AST semantics.
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

mod execution;
mod filesystem;
mod planning;
mod process;
mod transport;

pub use execution::execute;
pub use filesystem::{audit, materialize};
pub use planning::plan;
use planning::validate;
pub use transport::{carry, replay};

const PARSER: &str = include_str!("../adapters/python_slice_imports.py");
const MAX_FILES: usize = 512;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PYTHON_FILES: usize = 4096;
const MAX_PYTHON_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_MANIFEST: usize = 5 * 1024 * 1024;
const MAX_RECEIPT: usize = 1024 * 1024;
const PLAN_TIME: Duration = Duration::from_secs(30);
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temporary(parent: &Path, prefix: &str) -> Result<PathBuf> {
    for _ in 0..64 {
        let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let name = format!(".{prefix}-{}-{number}", std::process::id());
        let path = parent.join(name);
        match fs::create_dir(&path) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                        .map_err(|e| e.to_string())?;
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => {
                return refuse(format!(
                    "cannot create private temporary directory: {error}"
                ));
            }
        }
    }
    refuse("could not allocate private temporary directory")
}

struct PrivateDir(PathBuf);
impl PrivateDir {
    fn new(parent: &Path, prefix: &str) -> Result<Self> {
        temporary(parent, prefix).map(Self)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_manifest(path: &Path) -> Result<Value> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.len() > MAX_MANIFEST {
        return refuse("slice manifest exceeds 5 MiB");
    }
    json::parse(&data)
}
fn read_receipt(path: &Path) -> Result<Value> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    if data.len() > MAX_RECEIPT {
        return refuse("slice receipt exceeds 1 MiB");
    }
    json::parse(&data)
}

pub fn plan_cli(
    repo: &Path,
    revision: &str,
    entry: &str,
    asset_json: &str,
    historical: bool,
) -> Result<Value> {
    let assets = json::parse(asset_json.as_bytes())?
        .array_ref()?
        .iter()
        .map(|v| v.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    plan(repo, revision, entry, &assets, historical)
}
pub fn materialize_cli(repo: &Path, manifest: &Path, directory: &Path) -> Result<Value> {
    materialize(repo, &read_manifest(manifest)?, directory)
}
pub fn audit_cli(repo: &Path, manifest: &Path, directory: &Path) -> Result<Value> {
    audit(repo, &read_manifest(manifest)?, directory)
}
pub fn run_cli(
    repo: &Path,
    manifest: &Path,
    directory: &Path,
    timeout: u64,
    isolated: bool,
    trusted: bool,
) -> Result<Value> {
    execute(
        repo,
        &read_manifest(manifest)?,
        directory,
        timeout,
        isolated,
        trusted,
    )
}
pub fn carry_cli(
    repo: &Path,
    manifest: &Path,
    receipt: &Path,
    revision: &str,
    replay_target: bool,
    isolated: bool,
    trusted: bool,
) -> Result<Value> {
    let manifest = read_manifest(manifest)?;
    let receipt = read_receipt(receipt)?;
    if replay_target {
        replay(repo, &manifest, &receipt, revision, isolated, trusted)
    } else {
        carry(repo, &manifest, &receipt, revision)
    }
}

pub fn validate_cli(repo: &Path, manifest: &Path) -> Result<Value> {
    validate(repo, &read_manifest(manifest)?)?;
    Ok(obj(&[("status", Value::string("VALIDATED"))]))
}

#[cfg(test)]
mod tests {
    use super::planning::sorted_assets;
    use super::*;
    #[test]
    fn canonical_slice_identity() {
        let mut manifest = obj(&[
            ("schema", Value::string("excession.executable-slice.v1")),
            ("entry", Value::string("a.py")),
        ]);
        let sha = manifest.sha256();
        manifest
            .entries_mut()
            .unwrap()
            .insert("view_id".into(), Value::string(&sha));
        assert_eq!(manifest.get("view_id").unwrap().str().unwrap(), sha);
    }
    #[test]
    fn asset_validation() {
        assert!(sorted_assets(&["a/../x".into()]).is_err());
        assert!(sorted_assets(&["a.txt".into(), "a.txt".into()]).is_err());
        assert!(sorted_assets(&["z.txt".into(), "a.txt".into()]).is_ok());
    }
}
