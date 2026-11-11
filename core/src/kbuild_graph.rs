//! Git-bound Kbuild archive observations using Linux's own .cmd parser.
//! Only trusted local native archive inspection and generated command records.
use crate::json::{Value, obj};
use crate::sqlite::Db;
use crate::{Result, configured_store, digest, git, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPORT: usize = 24 * 1024 * 1024;
const MAX_MEMBERS: usize = 10_000;

fn sha_file(path: &Path, limit: u64) -> Result<String> {
    let len = fs::metadata(path)
        .map_err(|e| format!("Kbuild input {}: {e}", path.display()))?
        .len();
    if len > limit {
        return refuse("Kbuild input exceeds byte budget");
    }
    let bytes = fs::read(path).map_err(|e| format!("Kbuild read {}: {e}", path.display()))?;
    Ok(digest::sha256(&bytes))
}
fn relative(name: &str) -> Result<&Path> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains('\\')
        || name.contains('\0')
        || path.is_absolute()
        || !path.components().all(|p| matches!(p, Component::Normal(_)))
    {
        return refuse("unsafe Kbuild artifact path");
    }
    Ok(path)
}
fn in_build(build: &Path, name: &str) -> Result<PathBuf> {
    let path = build.join(relative(name)?);
    if path.exists()
        && !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(build)
    {
        return refuse("Kbuild path escapes selected build directory");
    }
    Ok(path)
}
fn cmd_of(object: &str) -> Result<String> {
    let path = relative(object)?;
    if !object.ends_with(".o") {
        return refuse("archive member not a .o object");
    }
    let name = path
        .file_name()
        .ok_or("missing object basename")?
        .to_string_lossy();
    Ok(path
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!(".{name}.cmd"))
        .to_string_lossy()
        .replace('\\', "/"))
}
fn native_export(repo: &Path, build: &Path, archive: &str, archiver: &str) -> Result<Value> {
    let output = Command::new("python3")
        .args(["-I", "-c", include_str!("../adapters/kbuild_native.py")])
        .arg(repo)
        .arg(build)
        .arg(archive)
        .arg(archiver)
        .output()
        .map_err(|e| format!("Kbuild native exporter: {e}"))?;
    if output.stdout.len() > MAX_EXPORT || output.stderr.len() > 16_384 {
        return refuse("Kbuild native exporter output exceeds budget");
    }
    if !output.status.success() {
        return refuse(format!(
            "Linux native Kbuild export failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    crate::json::parse(&output.stdout)
}
fn node(nodes: &mut BTreeMap<String, Value>, id: &str, kind: &str, payload: Value) -> Result<()> {
    configured_store::push_node(nodes, id.to_owned(), kind, payload)
}
fn edge(
    edges: &mut BTreeMap<String, Value>,
    from: &str,
    to: &str,
    kind: &str,
    status: &str,
    evidence: &str,
) -> Result<()> {
    configured_store::push_edge(edges, from, to, kind, status, evidence)
}

/// One archive of one local configured Kbuild output; no complete-kernel claims.
pub fn assemble(
    repo: &Path,
    rev: &str,
    build: &Path,
    archive: &str,
    archiver: &str,
    trusted: &str,
    db: &Db,
) -> Result<Value> {
    if trusted != "true" {
        return refuse("Kbuild tools require --trusted-local true");
    }
    if !matches!(archiver, "ar" | "llvm-ar") {
        return refuse("archiver must be ar or llvm-ar");
    }
    git::verify_repo(repo)?;
    let repo = repo.canonicalize().map_err(|e| e.to_string())?;
    let build = build.canonicalize().map_err(|e| e.to_string())?;
    let commit = git::rev(&repo, rev)?;
    if git::text(&repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("selected revision must resolve to commit");
    }
    let tree = git::rev(&repo, &format!("{commit}^{{tree}}"))?;
    if !archive.ends_with(".a") {
        return refuse("Kbuild scope must be an archive");
    }
    let archive_file = in_build(&build, archive)?;
    let config_file = build.join(".config");
    let config_sha = sha_file(&config_file, 4 * 1024 * 1024)?;
    let archive_sha = sha_file(&archive_file, MAX_BYTES)?;
    let generator = repo.join("scripts/clang-tools/gen_compile_commands.py");
    let producer_bytes = fs::read(&generator).map_err(|e| format!("generator: {e}"))?;
    let producer_blob = git::blob(
        &repo,
        &commit,
        "scripts/clang-tools/gen_compile_commands.py",
    )?;
    if digest::git_blob(&producer_bytes) != producer_blob {
        return refuse("Linux native generator differs from pinned Git source");
    }
    let producer_sha = digest::sha256(&producer_bytes);
    let exported = native_export(&repo, &build, archive, archiver)?;
    if exported.get("schema")?.str()? != "excession.kbuild-native-export.v1"
        || exported.get("archive")?.str()? != archive
        || exported.get("archive_sha256")?.str()? != archive_sha
        || exported.get("producer_sha256")?.str()? != producer_sha
        || exported.get("archiver")?.str()? != archiver
    {
        return refuse("native exporter disagrees with selected Kbuild inputs");
    }
    let members = exported.get("members")?.array_ref()?;
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return refuse("unsupported Kbuild archive member count");
    }
    let export_id = exported.sha256();
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let archive_id = format!("archive:{archive}");
    let config_id = format!("config:{config_sha}");
    node(
        &mut nodes,
        &archive_id,
        "kbuild-archive",
        obj(&[
            ("path", Value::string(archive)),
            ("sha256", Value::string(&archive_sha)),
        ]),
    )?;
    node(
        &mut nodes,
        &config_id,
        "kbuild-config",
        obj(&[
            ("sha256", Value::string(&config_sha)),
            ("path", Value::string(config_file.to_string_lossy())),
        ]),
    )?;
    edge(
        &mut edges,
        &config_id,
        &archive_id,
        "CONFIGURATION_SNAPSHOT",
        "LOCAL_CONFIG_HASHED_NOT_BUILD_CAUSALITY_PROOF",
        &config_sha,
    )?;
    let mut recorded = 0usize;
    let mut git_count = 0usize;
    let mut generated_count = 0usize;
    let mut unresolved = Vec::new();
    for item in members {
        let object = item.get("object")?.str()?;
        if !seen.insert(object.to_owned()) {
            return refuse("duplicate archive object");
        }
        let physical = in_build(&build, object)?;
        let cmd = item.get("cmd")?.str()?;
        if cmd != cmd_of(object)? {
            return refuse("command record differs from object identity");
        }
        let status = item.get("status")?.str()?;
        let object_hash = match item.get("object_sha256")? {
            Value::Null => None,
            v => {
                let sha = v.str()?;
                if sha_file(&physical, MAX_BYTES)? != sha {
                    return refuse("object bytes changed after native export");
                }
                Some(sha)
            }
        };
        let object_id = format!("object:{object}");
        node(
            &mut nodes,
            &object_id,
            "kbuild-object",
            obj(&[
                ("path", Value::string(object)),
                ("sha256", object_hash.map_or(Value::Null, Value::string)),
            ]),
        )?;
        edge(
            &mut edges,
            &archive_id,
            &object_id,
            "CONTAINS_OBJECT",
            "NATIVE_ARCHIVER_MEMBERSHIP_OBSERVED",
            &export_id,
        )?;
        let cmd_hash = match item.get("cmd_sha256")? {
            Value::Null => None,
            v => {
                let hash = v.str()?;
                if sha_file(&in_build(&build, cmd)?, 2 * 1024 * 1024)? != hash {
                    return refuse("Kbuild .cmd content changed during import");
                }
                Some(hash)
            }
        };
        if let Some(hash) = cmd_hash {
            let cmd_id = format!("record:{cmd}");
            node(
                &mut nodes,
                &cmd_id,
                "kbuild-command-record",
                obj(&[
                    ("path", Value::string(cmd)),
                    ("sha256", Value::string(hash)),
                    ("parser_status", Value::string(status)),
                ]),
            )?;
            edge(
                &mut edges,
                &object_id,
                &cmd_id,
                "HAS_BUILD_RECORD",
                "KBUILD_RECORD_BYTES_HASHED",
                hash,
            )?;
        }
        if status != "KBUILD_COMPILE_RECORD" {
            unresolved.push(obj(&[
                ("object", Value::string(object)),
                ("reason", Value::string(status)),
            ]));
            continue;
        }
        let cmd_hash = cmd_hash.ok_or("compile record is missing .cmd integrity evidence")?;
        let command = item.get("command")?.str()?;
        let source = Path::new(item.get("source")?.str()?)
            .canonicalize()
            .map_err(|e| format!("Kbuild input source: {e}"))?;
        let (source_name, oid, sha, relation_status) = if source.starts_with(&repo) {
            let name = source
                .strip_prefix(&repo)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            git::validate_path(&name)?;
            let blob = git::blob(&repo, &commit, &name)?;
            let content = fs::read(&source).map_err(|e| e.to_string())?;
            if content.len() > 4 * 1024 * 1024 || digest::git_blob(&content) != blob {
                return refuse(format!("Kbuild source differs from Git: {name}"));
            }
            git_count += 1;
            (
                name,
                Some(blob),
                digest::sha256(&content),
                "GIT_SOURCE_VERIFIED",
            )
        } else if source.starts_with(&build) {
            generated_count += 1;
            let name = source
                .strip_prefix(&build)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            (
                format!("generated:{name}"),
                None,
                sha_file(&source, 4 * 1024 * 1024)?,
                "GENERATED_SOURCE_HASHED",
            )
        } else {
            unresolved.push(obj(&[
                ("object", Value::string(object)),
                ("reason", Value::string("EXTERNAL_SOURCE_NOT_AUTHENTICATED")),
            ]));
            continue;
        };
        recorded += 1;
        let source_id = format!("source:{source_name}");
        node(
            &mut nodes,
            &source_id,
            "kbuild-source",
            obj(&[
                ("path", Value::string(&source_name)),
                ("git_blob", oid.map_or(Value::Null, Value::string)),
                ("sha256", Value::string(&sha)),
            ]),
        )?;
        edge(
            &mut edges,
            &format!("record:{cmd}"),
            &source_id,
            "COMPILES_SOURCE",
            relation_status,
            cmd_hash,
        )?;
        let command_hash = digest::sha256(command.as_bytes());
        let action_id = format!("compile:{}:{command_hash}", object);
        node(
            &mut nodes,
            &action_id,
            "kbuild-compile-action",
            obj(&[
                ("command_sha256", Value::string(&command_hash)),
                ("object", Value::string(object)),
            ]),
        )?;
        edge(
            &mut edges,
            &format!("record:{cmd}"),
            &action_id,
            "RECORDS_COMPILER_COMMAND",
            "LINUX_NATIVE_GENERATOR_EXTRACTED",
            &export_id,
        )?;
    }
    if sha_file(&config_file, 4 * 1024 * 1024)? != config_sha
        || sha_file(&archive_file, MAX_BYTES)? != archive_sha
        || sha_file(&generator, 2 * 1024 * 1024)? != producer_sha
    {
        return refuse("Kbuild inputs changed during assembly");
    }
    let body = obj(&[
        ("schema", Value::string("excession.kbuild-graph.v1")),
        ("source_commit", Value::string(&commit)),
        ("source_tree", Value::string(&tree)),
        ("build_directory", Value::string(build.to_string_lossy())),
        ("archive", Value::string(archive)),
        ("archive_sha256", Value::string(&archive_sha)),
        ("config_sha256", Value::string(&config_sha)),
        ("native_export_sha256", Value::string(&export_id)),
        ("archive_member_count", Value::number(members.len())),
        ("recorded_compile_action_count", Value::number(recorded)),
        ("git_verified_source_count", Value::number(git_count)),
        ("generated_source_count", Value::number(generated_count)),
        ("unresolved_object_count", Value::number(unresolved.len())),
        ("unresolved_objects", Value::array(unresolved)),
        (
            "status",
            Value::string(if recorded == members.len() {
                "ALL_ARCHIVE_MEMBERS_HAVE_COMPILER_RECORDS"
            } else {
                "PARTIAL_ARCHIVE_COMPILER_RECORDS"
            }),
        ),
        ("nodes", Value::array(nodes.values().cloned())),
        ("edges", Value::array(edges.values().cloned())),
        (
            "limitations",
            Value::array([
                Value::string("One selected Kbuild archive, not transitive kernel build closure"),
                Value::string(
                    "Archive membership and .cmd records do not prove successful compilation",
                ),
                Value::string("C and assembly only when Linux native parser accepts the .cmd"),
                Value::string("Generated inputs locally hashed, not Git-authenticated"),
            ]),
        ),
    ]);
    let id = body.sha256();
    let receipts = vec![(archive.to_owned(), export_id, exported)];
    configured_store::persist(
        db,
        &id,
        &commit,
        archive,
        &config_sha,
        &body,
        &nodes,
        &edges,
        &receipts,
    )?;
    let mut result = body.entries()?.clone();
    result.insert("graph_id".into(), Value::string(id));
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_guards() {
        assert!(relative("../escape").is_err());
        assert!(relative("/etc/passwd").is_err());
        assert!(relative("init/main.o").is_ok());
        assert_eq!(cmd_of("init/main.o").unwrap(), "init/.main.o.cmd");
        assert_eq!(cmd_of("foo.o").unwrap(), ".foo.o.cmd");
    }
}
