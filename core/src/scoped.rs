//! Conservative repository-independent local test-evidence scheduling.
//!
//! This is a conditional, declared-scope contract, NOT semantic equivalence,
//! signed attestation, a sandbox, or permission to omit joint invariants.
//! All executable source that differs must be reverified; only explicitly
//! reviewed non-execution documentation may differ for evidence transfer.
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Suite {
    id: String,
    argv: Vec<String>,
}

fn required_string(v: &Value, k: &str) -> Result<String> {
    let s = v.get(k)?.str()?;
    if s.is_empty() {
        return refuse(format!("empty {k}"));
    }
    Ok(s.into())
}

fn unique_suites(v: &Value, name: &str) -> Result<Vec<Suite>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in v.get(name)?.array_ref()? {
        let id = required_string(item, "id")?;
        if !seen.insert(id.clone()) {
            return refuse("duplicate suite ID");
        }
        let argv = item
            .get("argv")?
            .array_ref()?
            .iter()
            .map(|s| s.str().map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        if argv.is_empty()
            || argv.len() > 32
            || argv.iter().any(|s| s.is_empty() || s.contains('\0'))
        {
            return refuse("invalid native test argv");
        }
        // No shell, arbitrary PATH search, or proxy executable. Commands are
        // reviewed and executed only in explicitly trusted local mode.
        if !["python3", "node"].contains(&argv[0].as_str()) {
            return refuse("unsupported native test interpreter");
        }
        out.push(Suite { id, argv });
    }
    if out.is_empty() {
        return refuse(format!("{name} needs at least one suite"));
    }
    Ok(out)
}

fn inert_path(s: &str) -> bool {
    // Even these may influence dynamic data loads. The contract *assumes*
    // their non-execution; this is not established by filename alone.
    s == "README.md" || (s.starts_with("docs/") && s.ends_with(".md"))
}

#[derive(Clone)]
struct Origin {
    kind: String,
    name: String,
    file: String,
}

fn origin_from_spec(spec: &Value) -> Result<Origin> {
    let origin = spec.get("runtime_origin")?;
    let kind = required_string(origin, "kind")?;
    let name = required_string(origin, "name")?;
    let file = required_string(origin, "file")?;
    git::validate_path(&file)?;
    if !["python", "node"].contains(&kind.as_str()) {
        return refuse("unsupported origin language");
    }
    if kind == "python"
        && (!name
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            || !file.ends_with(".py"))
    {
        return refuse("invalid python origin module");
    }
    if kind == "node"
        && (!name.split('/').all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }) || !file.ends_with(".js"))
    {
        return refuse("invalid node origin module");
    }
    Ok(Origin { kind, name, file })
}

const PY_NATIVE: &str = r#"import importlib, pathlib, sys
module, expected, mode = sys.argv[1:4]
obj = importlib.import_module(module)
actual = pathlib.Path(obj.__file__).resolve()
target = (pathlib.Path.cwd() / expected).resolve()
if actual != target:
    sys.exit('REFUSED: Python imported an unpinned package origin')
if mode == 'pytest':
    import pytest
    sys.exit(pytest.main(sys.argv[4:]))
if mode == 'code':
    exec(sys.argv[4], {'__name__':'__main__'})
else:
    sys.exit('REFUSED: invalid Python test mode')
"#;
const JS_NATIVE: &str = r#"'use strict';
const p=require('node:path');
const moduleDir=process.argv[1], expected=process.argv[2], script=process.argv[3];
if (require.resolve(p.resolve(process.cwd(),moduleDir)) !== p.resolve(process.cwd(),expected)) {
  throw Error('REFUSED: Node imported an unpinned package origin');
}
require(p.resolve(process.cwd(),script));
"#;

fn native_command(suite: &Suite, origin: &Origin) -> Result<Command> {
    let argv = &suite.argv;
    match (argv[0].as_str(), origin.kind.as_str()) {
        ("python3", "python") if argv.len() >= 5 && argv[1] == "-m" && argv[2] == "pytest" => {
            let mut c = crate::adapter::python_command();
            c.args(["-c", PY_NATIVE, &origin.name, &origin.file, "pytest"])
                .args(&argv[3..]);
            Ok(c)
        }
        ("python3", "python") if argv.len() == 3 && argv[1] == "-c" => {
            let mut c = crate::adapter::python_command();
            c.args([
                "-c",
                PY_NATIVE,
                &origin.name,
                &origin.file,
                "code",
                &argv[2],
            ]);
            Ok(c)
        }
        ("node", "node") if argv.len() == 2 && !argv[1].starts_with('-') => {
            git::validate_path(&argv[1])?;
            let mut c = crate::adapter::node_command();
            c.args(["-e", JS_NATIVE, &origin.name, &origin.file, &argv[1]]);
            Ok(c)
        }
        ("node", "node") if argv.len() == 3 && argv[1] == "--check" => {
            git::validate_path(&argv[2])?;
            let mut c = crate::adapter::node_command();
            c.args(["--check", &argv[2]]);
            Ok(c)
        }
        _ => refuse("native command does not match approved runtime adapter form"),
    }
}

