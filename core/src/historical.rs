//! Audited historical patch replay across typed module moves.
//! Rust validates paths, signed-off scope, Git object versions, file modes and
//! applies the mapped hunk using Git's three-way algorithm in a disposable clone.
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, json, refuse, transform};
use json::{Value, obj};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS replay_provenance (
 id TEXT PRIMARY KEY,
 projection_id TEXT NOT NULL REFERENCES projection(id),
 upstream_repo TEXT NOT NULL,
 upstream_parent TEXT NOT NULL,
 upstream_commit TEXT NOT NULL,
 source_repo INTEGER NOT NULL REFERENCES repository(id),
 source_change_commit TEXT NOT NULL,
 target_repo INTEGER NOT NULL REFERENCES repository(id),
 target_commit TEXT NOT NULL,
 target_tree TEXT NOT NULL,
 scope TEXT NOT NULL CHECK(scope IN ('exact_changed_blobs','historical_patch_transplant')),
 upstream_manifest_sha256 TEXT NOT NULL,
 original_patch_sha256 TEXT NOT NULL,
 mapped_patch_sha256 TEXT NOT NULL,
 FOREIGN KEY(source_repo,source_change_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(target_repo,target_commit) REFERENCES revision(repo_id,commit_oid)
);
CREATE TABLE IF NOT EXISTS replay_path (
 replay_id TEXT NOT NULL REFERENCES replay_provenance(id),
 original_path TEXT NOT NULL,
 mapped_path TEXT NOT NULL,
 exact_upstream INTEGER NOT NULL CHECK(exact_upstream IN (0,1)),
 PRIMARY KEY(replay_id,original_path), UNIQUE(replay_id,mapped_path)
);
"#;
#[derive(Clone)]
pub struct Manifest {
    pub upstream_repo: String,
    pub parent: String,
    pub commit: String,
    pub scope: String,
    pub paths: Vec<String>,
    pub old_blobs: Vec<String>,
    pub new_blobs: Vec<String>,
    pub historic_patch: PathBuf,
    pub digest: String,
}
impl Manifest {
    fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        let strings = |name| -> Result<Vec<String>> {
            v.get(name)?
                .array_ref()?
                .iter()
                .map(|x| Ok(x.str()?.to_owned()))
                .collect()
        };
        let paths = strings("paths")?;
        let old_blobs = strings("old_blobs")?;
        let new_blobs = strings("new_blobs")?;
        let historic_patch = PathBuf::from(v.get("fixture_patch")?.str()?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let obj = Self {
            upstream_repo: v.get("upstream_repository")?.str()?.to_owned(),
            parent: git::oid(v.get("upstream_parent")?.str()?)?,
            commit: git::oid(v.get("upstream_commit")?.str()?)?,
            scope: v.get("scope")?.str()?.to_owned(),
            paths,
            old_blobs,
            new_blobs,
            historic_patch,
            digest: v.sha256(),
        };
        if !matches!(
            obj.scope.as_str(),
            "exact_changed_blobs" | "historical_patch_transplant"
        ) || obj.paths.is_empty()
            || obj.paths.len() != obj.old_blobs.len()
            || obj.paths.len() != obj.new_blobs.len()
        {
            return refuse("invalid upstream manifest");
        }
        if obj.upstream_repo.trim().is_empty() {
            return refuse("empty upstream identity");
        }
        let mut seen = BTreeSet::new();
        for path in &obj.paths {
            git::validate_path(path)?;
            if path.bytes().any(|x| x < 32) || !seen.insert(path) {
                return refuse("duplicate or unsafe historical path");
            }
        }
        for hash in obj.old_blobs.iter().chain(obj.new_blobs.iter()) {
            git::oid(hash)?;
        }
        Ok(obj)
    }
}
fn run(root: &Path, args: &[&str], data: Option<&[u8]>, env: bool) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    if data.is_some() {
        cmd.stdin(Stdio::piped());
    }
    if env {
        cmd.env("GIT_AUTHOR_NAME", "RelGit Historical Replay")
            .env("GIT_COMMITTER_NAME", "RelGit Historical Replay")
            .env("GIT_AUTHOR_EMAIL", "relgit@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "relgit@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-11-09T00:00:00+0000")
            .env("GIT_COMMITTER_DATE", "2026-11-09T00:00:00+0000");
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(d) = data {
        child
            .stdin
            .take()
            .ok_or("git stdin unavailable")?
            .write_all(d)
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}
fn scalar(root: &Path, args: &[&str], env: bool) -> Result<String> {
    Ok(String::from_utf8(run(root, args, None, env)?)
        .map_err(|e| e.to_string())?
        .trim()
        .into())
}
fn clone(source: &Path, commit: &str, to: &Path) -> Result<()> {
    if to.exists() {
        return refuse("output already exists");
    }
    let output = Command::new("git")
        .args(["clone", "--local", "--no-hardlinks", "--quiet"])
        .arg(source)
        .arg(to)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return refuse(format!(
            "clone: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    run(
        to,
        &["checkout", "--quiet", "--detach", commit],
        None,
        false,
    )?;
    git::clean(to)
}
fn changed_lines(data: &[u8]) -> Vec<Vec<u8>> {
    data.split(|b| *b == b'\n')
        .filter(|line| {
            line.len() > 1
                && (line[0] == b'+' || line[0] == b'-')
                && !line.starts_with(b"+++ ")
                && !line.starts_with(b"--- ")
        })
        .map(|l| l.to_vec())
        .collect()
}
fn strict_path(raw: &[u8]) -> Result<String> {
    let s = std::str::from_utf8(raw).map_err(|_| "historical non-UTF8 path")?;
    git::validate_path(s)?;
    if s.bytes().any(|b| b <= 32) || s.split('/').any(|p| p.starts_with(".git")) {
        return refuse("unsupported quoted, escaped or control-character Git path");
    }
    Ok(s.to_owned())
}
/// Conservative unified diff rewriting. Keep hunk bytes exactly unchanged.
pub fn rewrite_patch(
    raw: &[u8],
    allowed: &BTreeSet<String>,
    moves: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, Vec<String>)> {
    if raw.is_empty() || !raw.ends_with(b"\n") {
        return refuse("empty or unterminated historical patch");
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    let mut state = 0u8;
    let mut current = String::new();
    let mut headers = 0u8;
    for line in raw.split_inclusive(|x| *x == b'\n') {
        if line.starts_with(b"diff --git ") {
            if state > 0 && headers != 3 {
                return refuse("missing prior patch headers");
            }
            let rest = &line[b"diff --git ".len()..line.len() - 1];
            let fields = rest.split(|b| *b == b' ').collect::<Vec<_>>();
            if fields.len() != 2
                || !fields[0].starts_with(b"a/")
                || !fields[1].starts_with(b"b/")
                || fields[0][2..] != fields[1][2..]
            {
                return refuse("unsupported quoted or renamed diff header");
            }
            let old = strict_path(&fields[0][2..])?;
            if !allowed.contains(&old) || seen.contains(&old) {
                return refuse(format!("undeclared or duplicate changed path {old}"));
            }
            let new = moves.get(&old).unwrap_or(&old);
            out.extend(format!("diff --git a/{new} b/{new}\n").as_bytes());
            seen.push(old.clone());
            current = old;
            state = 1;
            headers = 0;
            continue;
        }
        if state == 0 {
            return refuse("patch lacks diff --git");
        }
        if [
            b"new file mode ".as_slice(),
            b"deleted file mode ",
            b"rename ",
            b"copy ",
            b"Binary files ",
            b"GIT binary patch",
            b"old mode ",
            b"new mode ",
        ]
        .iter()
        .any(|p| line.starts_with(p))
        {
            return refuse("binary, rename, add, delete or mode change requires separate adapter");
        }
        if line.starts_with(b"--- ") || line.starts_with(b"+++ ") {
            let is_old = line.starts_with(b"--- ");
            let prefix = if is_old { b"a/" } else { b"b/" };
            if !line[4..].starts_with(prefix) {
                return refuse("add/delete patch unsupported");
            }
            let p = strict_path(&line[6..line.len() - 1])?;
            if p != current {
                return refuse("patch file header disagrees with diff header");
            }
            headers |= if is_old { 1 } else { 2 };
            let new = moves.get(&current).unwrap_or(&current);
            out.extend(if is_old { b"--- a/" } else { b"+++ b/" });
            out.extend(new.as_bytes());
            out.push(b'\n');
        } else {
            out.extend(line)
        }
    }
    if headers != 3 || seen.is_empty() || seen.iter().cloned().collect::<BTreeSet<_>>() != *allowed
    {
        return refuse("patch files differ from declared change set");
    }
    let mapped = seen
        .iter()
        .map(|p| moves.get(p).unwrap_or(p))
        .collect::<BTreeSet<_>>();
    if mapped.len() != seen.len() {
        return refuse("destination path collision");
    }
    Ok((out, seen))
}
pub struct ReplayPlan {
    pub manifest: Manifest,
    pub source_index_name: Option<String>,
    pub source: PathBuf,
    pub projection: PathBuf,
    pub projection_id: String,
    pub output: PathBuf,
    pub name: String,
    pub native_tests: Vec<Vec<String>>,
}

pub fn replay(db: &Db, plan: &ReplayPlan) -> Result<Value> {
    let manifest = &plan.manifest;
    let source = &plan.source;
    let projection = &plan.projection;
    let projection_id = plan.projection_id.as_str();
    let output = &plan.output;
    let repo_name = plan.name.as_str();
    let tests = &plan.native_tests;
    git::clean(source)?;
    git::clean(projection)?;
    if output.exists() {
        return refuse("replay output already exists");
    }
    let (base, projected_commit, projected_tree, moves) = transform::maps(db, projection_id)?;
    if git::rev(projection, "HEAD")? != projected_commit
        || git::rev(projection, &format!("{projected_commit}^{{tree}}"))? != projected_tree
    {
        return refuse("projection not at verified revision");
    }
    let changed = git::rev(source, "HEAD")?;
    if scalar(source, &["rev-parse", "HEAD^"], false)? != base {
        return refuse("historical change must be a direct child of projection base");
    }
    let changed_set = git::changed(source, &base, &changed)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let allowed = manifest.paths.iter().cloned().collect::<BTreeSet<_>>();
    if allowed != changed_set {
        return refuse("historical source changed undeclared paths");
    }
    if moves.keys().any(|path| !allowed.contains(path)) {
        return refuse("projection includes move outside reviewed change scope");
    }
    let mut verified = Vec::new();
    for (i, p) in manifest.paths.iter().enumerate() {
        let before = git::blob(source, &base, p)?;
        let after = git::blob(source, &changed, p)?;
        let exact = before == manifest.old_blobs[i] && after == manifest.new_blobs[i];
        if !exact && (manifest.scope == "exact_changed_blobs" || p.ends_with(".test.ts")) {
            return refuse(format!("upstream blob identity mismatch: {p}"));
        }
        let target = moves.get(p).unwrap_or(p);
        let mode_before = scalar(source, &["ls-tree", &base, "--", p], false)?;
        let mode_target = scalar(
            projection,
            &["ls-tree", &projected_commit, "--", target],
            false,
        )?;
        if mode_before.split_whitespace().next() != mode_target.split_whitespace().next() {
            return refuse(format!("source file mode mismatch: {p}"));
        }
        // The pinned source blob must be present in the projected repository's object database.
        let present = Command::new("git")
            .arg("-C")
            .arg(projection)
            .args(["cat-file", "-e", &format!("{before}^{{blob}}")])
            .output()
            .map_err(|e| e.to_string())?;
        if !present.status.success() {
            return refuse(format!("missing Git base blob: {p}"));
        }
        verified.push((p.clone(), target.to_owned(), exact));
    }
    let original = run(
        source,
        &["diff", "--binary", "--full-index", &base, &changed, "--"],
        None,
        false,
    )?;
    let fixture = fs::read(&manifest.historic_patch).map_err(|e| e.to_string())?;
    if changed_lines(&original) != changed_lines(&fixture) {
        return refuse("real change additions/deletions differ from upstream patch");
    }
    let (mapped, seen) = rewrite_patch(&original, &allowed, &moves)?;
    if seen.len() != verified.len() {
        return refuse("changed path count mismatch");
    }
    let parent = output.parent().ok_or("output needs parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(
        ".relgit-replay-{}-{}",
        output
            .file_name()
            .ok_or("missing output basename")?
            .to_string_lossy(),
        std::process::id()
    ));
    if tmp.exists() {
        return refuse("temporary replay path exists");
    }
    let execute = || -> Result<Value> {
        clone(projection, &projected_commit, &tmp)?;
        run(
            &tmp,
            &["apply", "--3way", "--index", "-"],
            Some(&mapped),
            true,
        )?;
        let after = run(
            &tmp,
            &["diff", "--cached", "--name-only", "-z"],
            None,
            false,
        )?;
        let changed_paths = after
            .split(|b| *b == 0)
            .filter(|x| !x.is_empty())
            .map(strict_path)
            .collect::<Result<BTreeSet<_>>>()?;
        let expected = verified
            .iter()
            .map(|(_, mapped, _)| mapped.clone())
            .collect::<BTreeSet<_>>();
        if changed_paths != expected {
            return refuse("mapped Git merge touched undeclared files");
        }
        let result_commit = scalar(
            &tmp,
            &[
                "commit-tree",
                &scalar(&tmp, &["write-tree"], false)?,
                "-p",
                &projected_commit,
                "-m",
                &format!("relgit: mapped historical {}", &manifest.commit[..12]),
            ],
            true,
        )?;
        run(
            &tmp,
            &["update-ref", "HEAD", &result_commit, &projected_commit],
            None,
            false,
        )?;
        git::clean(&tmp)?;
        let result_tree = git::rev(&tmp, &format!("{result_commit}^{{tree}}"))?;
        for argv in tests {
            if argv.is_empty() {
                return refuse("empty post-merge verification");
            }
            let result = Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(&tmp)
                .env("PYTHONPATH", &tmp)
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .output()
                .map_err(|e| e.to_string())?;
            if !result.status.success() {
                return refuse(format!(
                    "native regression failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ));
            }
        }
        fs::rename(&tmp, output).map_err(|e| e.to_string())?;
        let source_id = index::index_revision_with_roots(
            db,
            source,
            plan.source_index_name.as_deref().unwrap_or(repo_name),
            &changed,
            "historical source",
            &[repo_name, "tests"],
        )?;
        let target_name = format!("{repo_name}-replayed");
        let target_id = index::index_revision_with_roots(
            db,
            output,
            &target_name,
            &result_commit,
            "mapped historical patch",
            &[repo_name, "tests"],
        )?;
        if index::reconstruct_tree(db, target_id, &result_commit)? != result_tree {
            return refuse("SQL vs Git replay tree mismatch");
        }
        let original_hash = digest::sha256(&original);
        let mapped_hash = digest::sha256(&mapped);
        let id = obj(&[
            ("projection", Value::string(projection_id)),
            ("source", Value::string(changed.clone())),
            ("target", Value::string(result_commit.clone())),
            ("manifest", Value::string(&manifest.digest)),
        ])
        .sha256();
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            db.execute(
                "INSERT INTO replay_provenance VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&id),
                    Param::Text(projection_id),
                    Param::Text(&manifest.upstream_repo),
                    Param::Text(&manifest.parent),
                    Param::Text(&manifest.commit),
                    Param::Integer(source_id),
                    Param::Text(&changed),
                    Param::Integer(target_id),
                    Param::Text(&result_commit),
                    Param::Text(&result_tree),
                    Param::Text(&manifest.scope),
                    Param::Text(&manifest.digest),
                    Param::Text(&original_hash),
                    Param::Text(&mapped_hash),
                ],
            )?;
            for (old, new, exact) in &verified {
                db.execute(
                    "INSERT INTO replay_path VALUES(?,?,?,?)",
                    &[
                        Param::Text(&id),
                        Param::Text(old),
                        Param::Text(new),
                        Param::Integer(i64::from(*exact)),
                    ],
                )?;
            }
            db.foreign_keys_valid()
        })?;
        Ok(obj(&[
            ("replay_id", Value::string(id)),
            ("upstream_commit", Value::string(&manifest.commit)),
            ("source_commit", Value::string(changed)),
            ("projected_commit", Value::string(result_commit)),
            ("projected_tree", Value::string(result_tree)),
            ("source_patch_sha256", Value::string(original_hash)),
            ("mapped_patch_sha256", Value::string(mapped_hash)),
            ("checked_paths", Value::number(verified.len())),
            (
                "exact_upstream_blobs",
                Value::number(verified.iter().filter(|(_, _, ok)| *ok).count()),
            ),
        ]))
    };
    let result = execute();
    if result.is_err() && tmp.exists() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

pub fn from_spec(path: &Path) -> Result<ReplayPlan> {
    let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
    let manifest = Manifest::read(Path::new(v.get("manifest")?.str()?))?;
    let source = PathBuf::from(v.get("source")?.str()?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let projection = PathBuf::from(v.get("projection")?.str()?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let projection_id = v.get("projection_id")?.str()?.to_owned();
    if projection_id.len() != 64 || !projection_id.bytes().all(|x| x.is_ascii_hexdigit()) {
        return refuse("bad projection identifier");
    }
    let output = PathBuf::from(v.get("output")?.str()?);
    let name = v.get("name")?.str()?.to_owned();
    let native_tests = v
        .get("native_tests")?
        .array_ref()?
        .iter()
        .map(|row| {
            row.array_ref()?
                .iter()
                .map(|x| Ok(x.str()?.to_owned()))
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ReplayPlan {
        manifest,
        source_index_name: None,
        source,
        projection,
        projection_id,
        output,
        name,
        native_tests,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_scope_refusal() {
        let allowed = BTreeSet::from(["src/a.rs".to_owned()]);
        let moves = BTreeMap::from([("src/a.rs".to_owned(), "src/b.rs".to_owned())]);
        assert!(rewrite_patch(b"diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n",&allowed,&moves).is_ok());
        assert!(
            rewrite_patch(
                b"diff --git a/src/../a.rs b/src/../a.rs\n--- a/src/../a.rs\n+++ b/src/../a.rs\n",
                &allowed,
                &moves
            )
            .is_err()
        );
        assert!(rewrite_patch(b"diff --git a/src/a.rs b/src/a.rs\nnew file mode 100755\n--- a/src/a.rs\n+++ b/src/a.rs\n",&allowed,&moves).is_err());
        assert!(
            rewrite_patch(
                b"diff --git a/src/a.rs b/src/b.rs\n--- a/src/a.rs\n+++ b/src/b.rs\n",
                &allowed,
                &moves
            )
            .is_err()
        );
    }
}
