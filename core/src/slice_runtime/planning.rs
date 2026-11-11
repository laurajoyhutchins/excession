//! Canonical Git-source planning with a narrow CPython AST parser.
use super::process::process;
use super::{
    MAX_FILES, MAX_OUTPUT, MAX_PYTHON_BYTES, MAX_PYTHON_FILES, MAX_SOURCE_BYTES, PARSER, PLAN_TIME,
};
use crate::json::{self, Value, obj};
use crate::{Result, digest, git, refuse, slice_evidence};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub(super) fn validate_path(path: &str) -> Result<()> {
    git::validate_path(path)?;
    if path == ".git" || path.starts_with(".git/") {
        return refuse("Git metadata cannot be a slice input");
    }
    if !path.is_ascii() {
        // The original Python JSON used ensure_ascii=True for identity-forming
        // manifests. Until the schemas normalize Unicode, refuse ambiguity.
        return refuse("non-ASCII paths require a versioned manifest identity rule");
    }
    Ok(())
}
pub(super) fn sorted_assets(assets: &[String]) -> Result<Vec<String>> {
    let mut out = assets.to_vec();
    for asset in &out {
        validate_path(asset)?;
    }
    out.sort();
    if out.windows(2).any(|p| p[0] == p[1]) {
        return refuse("duplicate slice asset");
    }
    Ok(out)
}

/// Resolve CPython import semantics on Git-authenticated bytes, then construct
/// the same v1 manifest as the legacy Python planner in Rust.
pub fn plan(
    repo: &Path,
    revision: &str,
    entry: &str,
    assets: &[String],
    historical: bool,
) -> Result<Value> {
    git::verify_repo(repo)?;
    let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
    if !historical {
        git::clean(repo)?;
        if commit != git::rev(repo, "HEAD^{commit}")? {
            return refuse("revision must be clean checkout HEAD unless historical is enabled");
        }
    }
    validate_path(entry)?;
    if !entry.ends_with(".py") {
        return refuse("entry must be a Python file");
    }
    let assets = sorted_assets(assets)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let inventory = git::entries(repo, &commit)?;
    let mut tracked = BTreeMap::new();
    let mut source_files = BTreeMap::new();
    let mut python_file_count = 0usize;
    let mut source_bytes = 0usize;
    for item in &inventory {
        validate_path(&item.path)?;
        tracked.insert(item.path.clone(), item.clone());
        if !item.path.ends_with(".py") || !matches!(item.mode.as_str(), "100644" | "100755") {
            continue;
        }
        python_file_count += 1;
        if python_file_count > MAX_PYTHON_FILES {
            return refuse("Python inventory exceeds 4096 files");
        }
        let data = git::command(repo, &["cat-file", "blob", &item.oid])?;
        source_bytes = source_bytes
            .checked_add(data.len())
            .ok_or("source byte overflow")?;
        if source_bytes > MAX_PYTHON_BYTES {
            return refuse("Python inventory exceeds 32 MiB");
        }
        // Unrelated sources may have unsupported encodings or syntax. Only a
        // file selected by CPython's import closure must successfully parse.
        if let Ok(source) = String::from_utf8(data) {
            source_files.insert(item.path.clone(), Value::string(source));
        }
    }
    if !tracked.contains_key(entry) {
        return refuse("untracked Python entrypoint");
    }
    let input = obj(&[
        ("entry", Value::string(entry)),
        ("assets", Value::array(assets.iter().map(Value::string))),
        ("paths", Value::array(tracked.keys().map(Value::string))),
        ("sources", Value::Object(source_files)),
    ]);
    let mut cmd = Command::new("python3");
    cmd.args(["-I", "-B", "-c", PARSER]);
    cmd.env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LC_ALL", "C.UTF-8")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    let observed = process(
        cmd,
        Some(input.canonical().into_bytes()),
        PLAN_TIME,
        MAX_OUTPUT,
    )?;
    if observed.timed_out || observed.code != 0 {
        return refuse(format!(
            "CPython import adapter refused: {}",
            String::from_utf8_lossy(&observed.stderr)
        ));
    }
    let paths = json::parse(&observed.stdout)?
        .get("paths")?
        .array_ref()?
        .iter()
        .map(|p| p.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    if paths.is_empty() || paths.len() > MAX_FILES {
        return refuse("slice closure exceeds 512 files");
    }
    let mut selected = Vec::new();
    let mut count = 0usize;
    let mut last: Option<&str> = None;
    for path in &paths {
        validate_path(path)?;
        if last.is_some_and(|p| path.as_str() <= p) {
            return refuse("parser paths unsorted or repeated");
        }
        last = Some(path);
        let git_entry = tracked.get(path).ok_or("parser selected untracked file")?;
        if git_entry.kind != "blob" || git_entry.mode != "100644" {
            return refuse("slice requires regular nonexecutable Git blobs");
        }
        let data = git::command(repo, &["cat-file", "blob", &git_entry.oid])?;
        count = count.checked_add(data.len()).ok_or("slice size overflow")?;
        if count > MAX_SOURCE_BYTES {
            return refuse("slice closure exceeds 32 MiB");
        }
        selected.push(obj(&[
            ("path", Value::string(path)),
            ("blob", Value::string(&git_entry.oid)),
            ("bytes", Value::number(data.len())),
            ("sha256", Value::string(digest::sha256(&data))),
        ]));
    }
    if !paths.iter().any(|p| p == entry) || assets.iter().any(|asset| !paths.contains(asset)) {
        return refuse("parser omitted requested entry or asset");
    }
    let mut manifest = obj(&[
        ("schema", Value::string("excession.executable-slice.v1")),
        ("commit", Value::string(commit)),
        ("tree", Value::string(tree)),
        ("entry", Value::string(entry)),
        ("assets", Value::array(assets.iter().map(Value::string))),
        ("entries", Value::Array(selected)),
        ("scope", Value::string("STATIC_IMPORT_CLOSURE_ONLY")),
        ("semantic_equivalence", Value::string("UNVERIFIED")),
    ]);
    let id = manifest.sha256();
    manifest
        .entries_mut()?
        .insert("view_id".into(), Value::string(id));
    Ok(manifest)
}

pub(super) fn validate(
    repo: &Path,
    manifest: &Value,
) -> Result<(String, Vec<slice_evidence::Source>)> {
    let (_view, commit, _tree, entry, sources) = slice_evidence::source_rows(repo, manifest)?;
    let assets = manifest
        .get("assets")?
        .array_ref()?
        .iter()
        .map(|x| x.str().map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    // Recompute CPython closure from pinned objects. An authenticated subset
    // alone does not establish a semantically complete declared import closure.
    let expected = plan(repo, &commit, &entry, &assets, true)?;
    if &expected != manifest {
        return refuse("slice manifest differs from derived CPython dependency closure");
    }
    Ok((commit, sources))
}
