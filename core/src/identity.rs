//! Rust-controlled Git/SQLite materialization of Python module metadata views.
//! CPython AST determines guarded assignments; Python itself checks pickle/reflection.
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, json, refuse};
use json::{Value, obj};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS python_identity_view (
 id TEXT PRIMARY KEY,
 baseline_repo INTEGER NOT NULL REFERENCES repository(id), baseline_commit TEXT NOT NULL,
 parent_repo INTEGER NOT NULL REFERENCES repository(id), parent_commit TEXT NOT NULL,
 output_repo INTEGER NOT NULL REFERENCES repository(id), output_commit TEXT NOT NULL,
 output_tree TEXT NOT NULL, physical_path TEXT NOT NULL,
 logical_module TEXT NOT NULL, physical_module TEXT NOT NULL,
 appendix_sha256 TEXT NOT NULL, symbol_count INTEGER NOT NULL CHECK(symbol_count>0),
 status TEXT NOT NULL CHECK(status='BOUNDED_REFLECTION_AND_PICKLE_COMPATIBILITY'),
 FOREIGN KEY(baseline_repo,baseline_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(parent_repo,parent_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(output_repo,output_commit) REFERENCES revision(repo_id,commit_oid)
);
CREATE TABLE IF NOT EXISTS python_identity_symbol (
 view_id TEXT NOT NULL REFERENCES python_identity_view(id),
 qualname TEXT NOT NULL, PRIMARY KEY(view_id,qualname)
);
"#;
#[derive(Clone)]
pub struct IdentityPlan {
    pub original: PathBuf,
    pub projected: PathBuf,
    pub output: PathBuf,
    pub original_path: String,
    pub projected_path: String,
    pub logical: String,
    pub physical: String,
    pub adapter: PathBuf,
    pub probe: PathBuf,
    pub crossload: PathBuf,
    pub label: String,
}
impl IdentityPlan {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        let get = |k| -> Result<String> { Ok(v.get(k)?.str()?.to_owned()) };
        let real = |k| -> Result<PathBuf> {
            PathBuf::from(get(k)?)
                .canonicalize()
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            original: real("original")?,
            projected: real("projected")?,
            output: PathBuf::from(get("output")?),
            original_path: get("original_path")?,
            projected_path: get("projected_path")?,
            logical: get("logical")?,
            physical: get("physical")?,
            adapter: real("adapter")?,
            probe: real("probe")?,
            crossload: real("crossload")?,
            label: get("label")?,
        })
    }
}
fn native(cmd: &Path, repo: &Path, adapter: &Path, input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut p = Command::new("python");
    p.arg("-B")
        .arg(cmd)
        .current_dir(repo)
        .env(
            "PYTHONPATH",
            format!(
                "{}:{}",
                repo.display(),
                adapter.parent().ok_or("adapter has no parent")?.display()
            ),
        )
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONHASHSEED", "0")
        .env("PYTHONNOUSERSITE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if input.is_some() {
        p.stdin(Stdio::piped());
    }
    let mut child = p.spawn().map_err(|e| e.to_string())?;
    if let Some(v) = input {
        child
            .stdin
            .take()
            .ok_or("python stdin unavailable")?
            .write_all(v)
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "Python identity adapter/reflection refused: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}
fn git_command(repo: &Path, args: &[&str], dated: bool) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    if dated {
        command
            .env("GIT_AUTHOR_NAME", "RelGit Test")
            .env("GIT_COMMITTER_NAME", "RelGit Test")
            .env("GIT_AUTHOR_EMAIL", "relgit@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "relgit@example.invalid")
            .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00+0000")
            .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00+0000");
    }
    let out = command.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git metadata projection: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|e| e.to_string())
}
fn observe(root: &Path, p: &IdentityPlan) -> Result<Value> {
    json::parse(&native(&p.probe, root, &p.adapter, None)?)
}
fn check(
    original: &Value,
    generated: &Value,
    names: &[String],
    logical: &str,
    physical: &str,
) -> Result<()> {
    let x = original.get("objects")?.entries()?;
    let y = generated.get("objects")?.entries()?;
    if x.keys().cloned().collect::<BTreeSet<_>>() != names.iter().cloned().collect()
        || x.keys().ne(y.keys())
    {
        return refuse("identity object inventory mismatches");
    }
    for name in names {
        let before = x.get(name).ok_or("missing original symbol")?;
        let after = y.get(name).ok_or("missing projected symbol")?;
        for k in [
            "module",
            "qualname",
            "signature",
            "pickle_protocol_4",
            "pickle_protocol_5",
            "unpickle_same",
        ] {
            if before.get(k)? != after.get(k)? {
                return refuse(format!("reflection/pickle differed on {name}.{k}"));
            }
        }
        if before.get("module")?.str()? != logical || !before.get("unpickle_same")?.bool()? {
            return refuse("logical Python identity or pickle round trip not preserved");
        }
    }
    if original.get("instance")? != generated.get("instance")? {
        return refuse("Headers instance pickle output differed");
    }
    if !generated.get("legacy_alias_identical")?.bool()?
        || !generated.get("class_identity")?.bool()?
    {
        return refuse("physical and logical aliases disagree");
    }
    if original.get("module_name")?.str()? != logical
        || generated.get("module_name")?.str()? != physical
    {
        return refuse("module object name mismatch");
    }
    if original.get("source_path")? == generated.get("source_path")?
        || original.get("spec_name")? == generated.get("spec_name")?
    {
        return refuse("physical path/spec distinction must remain explicit");
    }
    Ok(())
}
pub fn materialize(db: &Db, p: &IdentityPlan) -> Result<Value> {
    git::clean(&p.original)?;
    git::clean(&p.projected)?;
    git::validate_path(&p.original_path)?;
    git::validate_path(&p.projected_path)?;
    if !p.logical.starts_with("h11.") || p.logical == p.physical || p.label.is_empty() {
        return refuse("unsupported Python identity mapping");
    }
    if p.output.exists() {
        return refuse("identity output exists");
    }
    let baseline = git::rev(&p.original, "HEAD")?;
    let parent = git::rev(&p.projected, "HEAD")?;
    let original_bytes = git::command(
        &p.original,
        &["show", &format!("{baseline}:{}", p.original_path)],
    )?;
    let old = git::command(
        &p.projected,
        &["show", &format!("{parent}:{}", p.projected_path)],
    )?;
    let original_text =
        String::from_utf8(original_bytes).map_err(|_| "non UTF8 Python baseline")?;
    let projected_text = String::from_utf8(old.clone()).map_err(|_| "non UTF8 Python view")?;
    let request = obj(&[
        ("original", Value::string(original_text)),
        ("projected", Value::string(projected_text)),
        ("logical", Value::string(&p.logical)),
        ("physical", Value::string(&p.physical)),
    ]);
    let response = json::parse(&native(
        &p.adapter,
        &p.projected,
        &p.adapter,
        Some(request.canonical().as_bytes()),
    )?)?;
    let content = response.get("content")?.str()?.as_bytes();
    let names = response
        .get("symbols")?
        .array_ref()?
        .iter()
        .map(|x| Ok(x.str()?.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    let suffix = digest::unhex(response.get("suffix_hex")?.str()?)?;
    if names.is_empty()
        || content.len() <= old.len()
        || !content.starts_with(&old)
        || content[old.len()..] != suffix
        || !content.ends_with(&suffix)
    {
        return refuse("native adapter output not exact append-only change");
    }
    let parent_dir = p.output.parent().ok_or("output needs parent")?;
    fs::create_dir_all(parent_dir).map_err(|e| e.to_string())?;
    let temp = parent_dir.join(format!(
        ".relgit-identity-{}-{}",
        p.label,
        std::process::id()
    ));
    if temp.exists() {
        return refuse("identity temp path exists");
    }
    let execute = || -> Result<Value> {
        let clone = Command::new("git")
            .args(["clone", "--local", "--no-hardlinks", "--quiet"])
            .arg(&p.projected)
            .arg(&temp)
            .output()
            .map_err(|e| e.to_string())?;
        if !clone.status.success() {
            return refuse("cannot clone projected source");
        }
        git_command(&temp, &["checkout", "--quiet", "--detach", &parent], false)?;
        if fs::read(temp.join(&p.projected_path)).map_err(|e| e.to_string())? != old {
            return refuse("source checkout differs from projected Git blob");
        }
        fs::write(temp.join(&p.projected_path), content).map_err(|e| e.to_string())?;
        git_command(&temp, &["add", "--", &p.projected_path], false)?;
        let changed = git::command(&temp, &["diff", "--cached", "--name-only", "-z"])?;
        if changed != [p.projected_path.as_bytes(), b"\0"].concat() {
            return refuse("identity change edited undeclared paths");
        }
        let tree = git_command(&temp, &["write-tree"], false)?;
        let commit = git_command(
            &temp,
            &[
                "commit-tree",
                &tree,
                "-p",
                &parent,
                "-m",
                &format!("relgit: logical Python identities {}", p.logical),
            ],
            true,
        )?;
        git_command(&temp, &["update-ref", "HEAD", &commit, &parent], false)?;
        git::clean(&temp)?;
        let observation_before = observe(&p.original, p)?;
        let observation_after = observe(&temp, p)?;
        check(
            &observation_before,
            &observation_after,
            &names,
            &p.logical,
            &p.physical,
        )?;
        let pickle = observation_before.get("pickle_bytes")?.canonical();
        let loaded = native(&p.crossload, &temp, &p.adapter, Some(pickle.as_bytes()))?;
        if loaded != b"PICKLE_CROSS_LOAD_OK\n" {
            return refuse("cross-process pickle lookup refused");
        }
        fs::rename(&temp, &p.output).map_err(|e| e.to_string())?;
        let baseline_id = index::index_revision_with_roots(
            db,
            &p.original,
            &format!("identity-original-{}", p.label),
            &baseline,
            "identity oracle source",
            &["h11"],
        )?;
        let parent_id = index::index_revision_with_roots(
            db,
            &p.projected,
            &format!("identity-parent-{}", p.label),
            &parent,
            "identity parent",
            &["h11"],
        )?;
        let output_id = index::index_revision_with_roots(
            db,
            &p.output,
            &format!("identity-output-{}", p.label),
            &commit,
            "identity generated",
            &["h11"],
        )?;
        if index::reconstruct_tree(db, output_id, &commit)? != tree {
            return refuse("metadata view tree not reconstructible");
        }
        let ident = obj(&[
            ("baseline", Value::string(&baseline)),
            ("parent", Value::string(&parent)),
            ("output", Value::string(&commit)),
            ("logical", Value::string(&p.logical)),
            ("physical", Value::string(&p.physical)),
        ])
        .sha256();
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            db.execute(
                "INSERT INTO python_identity_view VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&ident),
                    Param::Integer(baseline_id),
                    Param::Text(&baseline),
                    Param::Integer(parent_id),
                    Param::Text(&parent),
                    Param::Integer(output_id),
                    Param::Text(&commit),
                    Param::Text(&tree),
                    Param::Text(&p.projected_path),
                    Param::Text(&p.logical),
                    Param::Text(&p.physical),
                    Param::Text(&digest::sha256(&suffix)),
                    Param::Integer(names.len() as i64),
                    Param::Text("BOUNDED_REFLECTION_AND_PICKLE_COMPATIBILITY"),
                ],
            )?;
            for name in &names {
                db.execute(
                    "INSERT INTO python_identity_symbol VALUES(?,?)",
                    &[Param::Text(&ident), Param::Text(name)],
                )?;
            }
            db.foreign_keys_valid()
        })?;
        Ok(obj(&[
            ("id", Value::string(ident)),
            ("commit", Value::string(commit)),
            ("tree", Value::string(tree)),
            ("symbols", Value::number(names.len())),
            ("appendix_sha256", Value::string(digest::sha256(&suffix))),
            (
                "status",
                Value::string("BOUNDED_REFLECTION_AND_PICKLE_COMPATIBILITY"),
            ),
        ]))
    };
    let outcome = execute();
    if outcome.is_err() && temp.exists() {
        let _ = fs::remove_dir_all(&temp);
    }
    outcome
}