fn run_native(path: &Path, suite: &Suite, origin: &Origin, timeout: Duration) -> Result<Value> {
    let start = Instant::now();
    let mut cmd = native_command(suite, origin)?;
    cmd.current_dir(path)
        .env("PYTHONPATH", path)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
        .env("PYTHONHASHSEED", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("native suite {} could not start: {e}", suite.id))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("suite wait: {e}"))? {
            if !status.success() {
                return refuse(format!("native suite {} failed with {status}", suite.id));
            }
            return Ok(obj(&[
                ("id", Value::string(&suite.id)),
                ("passed", Value::Bool(true)),
                ("elapsed_ms", Value::number(start.elapsed().as_millis())),
            ]));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return refuse(format!("native suite {} timed out", suite.id));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn repo_dir(root: &Path, name: &str) -> Result<PathBuf> {
    git::validate_path(name)?;
    if name.contains('/') {
        return refuse("checkout name must be one directory");
    }
    let actual = root
        .join(name)
        .canonicalize()
        .map_err(|e| format!("checkout {name}: {e}"))?;
    if actual.parent() != Some(root) {
        return refuse("checkout escaped workspace");
    }
    git::clean(&actual)?;
    Ok(actual)
}

fn checked_revision(path: &Path, oid: &str) -> Result<String> {
    git::oid(oid)?;
    let actual = git::rev(path, "HEAD")?;
    if actual != oid {
        return refuse(format!(
            "untrusted pinned checkout revision {}",
            path.display()
        ));
    }
    git::rev(path, "HEAD^{tree}")
}

fn same_execution_tree(
    branch: &[git::Entry],
    merge: &[git::Entry],
    inert: &BTreeSet<String>,
) -> bool {
    let filtered = |entries: &[git::Entry]| {
        entries
            .iter()
            .filter(|e| !inert.contains(&e.path))
            .map(|e| {
                (
                    e.path.clone(),
                    (e.mode.clone(), e.kind.clone(), e.oid.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    filtered(branch) == filtered(merge)
}

fn check_merge(path: &Path, base: &str, a: &str, b: &str, merge: &str) -> Result<()> {
    if git::rev(path, "HEAD^1")? != a || git::rev(path, "HEAD^2")? != b {
        return refuse("merge parents are not the pinned independent branches");
    }
    if git::text(path, &["merge-base", a, b])? != base {
        return refuse("branches have a different merge-base");
    }
    let auto = git::text(path, &["merge-tree", "--write-tree", a, b])?;
    if auto != git::rev(path, "HEAD^{tree}")? || git::rev(path, "HEAD")? != merge {
        return refuse("manual, conflicted or noncanonical merge cannot carry scoped evidence");
    }
    Ok(())
}

/// Execute pinned branch suites once. On merge, a suite may use a branch
/// observation if that branch has exactly the merged tracked execution tree
/// (apart from explicitly reviewed non-executable documentation). Otherwise
/// it is freshly run on the merged checkout. Joint suites always run.
/// The manifest and sources MUST have been independently reviewed: this local
/// executor provides no sandbox and cannot authenticate remote attestations.
pub fn carry_scoped(
    spec_file: &Path,
    database: Option<&Path>,
    trusted_local: bool,
) -> Result<Value> {
    if !trusted_local {
        return refuse("explicit --trusted-local required; execution is not sandboxed");
    }
    let spec_bytes = std::fs::read(spec_file).map_err(|e| format!("spec: {e}"))?;
    let spec = json::parse(&spec_bytes)?;
    if spec.get("schema")?.str()? != "relgit-scoped-v1" {
        return refuse("unsupported scoped manifest schema");
    }
    let root = PathBuf::from(required_string(&spec, "root")?)
        .canonicalize()
        .map_err(|e| format!("workspace: {e}"))?;
    if !root.is_dir() {
        return refuse("workspace is not a directory");
    }
    let folders = spec.get("folders")?;
    let paths = ["base", "a", "b", "merge"].map(|k| repo_dir(&root, &required_string(folders, k)?));
    let [base_path, a_path, b_path, merge_path] = paths
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .map_err(|_| "invalid checkout count")?;
    let revision = spec.get("revisions")?;
    let base = required_string(revision, "base")?;
    let a = required_string(revision, "a")?;
    let b = required_string(revision, "b")?;
    let merge = required_string(revision, "merge")?;
    let base_tree = checked_revision(&base_path, &base)?;
    let a_tree = checked_revision(&a_path, &a)?;
    let b_tree = checked_revision(&b_path, &b)?;
    let merge_tree = checked_revision(&merge_path, &merge)?;
    check_merge(&merge_path, &base, &a, &b, &merge)?;
    // Restrict exemptions to exact, tracked, non-symlink Markdown paths.
    let mut inert = BTreeSet::<String>::new();
    for v in spec.get("declared_non_execution")?.array_ref()? {
        let path = v.str()?;
        git::validate_path(path)?;
        if !inert_path(path) || !inert.insert(path.to_owned()) {
            return refuse(format!(
                "unsupported or duplicated non-execution path {path}"
            ));
        }
    }
    let trees = [
        git::entries(&base_path, &base)?,
        git::entries(&a_path, &a)?,
        git::entries(&b_path, &b)?,
        git::entries(&merge_path, &merge)?,
    ];
    for p in &inert {
        for tree in &trees {
            if tree
                .iter()
                .any(|e| e.path == *p && (e.kind != "blob" || e.mode != "100644"))
            {
                return refuse("non-execution exception is not a regular file");
            }
        }
    }
    let suites_a = unique_suites(&spec, "agent_a_suites")?;
    let suites_b = unique_suites(&spec, "agent_b_suites")?;
    let joint = unique_suites(&spec, "joint_suites")?;
    let origin = origin_from_spec(&spec)?;
    for tree in &trees {
        if !tree.iter().any(|e| {
            e.path == origin.file
                && e.kind == "blob"
                && matches!(e.mode.as_str(), "100644" | "100755")
        }) {
            return refuse("runtime entry is not a tracked regular source file");
        }
    }
    let timeout_secs = spec.get("timeout_seconds")?.integer()?;
    if !(1..=300).contains(&timeout_secs) {
        return refuse("invalid timeout budget");
    }
    let timeout = Duration::from_secs(u64::try_from(timeout_secs).map_err(|_| "invalid timeout")?);
    let t0 = Instant::now();
    let (results_a, results_b) = std::thread::scope(|scope| {
        let wa = scope.spawn(|| {
            suites_a
                .iter()
                .map(|s| run_native(&a_path, s, &origin, timeout))
                .collect::<Result<Vec<_>>>()
        });
        let wb = scope.spawn(|| {
            suites_b
                .iter()
                .map(|s| run_native(&b_path, s, &origin, timeout))
                .collect::<Result<Vec<_>>>()
        });
        let ra = wa.join().map_err(|_| "branch A worker panicked")??;
        let rb = wb.join().map_err(|_| "branch B worker panicked")??;
        Ok::<_, String>((ra, rb))
    })?;
    let witness_candidates = [
        ("a", &suites_a, &trees[1], &a_tree),
        ("b", &suites_b, &trees[2], &b_tree),
    ];
    let mut decisions = Vec::new();
    let mut fresh = BTreeMap::<Vec<String>, Value>::new();
    let mut carried = 0;
    let mut reverified = 0;
    for (branch_name, suites, _, _) in witness_candidates {
        for suite in suites {
            let witness = [
                (&suites_a[..], &trees[1], "a"),
                (&suites_b[..], &trees[2], "b"),
            ]
            .iter()
            .find_map(|(candidates, tree, witness_name)| {
                (same_execution_tree(tree, &trees[3], &inert)
                    && candidates.iter().any(|s| s.argv == suite.argv))
                .then_some(*witness_name)
            });
            let decision = if let Some(witness_name) = witness {
                carried += 1;
                obj(&[
                    ("suite", Value::string(&suite.id)),
                    ("branch", Value::string(branch_name)),
                    ("status", Value::string("CARRIED_DECLARED_SCOPE")),
                    ("observed_on", Value::string(witness_name)),
                ])
            } else {
                if !fresh.contains_key(&suite.argv) {
                    fresh.insert(
                        suite.argv.clone(),
                        run_native(&merge_path, suite, &origin, timeout)?,
                    );
                    reverified += 1;
                }
                obj(&[
                    ("suite", Value::string(&suite.id)),
                    ("branch", Value::string(branch_name)),
                    ("status", Value::string("FRESH_MERGE_REVERIFICATION")),
                    ("observed_on", Value::string("merge")),
                ])
            };
            decisions.push(decision);
        }
    }
    let mut joint_results = Vec::new();
    for suite in &joint {
        joint_results.push(run_native(&merge_path, suite, &origin, timeout)?);
    }
    // A digest of checked facts only. Timing, machine-local paths, and unsigned
    // console output do not participate in the provenance fingerprint.
    // Do not hash host-local checkout paths. Immutable Git revisions and
    // executable manifest policy, not the absolute VM path, identify evidence.
    let mut portable_spec = spec.clone();
    portable_spec.entries_mut()?.remove("root");
    let policy_sha = portable_spec.sha256();
    let facts = obj(&[
        ("manifest_sha256", Value::string(&policy_sha)),
        ("base", Value::string(base)),
        ("branch_a", Value::string(a)),
        ("branch_b", Value::string(b)),
        ("merge", Value::string(&merge)),
        ("base_tree", Value::string(base_tree)),
        ("a_tree", Value::string(a_tree)),
        ("b_tree", Value::string(b_tree)),
        ("merge_tree", Value::string(&merge_tree)),
        ("decisions", Value::array(decisions.clone())),
        (
            "joint_passed",
            Value::array(joint.iter().map(|s| Value::string(&s.id))),
        ),
        (
            "assumption",
            Value::string(
                "Declared markdown paths are not runtime inputs; source closure is all other tracked files; same local native execution environment",
            ),
        ),
    ]);
    let digest = facts.sha256();
    let result = obj(&[
        ("status", Value::string("ACCEPTED_CONDITIONAL_LOCAL")),
        (
            "assurance",
            Value::string(
                "Only declared-scope finite observations, no universal equivalence or remote attestation",
            ),
        ),
        ("guard_sha256", Value::string(&digest)),
        ("facts", facts),
        ("branch_a_results", Value::array(results_a)),
        ("branch_b_results", Value::array(results_b)),
        ("joint_results", Value::array(joint_results)),
        ("carried", Value::number(carried)),
        ("fresh_merged_suites", Value::number(reverified)),
        ("elapsed_ms", Value::number(t0.elapsed().as_millis())),
    ]);
    if let Some(path) = database {
        let db = Db::open(path)?;
        db.exec("CREATE TABLE IF NOT EXISTS scoped_run (id INTEGER PRIMARY KEY, guard_sha256 TEXT NOT NULL, manifest_sha256 TEXT NOT NULL, merge_commit TEXT NOT NULL, merge_tree TEXT NOT NULL, result_json TEXT NOT NULL);")?;
        let json = result.canonical();
        db.transaction(|d| {
            d.execute("INSERT INTO scoped_run(guard_sha256,manifest_sha256,merge_commit,merge_tree,result_json) VALUES(?,?,?,?,?)",&[
                Param::Text(&digest),Param::Text(&policy_sha),Param::Text(&merge),Param::Text(&merge_tree),Param::Text(&json),
            ])
        })?;
        db.foreign_keys_valid()?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inert_policy_is_narrow() {
        for good in ["README.md", "docs/protocol.md"] {
            assert!(inert_path(good));
        }
        for bad in [
            "src/lib.rs",
            "lib/parser.js",
            "tests/test_bridge.py",
            "package.json",
            "docs/run.py",
            "CHANGELOG.md",
        ] {
            assert!(!inert_path(bad));
        }
    }
    #[test]
    fn test_inputs_require_unique_ids_and_known_runners() {
        let bad = json::parse(br#"{"tests":[{"id":"x","argv":["sh","-c","true"]}]}"#).unwrap();
        assert!(unique_suites(&bad, "tests").is_err());
        let dup=json::parse(br#"{"tests":[{"id":"x","argv":["node","--version"]},{"id":"x","argv":["node","--version"]}]}"#).unwrap();
        assert!(unique_suites(&dup, "tests").is_err());
    }
    #[test]
    fn full_tree_equality_includes_file_modes() {
        let mut a = vec![git::Entry {
            path: "src/a.py".into(),
            kind: "blob".into(),
            mode: "100644".into(),
            oid: "a".into(),
        }];
        let mut b = a.clone();
        assert!(same_execution_tree(&a, &b, &BTreeSet::new()));
        b[0].mode = "100755".into();
        assert!(!same_execution_tree(&a, &b, &BTreeSet::new()));
        a.push(git::Entry {
            path: "README.md".into(),
            kind: "blob".into(),
            mode: "100644".into(),
            oid: "a".into(),
        });
        b[0].mode = "100644".into();
        let inert = BTreeSet::from(["README.md".into()]);
        assert!(same_execution_tree(&a, &b, &inert));
        assert!(!same_execution_tree(&a, &b, &BTreeSet::new()));
    }
}
