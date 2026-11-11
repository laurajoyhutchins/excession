//! Verified source-layout transactions. Rust owns paths, Git, SQLite, and evidence;
//! CPython AST / TypeScript compiler adapters own syntax edits only.
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, json, refuse};
use json::{Value, obj};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Restore Git's tracked file modes without requiring POSIX permissions on Windows.
fn set_source_permissions(path: &Path, mode: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(if mode == "100755" { 0o755 } else { 0o644 });
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projection (
 id TEXT PRIMARY KEY,
 repo_id INTEGER NOT NULL REFERENCES repository(id),
 base_commit TEXT NOT NULL,
 adapter TEXT NOT NULL CHECK(adapter IN ('python','javascript')),
 policy TEXT NOT NULL CHECK(policy='compat-shim'),
 status TEXT NOT NULL CHECK(status='passed'),
 output_repo_id INTEGER NOT NULL REFERENCES repository(id),
 output_commit TEXT NOT NULL,
 output_tree TEXT NOT NULL,
 FOREIGN KEY(repo_id,base_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(output_repo_id,output_commit) REFERENCES revision(repo_id,commit_oid)
);
CREATE TABLE IF NOT EXISTS projection_move (
 projection_id TEXT NOT NULL REFERENCES projection(id),
 module_name TEXT NOT NULL,
 old_path TEXT NOT NULL,
 new_path TEXT NOT NULL,
 PRIMARY KEY(projection_id,module_name),
 UNIQUE(projection_id,old_path), UNIQUE(projection_id,new_path)
);
CREATE TABLE IF NOT EXISTS projection_identity (
 projection_id TEXT NOT NULL,
 logical_module TEXT NOT NULL,
 physical_module TEXT NOT NULL,
 PRIMARY KEY(projection_id,logical_module),
 FOREIGN KEY(projection_id,logical_module)
   REFERENCES projection_move(projection_id,module_name)
);
CREATE TABLE IF NOT EXISTS projection_check (
 projection_id TEXT NOT NULL REFERENCES projection(id),
 name TEXT NOT NULL,
 detail TEXT NOT NULL,
 PRIMARY KEY(projection_id,name)
);
"#;

#[derive(Clone, Debug)]
pub struct Move {
    pub logical: String,
    pub old: String,
    pub new: String,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub source: PathBuf,
    pub name: String,
    pub base: String,
    pub language: String,
    pub moves: Vec<Move>,
    pub probe: Vec<String>,
    pub compatibility: Vec<String>,
    pub adapter_path: PathBuf,
}
fn strings(v: &Value) -> Result<Vec<String>> {
    v.array_ref()?
        .iter()
        .map(|x| Ok(x.str()?.to_owned()))
        .collect()
}
impl Plan {
    pub fn parse(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        let source = PathBuf::from(v.get("source")?.str()?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let name = v.get("name")?.str()?.to_owned();
        let base = git::oid(v.get("base")?.str()?)?;
        let language = v.get("language")?.str()?.to_owned();
        let moves = v
            .get("moves")?
            .array_ref()?
            .iter()
            .map(|x| {
                Ok(Move {
                    logical: x.get("module")?.str()?.to_owned(),
                    old: x.get("old")?.str()?.to_owned(),
                    new: x.get("new")?.str()?.to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let probe = strings(v.get("probe")?)?;
        let compatibility = strings(v.get("compatibility")?)?;
        let adapter_path = PathBuf::from(v.get("adapter_script")?.str()?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            source,
            name,
            base,
            language,
            moves,
            probe,
            compatibility,
            adapter_path,
        })
    }
}

fn check_target(path: &str, language: &str) -> Result<()> {
    git::validate_path(path)?;
    if path.bytes().any(|x| x < 32) || path.split('/').any(|x| x.starts_with(".git")) {
        return refuse("unsafe projected path");
    }
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() < 2 {
        return refuse("projected path must be in package");
    }
    let extension = path.rsplit_once('.').map(|x| x.1).unwrap_or("");
    if (language == "python" && extension != "py")
        || (language == "javascript" && !matches!(extension, "js" | "cjs" | "mjs" | "ts"))
    {
        return refuse("target language extension mismatch");
    }
    if language == "python"
        && parts.iter().enumerate().any(|(i, x)| {
            let segment = if i + 1 == parts.len() {
                x.strip_suffix(".py").unwrap_or(x)
            } else {
                x
            };
            segment.is_empty()
                || !segment
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                || segment
                    .bytes()
                    .any(|b| !b.is_ascii_alphanumeric() && b != b'_')
        })
    {
        return refuse("invalid Python identifier in destination");
    }
    Ok(())
}
fn command(root: &Path, args: &[&str], env: bool, input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    if env {
        cmd.env("GIT_AUTHOR_NAME", "RelGit Test")
            .env("GIT_COMMITTER_NAME", "RelGit Test")
            .env("GIT_AUTHOR_EMAIL", "relgit@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "relgit@example.invalid")
            .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00+0000")
            .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00+0000");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(bytes) = input {
        child
            .stdin
            .take()
            .ok_or("stdin unavailable")?
            .write_all(bytes)
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}
fn git_string(root: &Path, args: &[&str], env: bool) -> Result<String> {
    Ok(String::from_utf8(command(root, args, env, None)?)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned())
}
fn probe(root: &Path, argv: &[String]) -> Result<Vec<u8>> {
    if argv.is_empty() {
        return refuse("empty runtime oracle command");
    }
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .env("PYTHONPATH", root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
        .output()
        .map_err(|e| format!("oracle launch: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "runtime oracle refused: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
fn syntax_adapter(
    plan: &Plan,
    checkout: &Path,
    entries: &[git::Entry],
    moves: &BTreeMap<String, String>,
    modules: &BTreeSet<String>,
) -> Result<Vec<Value>> {
    let mut files = Vec::new();
    let mut paths = Vec::new();
    for e in entries {
        if e.kind != "blob" {
            continue;
        }
        paths.push(Value::string(&e.path));
        if !matches!(e.mode.as_str(), "100644" | "100755") {
            continue;
        }
        if plan.language == "python" && !e.path.ends_with(".py") {
            continue;
        }
        if plan.language == "javascript"
            && (![".js", ".mjs", ".cjs", ".ts"]
                .iter()
                .any(|x| e.path.ends_with(x))
                || e.path.ends_with(".d.ts")
                || e.path.contains("/node_modules/"))
        {
            continue;
        }
        let raw = fs::read(checkout.join(&e.path)).map_err(|e| e.to_string())?;
        if digest::git_blob(&raw) != e.oid {
            return refuse(format!("working source not Git-authenticated: {}", e.path));
        }
        let content = String::from_utf8(raw).map_err(|_| "non UTF8 source unsupported")?;
        files.push(obj(&[
            ("path", Value::string(&e.path)),
            ("content", Value::string(content)),
        ]));
    }
    let values = if plan.language == "python" {
        obj(&[
            ("files", Value::array(files)),
            (
                "moves",
                Value::object(moves.iter().map(|(k, v)| (k.clone(), Value::string(v)))),
            ),
            ("internal", Value::array(modules.iter().map(Value::string))),
        ])
    } else {
        obj(&[
            ("files", Value::array(files)),
            ("paths", Value::array(paths)),
            (
                "moves",
                Value::object(moves.iter().map(|(k, v)| (k.clone(), Value::string(v)))),
            ),
        ])
    };
    let mut cmd = Command::new(if plan.language == "python" {
        "python"
    } else {
        "node"
    });
    cmd.arg(&plan.adapter_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("native adapter launch: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("native adapter stdin")?
        .write_all(values.canonical().as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "{} syntax adapter refused: {}",
            plan.language,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let parsed = json::parse(&out.stdout)?;
    Ok(parsed.array_ref()?.to_vec())
}

fn clone_detached(source: &Path, base: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        return refuse("destination already exists");
    }
    let output = Command::new("git")
        .args(["clone", "--local", "--no-hardlinks", "--quiet"])
        .arg(source)
        .arg(dest)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return refuse(format!(
            "clone refused: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    command(
        dest,
        &["checkout", "--quiet", "--detach", base],
        false,
        None,
    )?;
    git::clean(dest)
}
fn projection_id(p: &Plan) -> String {
    let moves: Vec<Value> = p
        .moves
        .iter()
        .map(|m| {
            obj(&[
                ("module", Value::string(&m.logical)),
                ("old_path", Value::string(&m.old)),
                ("new_path", Value::string(&m.new)),
            ])
        })
        .collect();
    obj(&[
        ("repo", Value::string(&p.name)),
        ("base", Value::string(&p.base)),
        ("adapter", Value::string(&p.language)),
        ("policy", Value::string("compat-shim")),
        ("moves", Value::array(moves)),
    ])
    .sha256()
}
fn symbol_rows(db: &Db, repo: i64, commit: &str, name: &str) -> Result<BTreeSet<String>> {
    Ok(db.query("SELECT qualname,kind,signature FROM symbol WHERE repo_id=? AND commit_oid=? AND module_name=?",
  &[Param::Integer(repo),Param::Text(commit),Param::Text(name)])?.iter().map(|r|
  r.iter().map(|x|x.as_deref().unwrap_or("")).collect::<Vec<_>>().join("\0")).collect())
}
/// Refuse to write through a symlinked Git path, even inside a disposable clone.
fn refuse_symlink_ancestors(root: &Path, relative: &str) -> Result<()> {
    git::validate_path(relative)?;
    let mut current = root.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return refuse(format!("symlink in generated source path: {relative}"));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return refuse(format!("cannot inspect generated source path: {e}")),
        }
    }
    Ok(())
}
fn do_projection(db: &Db, p: &Plan, dest: &Path, temp: &Path) -> Result<Value> {
    if !matches!(p.language.as_str(), "python" | "javascript") || p.moves.is_empty() {
        return refuse("unsupported language or empty projection");
    }
    git::clean(&p.source)?;
    if git::rev(&p.source, "HEAD")? != p.base {
        return refuse("source HEAD not pinned base");
    }
    let source_id = index::index_revision_with_roots(
        db,
        &p.source,
        &p.name,
        &p.base,
        "project source",
        &[&p.name, "tests"],
    )?;
    let entries = git::entries(&p.source, &p.base)?;
    let entry_by_path: BTreeMap<_, _> = entries
        .iter()
        .filter(|x| x.kind == "blob")
        .map(|x| (x.path.as_str(), x))
        .collect();
    let mut old = BTreeSet::new();
    let mut new = BTreeSet::new();
    let mut moves = BTreeMap::new();
    let mut module_set = BTreeSet::new();
    for r in db.query(
        "SELECT name FROM module WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(source_id), Param::Text(&p.base)],
    )? {
        module_set.insert(r[0].clone().ok_or("null module")?);
    }
    for m in &p.moves {
        git::validate_path(&m.old)?;
        check_target(&m.new, &p.language)?;
        let suffix = m.old.rsplit_once('.').map(|x| x.1).unwrap_or("");
        if suffix != m.new.rsplit_once('.').map(|x| x.1).unwrap_or("") {
            return refuse("file extension changed");
        }
        if m.old.ends_with("/__init__.py") || m.old.ends_with("/index.js") {
            return refuse("moving package entrypoint unsupported");
        }
        let src = entry_by_path
            .get(m.old.as_str())
            .ok_or("missing source module")?;
        if !matches!(src.mode.as_str(), "100644" | "100755") {
            return refuse("cannot move non-regular source");
        }
        if entry_by_path.contains_key(m.new.as_str()) || entries.iter().any(|x| x.path == m.new) {
            return refuse("destination collision");
        }
        let (name, _) = index::module_name(&m.old, &[&p.name, "tests"])
            .ok_or("source path cannot be a module")?;
        if name != m.logical || !module_set.contains(&name) {
            return refuse("module identity does not match indexed source");
        }
        if index::module_name(&m.new, &[&p.name, "tests"]).is_none() {
            return refuse("destination outside package roots");
        }
        if !old.insert(&m.old) || !new.insert(&m.new) {
            return refuse("duplicate moved source or target");
        }
        moves.insert(m.old.clone(), m.new.clone());
    }
    clone_detached(&p.source, &p.base, temp)?;
    let baseline = probe(temp, &p.probe)?;
    let generated = syntax_adapter(p, temp, &entries, &moves, &module_set)?;
    let mut expected = BTreeSet::new();
    let mut written = BTreeMap::<String, Vec<u8>>::new();
    let mut rewrites = 0usize;
    for e in &entries {
        if e.kind == "blob"
            && matches!(e.mode.as_str(), "100644" | "100755")
            && ((p.language == "python" && e.path.ends_with(".py"))
                || (p.language == "javascript"
                    && [".js", ".mjs", ".cjs", ".ts"]
                        .iter()
                        .any(|x| e.path.ends_with(x))
                    && !e.path.ends_with(".d.ts")
                    && !e.path.contains("/node_modules/")))
        {
            expected.insert(e.path.as_str());
        }
    }
    let mut received = BTreeSet::new();
    for row in &generated {
        let oldp = row.get("old_path")?.str()?;
        let newp = row.get("new_path")?.str()?;
        git::validate_path(oldp)?;
        git::validate_path(newp)?;
        if !expected.contains(oldp) || !received.insert(oldp.to_owned()) {
            return refuse("native adapter emitted unexpected or duplicate source");
        }
        let correct = moves.get(oldp).map(String::as_str).unwrap_or(oldp);
        if correct != newp {
            return refuse("native adapter output path not declared");
        }
        let data = row.get("content")?.str()?.as_bytes().to_vec();
        rewrites += row.get("edits")?.array_ref()?.len();
        if written.insert(newp.to_owned(), data).is_some() {
            return refuse("native adapter output collision");
        }
    }
    if received.len() != expected.len() {
        return refuse("native adapter omitted source");
    }
    for oldp in moves.keys() {
        fs::remove_file(temp.join(oldp)).map_err(|e| e.to_string())?;
    }
    for (path, content) in &written {
        refuse_symlink_ancestors(temp, path)?;
        let target = temp.join(path);
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(&target, content).map_err(|e| e.to_string())?;
        let source_path = moves
            .iter()
            .find(|(_, new)| *new == path)
            .map(|(old, _)| old.as_str())
            .unwrap_or(path);
        let e = entry_by_path
            .get(source_path)
            .ok_or("source file mode missing")?;
        set_source_permissions(&target, &e.mode)?;
    }
    for m in &p.moves {
        let oldp = temp.join(&m.old);
        let src = entry_by_path[m.old.as_str()];
        let shim = if p.language == "python" {
            let newparts = m.new.split('/').collect::<Vec<_>>();
            for level in 2..newparts.len() {
                let relative = format!("{}/__init__.py", newparts[..level].join("/"));
                refuse_symlink_ancestors(temp, &relative)?;
                let init = temp.join(relative);
                if !init.exists() {
                    fs::create_dir_all(init.parent().ok_or("bad init parent")?)
                        .map_err(|e| e.to_string())?;
                    fs::write(init, b"").map_err(|e| e.to_string())?;
                }
            }
            let physical = index::module_name(&m.new, &[&p.name, "tests"])
                .ok_or("bad target module")?
                .0;
            format!(
                "# Generated compatibility alias for {}.\nimport importlib as _relgit_importlib\nimport sys as _relgit_sys\n_relgit_sys.modules[__name__] = _relgit_importlib.import_module('{}')\n",
                m.old, physical
            )
        } else {
            let parent = m
                .old
                .rsplit_once('/')
                .map(|x| x.0)
                .ok_or("invalid JS source")?;
            let oldparts = parent.split('/').collect::<Vec<_>>();
            let newparts = m.new.split('/').collect::<Vec<_>>();
            let mut common = 0;
            while common < oldparts.len().min(newparts.len())
                && oldparts[common] == newparts[common]
            {
                common += 1;
            }
            let mut route = vec![".."; oldparts.len() - common];
            route.extend_from_slice(&newparts[common..]);
            let mut spec = route.join("/");
            if !spec.starts_with('.') {
                spec = format!("./{spec}");
            }
            if spec.ends_with(".js") {
                spec.truncate(spec.len() - 3)
            }
            format!(
                "'use strict';\nmodule.exports = require({})\n",
                Value::string(spec).canonical()
            )
        };
        fs::write(&oldp, shim).map_err(|e| e.to_string())?;
        set_source_permissions(&oldp, &src.mode)?;
        set_source_permissions(&temp.join(&m.new), &src.mode)?;
    }
    let after = probe(temp, &p.probe)?;
    if baseline != after {
        return refuse("baseline and projected runtime outputs differ");
    }
    probe(temp, &p.compatibility)?;
    command(temp, &["add", "-A"], false, None)?;
    let tree = git_string(temp, &["write-tree"], false)?;
    let id = projection_id(p);
    let commit = git_string(
        temp,
        &[
            "commit-tree",
            &tree,
            "-p",
            &p.base,
            "-m",
            &format!("relgit: deterministic module projection {}", &id[..12]),
        ],
        true,
    )?;
    command(temp, &["update-ref", "HEAD", &commit, &p.base], false, None)?;
    if git::rev(temp, &format!("{commit}^{{tree}}"))? != tree {
        return refuse("Git tree readback mismatch");
    }
    git::clean(temp)?;
    fs::rename(temp, dest).map_err(|e| e.to_string())?;
    let dest_name = format!("{}-projected", p.name);
    let target_id = index::index_revision_with_roots(
        db,
        dest,
        &dest_name,
        &commit,
        "projected source",
        &[&p.name, "tests"],
    )?;
    let verified = index::reconstruct_tree(db, target_id, &commit)?;
    if verified != tree {
        return refuse("SQL tree reconstruction differs");
    }
    let mut ident = Vec::new();
    for m in &p.moves {
        let physical = index::module_name(&m.new, &[&p.name, "tests"])
            .ok_or("invalid output identity")?
            .0;
        if symbol_rows(db, source_id, &p.base, &m.logical)?
            != symbol_rows(db, target_id, &commit, &physical)?
        {
            return refuse(format!(
                "logical declarations not preserved for {}",
                m.logical
            ));
        }
        ident.push((m, physical));
    }
    db.exec(SCHEMA)?;
    db.transaction(|db| {
        db.execute(
            "INSERT INTO projection VALUES(?,?,?,?,?,?,?,?,?)",
            &[
                Param::Text(&id),
                Param::Integer(source_id),
                Param::Text(&p.base),
                Param::Text(&p.language),
                Param::Text("compat-shim"),
                Param::Text("passed"),
                Param::Integer(target_id),
                Param::Text(&commit),
                Param::Text(&tree),
            ],
        )?;
        for (m, physical) in &ident {
            db.execute(
                "INSERT INTO projection_move VALUES(?,?,?,?)",
                &[
                    Param::Text(&id),
                    Param::Text(&m.logical),
                    Param::Text(&m.old),
                    Param::Text(&m.new),
                ],
            )?;
            db.execute(
                "INSERT INTO projection_identity VALUES(?,?,?)",
                &[
                    Param::Text(&id),
                    Param::Text(&m.logical),
                    Param::Text(physical),
                ],
            )?;
        }
        let checks = [
            ("behavior-equal", digest::sha256(&after)),
            ("git-tree", tree.clone()),
            (
                "logical-declarations",
                format!("{} moved modules", ident.len()),
            ),
            ("compatibility-probe", String::from("passed")),
        ];
        for (name, detail) in checks {
            db.execute(
                "INSERT INTO projection_check VALUES(?,?,?)",
                &[Param::Text(&id), Param::Text(name), Param::Text(&detail)],
            )?;
        }
        db.foreign_keys_valid()
    })?;
    Ok(obj(&[
        ("id", Value::string(id)),
        ("base", Value::string(&p.base)),
        ("repo", Value::string(&p.name)),
        ("projection_commit", Value::string(commit)),
        ("projection_tree", Value::string(tree)),
        ("edit_count", Value::number(rewrites)),
        ("moves", Value::number(ident.len())),
        ("behavior_sha256", Value::string(digest::sha256(&after))),
    ]))
}

pub fn project(db: &Db, p: &Plan, dest: &Path) -> Result<Value> {
    if dest.exists() {
        return refuse("refusing existing output destination");
    }
    if dest.canonicalize().is_ok() {
        return refuse("output already exists");
    }
    if dest.starts_with(&p.source) || p.source.starts_with(dest) {
        return refuse("output may not enclose source");
    }
    let parent = dest.parent().ok_or("output needs parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stem = dest
        .file_name()
        .ok_or("missing output filename")?
        .to_string_lossy();
    let temp = parent.join(format!(
        ".relgit-projection-{}-{}",
        stem,
        std::process::id()
    ));
    if temp.exists() {
        return refuse("temporary projection path already exists");
    }
    let result = do_projection(db, p, dest, &temp);
    if result.is_err() && temp.exists() {
        let _ = fs::remove_dir_all(&temp);
    }
    result
}

pub fn maps(
    db: &Db,
    projection_id: &str,
) -> Result<(String, String, String, BTreeMap<String, String>)> {
    db.exec(SCHEMA)?;
    let rows=db.query("SELECT base_commit,output_commit,output_tree FROM projection WHERE id=? AND status='passed'",
  &[Param::Text(projection_id)])?;
    if rows.len() != 1 {
        return refuse("unknown or unverified projection");
    }
    let unpack = |i: usize| rows[0][i].clone().ok_or("NULL projection field");
    let mut mapping = BTreeMap::new();
    for row in db.query(
        "SELECT old_path,new_path FROM projection_move WHERE projection_id=?",
        &[Param::Text(projection_id)],
    )? {
        let old = row[0].clone().ok_or("NULL source path")?;
        let new = row[1].clone().ok_or("NULL target path")?;
        if mapping.insert(old, new).is_some() {
            return refuse("duplicate projection path");
        }
    }
    Ok((unpack(0)?, unpack(1)?, unpack(2)?, mapping))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn symlinked_destination_parent_is_refused() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("relgit-symlink-check-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pkg")).unwrap();
        symlink("/tmp", root.join("pkg/redirect")).unwrap();
        assert!(refuse_symlink_ancestors(&root, "pkg/redirect/external.py").is_err());
        assert!(refuse_symlink_ancestors(&root, "pkg/real/ordinary.py").is_ok());
        fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn target_refusals() {
        assert!(check_target("../src/foo.py", "python").is_err());
        assert!(check_target("pkg/../x.py", "python").is_err());
        assert!(check_target("pkg/bad-name.py", "python").is_err());
        assert!(check_target("pkg/index.ts", "python").is_err());
        assert!(check_target("pkg/engine/parser.js", "javascript").is_ok());
        assert!(check_target("pkg/protocol/_state.py", "python").is_ok());
    }
}
