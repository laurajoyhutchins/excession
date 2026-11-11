//! Semantic transactions: compose a verified structural projection and a separately
//! developed old-layout defect fix, with a real two-parent explanatory Git commit.
//! Local, trusted execution only. Evidence is finite and conditional, not a theorem.
//! Test receipts record command identity and exit status, not nondeterministic pytest stdout.
use crate::sqlite::{Db, Param};
use crate::{Result, git, json, refuse};
use json::{Value, obj};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod execution;
mod verification;

pub use execution::compose;
pub use verification::audit;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_transaction (
  tx_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('module_move','historical_fix','composition')),
  logical_target TEXT NOT NULL,
  source_commit TEXT NOT NULL,
  result_commit TEXT NOT NULL,
  result_tree TEXT NOT NULL,
  evidence_status TEXT NOT NULL CHECK(evidence_status IN ('BOUNDED_RUNTIME_CHECK','EXACT_HISTORICAL_BLOBS','JOINT_CHECKS_PASSED'))
);
CREATE TABLE IF NOT EXISTS semantic_dependency (
  prerequisite TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  dependent TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  kind TEXT NOT NULL CHECK(kind IN ('requires')),
  PRIMARY KEY(prerequisite, dependent),
  CHECK(prerequisite != dependent)
);
CREATE TABLE IF NOT EXISTS semantic_witness (
  tx_id TEXT NOT NULL REFERENCES semantic_transaction(tx_id),
  phase TEXT NOT NULL,
  command_digest TEXT NOT NULL,
  input_commit TEXT NOT NULL,
  observed_exit INTEGER NOT NULL CHECK(observed_exit=0),
  PRIMARY KEY(tx_id, phase, command_digest)
);
"#;

#[derive(Clone)]
pub struct Spec {
    pub base: PathBuf,
    pub fix: PathBuf,
    pub output: PathBuf,
    pub name: String,
    pub logical: String,
    pub module_old: String,
    pub module_new: String,
    pub adapter: PathBuf,
    pub manifest: PathBuf,
    pub layout_probe: Vec<String>,
    pub layout_compat: Vec<String>,
    pub fix_checks: Vec<Vec<String>>,
    pub transported_checks: Vec<Vec<String>>,
    pub joint_checks: Vec<Vec<String>>,
}
fn get_cmd(v: &Value) -> Result<Vec<String>> {
    let row = v.array_ref()?;
    if row.is_empty() || row.len() > 16 {
        return refuse("empty or oversized native test command");
    }
    let argv: Vec<String> = row
        .iter()
        .map(|x| Ok(x.str()?.to_owned()))
        .collect::<Result<_>>()?;
    if !matches!(argv[0].as_str(), "python" | "python3" | "node" | "go") {
        return refuse("only reviewed native Python/Node commands supported");
    }
    if argv.iter().any(|x| x.is_empty() || x.contains('\0')) {
        return refuse("invalid native argv");
    }
    if argv[0] == "go" && !matches!(argv.get(1).map(String::as_str), Some("test" | "run")) {
        return refuse("Go native checks restricted to go test/run");
    }
    Ok(argv)
}
fn get_suite(v: &Value, name: &str) -> Result<Vec<Vec<String>>> {
    let vals = v.get(name)?.array_ref()?;
    if vals.is_empty() || vals.len() > 16 {
        return refuse(format!(
            "{name}: at least one and at most 16 checks required"
        ));
    }
    vals.iter().map(get_cmd).collect()
}
impl Spec {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        if !v.get("trusted_local")?.bool()? {
            return refuse("trusted_local must be explicitly true");
        }
        let path_field = |key: &str| -> Result<PathBuf> {
            PathBuf::from(v.get(key)?.str()?)
                .canonicalize()
                .map_err(|e| format!("{key}: {e}"))
        };
        let output = PathBuf::from(v.get("output")?.str()?);
        if !output.is_absolute() {
            return refuse("output must be an absolute directory");
        }
        let name = v.get("name")?.str()?.to_string();
        if name.trim().is_empty() || name.contains('/') {
            return refuse("invalid package root");
        }
        let logical = v.get("logical")?.str()?.to_string();
        let module_old = v.get("module_old")?.str()?.to_string();
        let module_new = v.get("module_new")?.str()?.to_string();
        git::validate_path(&module_old)?;
        git::validate_path(&module_new)?;
        if module_old == module_new {
            return refuse("no structural change");
        }
        if !module_old.starts_with(&format!("{name}/"))
            || !module_new.starts_with(&format!("{name}/"))
        {
            return refuse("moved module must remain in declared package");
        }
        let layout_probe = get_cmd(v.get("layout_probe")?)?;
        let layout_compat = get_cmd(v.get("layout_compat")?)?;
        Ok(Self {
            base: path_field("base")?,
            fix: path_field("fix")?,
            adapter: path_field("adapter")?,
            manifest: path_field("manifest")?,
            output,
            name,
            logical,
            module_old,
            module_new,
            layout_probe,
            layout_compat,
            fix_checks: get_suite(&v, "fix_checks")?,
            transported_checks: get_suite(&v, "transported_checks")?,
            joint_checks: get_suite(&v, "joint_checks")?,
        })
    }
}
fn native_check(root: &Path, argv: &[String]) -> Result<String> {
    if !matches!(
        argv.first().map(String::as_str),
        Some("python" | "python3" | "node" | "go")
    ) {
        return refuse("unsupported native command");
    }
    let out = Command::new("timeout")
        .arg("--kill-after=2s")
        .arg("12s")
        .arg(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .env("PYTHONPATH", root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
        .output()
        .map_err(|e| format!("native check launch failed: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "native check failed at {}: {} {}",
            root.display(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    Ok(obj(&[
        ("command", Value::array(argv.iter().map(Value::string))),
        ("exit", Value::number(0)),
    ])
    .sha256())
}
pub(crate) fn native_suite(root: &Path, suite: &[Vec<String>]) -> Result<Vec<String>> {
    suite.iter().map(|argv| native_check(root, argv)).collect()
}
pub(crate) fn git_with_env(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "RelGit Semantic Transaction")
        .env("GIT_COMMITTER_NAME", "RelGit Semantic Transaction")
        .env("GIT_AUTHOR_EMAIL", "relgit@example.invalid")
        .env("GIT_COMMITTER_EMAIL", "relgit@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-11-09T00:00:00+0000")
        .env("GIT_COMMITTER_DATE", "2026-11-09T00:00:00+0000")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return refuse(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout)
        .map(|x| x.trim().to_owned())
        .map_err(|e| e.to_string())
}
fn tx(kind: &str, logical: &str, input: &str, output: &str) -> String {
    obj(&[
        ("kind", Value::string(kind)),
        ("logical", Value::string(logical)),
        ("input", Value::string(input)),
        ("output", Value::string(output)),
    ])
    .sha256()
}

/// Order ideals for A, B incomparable, both required by integrated result C.
/// Their semantics reflect prerequisites, not chronological Git order.
pub fn dependency_ideals() -> Vec<Vec<&'static str>> {
    vec![
        vec![],
        vec!["structural"],
        vec!["behavioral"],
        vec!["structural", "behavioral"],
        vec!["structural", "behavioral", "integrated"],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poset_has_five_valid_ideals() {
        let ideals = dependency_ideals();
        assert_eq!(ideals.len(), 5);
        assert!(!ideals.contains(&vec!["integrated"]));
        assert!(ideals.contains(&vec!["structural", "behavioral", "integrated"]));
    }
    #[test]
    fn only_explicit_native_runtimes() {
        assert!(
            get_cmd(&Value::array(vec![
                Value::string("sh"),
                Value::string("-c")
            ]))
            .is_err()
        );
        assert!(
            get_cmd(&Value::array(vec![
                Value::string("python"),
                Value::string("-c")
            ]))
            .is_ok()
        );
        assert!(get_cmd(&Value::array(vec![])).is_err());
    }
}
