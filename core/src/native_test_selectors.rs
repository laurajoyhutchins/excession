//! Delegate test selection to established native tools. This module does not
//! implement dependency inference, test ownership, or a replacement selector.
//! Execution requires an explicitly trusted checkout, including native config.

use crate::json::{Value, obj};
use crate::{Result, change_inventory, digest, discovery, git, refuse};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_SELECTION: usize = 4096;
const MAX_LOG_BYTES: usize = 16 * 1024 * 1024;
static REPORT_SERIAL: AtomicUsize = AtomicUsize::new(0);

struct NativeReport {
    directory: PathBuf,
    file: PathBuf,
}

impl NativeReport {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "excession-native-pytest-report-{}-{}",
            std::process::id(),
            REPORT_SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)
            .map_err(|error| format!("create private pytest report directory: {error}"))?;
        Ok(Self {
            file: directory.join("pytest-report.json"),
            directory,
        })
    }
}

impl Drop for NativeReport {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn clean_checkout(root: &Path, commit: &str) -> Result<()> {
    if git::rev(root, "HEAD")? != commit {
        return refuse("native selector requires HEAD to equal the requested candidate commit");
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|error| format!("inspect candidate checkout: {error}"))?;
    if !output.status.success() || !output.stdout.is_empty() {
        return refuse("native selector requires a clean tracked checkout at candidate HEAD");
    }
    Ok(())
}

fn changed_sources(changes: &Value) -> Result<Vec<String>> {
    let mut sources = BTreeSet::new();
    for change in changes.get("changes")?.array_ref()? {
        let before = change.get("before")?;
        let after = change.get("after")?;
        if after == &Value::Null {
            return refuse("native test selection cannot prove coverage for deleted source paths");
        }
        let path = after.get("path")?.str()?;
        if before != &Value::Null && before.get("path")?.str()? != path {
            return refuse("native test selection cannot prove coverage for renamed source paths");
        }
        sources.insert(path.to_owned());
    }
    if sources.is_empty() {
        return refuse("native test selection requires changed tracked source files");
    }
    Ok(sources.into_iter().collect())
}

fn executable(root: &Path, provider: &str, tool: Option<&str>) -> PathBuf {
    match tool {
        Some(path) => {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        }
        None if provider == "jest" => root.join("node_modules/.bin/jest"),
        None => PathBuf::from("python3"),
    }
}

fn tracked_selection(root: &Path, commit: &str, output: &[u8]) -> Result<Vec<String>> {
    let decoded: serde_json::Value = serde_json::from_slice(output)
        .map_err(|error| format!("Jest --listTests did not return a JSON array: {error}"))?;
    let files = decoded
        .as_array()
        .ok_or("Jest --listTests returned a value other than a JSON array")?;
    if files.len() > MAX_SELECTION {
        return refuse("native selector returned more than 4096 tests; refusing truncation");
    }
    let tracked = git::entries(root, commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.path)
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeSet::new();
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize candidate root: {error}"))?;
    for file in files {
        let absolute = file
            .as_str()
            .ok_or("Jest --listTests returned a non-string file path")?;
        let candidate = Path::new(absolute);
        if !candidate.is_absolute() {
            return refuse("Jest selected a non-absolute test path");
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("Jest selected an unresolvable file: {error}"))?;
        let relative = canonical
            .strip_prefix(&canonical_root)
            .map_err(|_| "Jest selected a test outside the candidate repository")?;
        let relative = relative
            .to_str()
            .ok_or("Jest selected a non-UTF-8 Git path")?;
        if !tracked.contains(relative) {
            return refuse(format!(
                "Jest selected a file not tracked by candidate Git: {relative}"
            ));
        }
        selected.insert(relative.to_owned());
    }
    Ok(selected.into_iter().collect())
}

fn reported_pytest_tests(
    root: &Path,
    commit: &str,
    json_bytes: &[u8],
    exit_code: i32,
) -> Result<(Vec<String>, Vec<String>, usize, usize)> {
    let report: serde_json::Value = serde_json::from_slice(json_bytes)
        .map_err(|error| format!("pytest-json-report returned invalid JSON: {error}"))?;
    let reported_exit = report
        .get("exitcode")
        .and_then(serde_json::Value::as_i64)
        .ok_or("pytest-json-report has no exitcode")?;
    if reported_exit != i64::from(exit_code) {
        return refuse("pytest-json-report exitcode disagrees with native process");
    }
    let summary = report
        .get("summary")
        .ok_or("pytest-json-report has no summary")?;
    let total = summary
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .ok_or("pytest-json-report has no executed-test count")? as usize;
    let passed = summary
        .get("passed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let tests = report
        .get("tests")
        .and_then(serde_json::Value::as_array)
        .ok_or("pytest-json-report has no per-test observations")?;
    if total != tests.len() || total > MAX_SELECTION || passed > total {
        return refuse("pytest-json-report test count is inconsistent or too large");
    }
    let tracked = git::entries(root, commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.path)
        .collect::<BTreeSet<_>>();
    let mut files = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    for test in tests {
        let node_id = test
            .get("nodeid")
            .and_then(serde_json::Value::as_str)
            .ok_or("pytest-json-report has a test without nodeid")?;
        if test
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return refuse("pytest-json-report has a test without outcome");
        }
        let path = node_id.split("::").next().unwrap_or("");
        git::validate_path(path)?;
        if !tracked.contains(path) {
            return refuse(format!(
                "pytest-json-report selected test outside candidate Git: {path}"
            ));
        }
        files.insert(path.to_owned());
        node_ids.insert(node_id.to_owned());
    }
    Ok((
        files.into_iter().collect(),
        node_ids.into_iter().collect(),
        total,
        passed,
    ))
}

fn pytest_execution_outcome(code: i32, total: usize, passed: usize) -> &'static str {
    if total == 0 || (code == 0 && passed == 0) {
        "EMPTY_UNPROVEN"
    } else if code == 0 {
        "EXECUTED_PASS"
    } else {
        "EXECUTED_FAIL"
    }
}

fn require_native_execution(trusted_local: &str) -> Result<()> {
    if trusted_local != "true" {
        return refuse("native selectors execute project configuration; pass --trusted-local true");
    }
    Ok(())
}

fn checked_scope(root: &Path, commit: &str, focus: Option<&str>) -> Result<Vec<String>> {
    let tracked = git::entries(root, commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.path)
        .collect::<BTreeSet<_>>();
    let mut paths = BTreeSet::new();
    if let Some(value) = focus {
        for part in value.split(',') {
            if part.is_empty() {
                return refuse("native pytest scope cannot contain an empty path");
            }
            git::validate_path(part)?;
            if Path::new(part)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                return refuse("native pytest scope must contain normalized repository paths");
            }
            if !root.join(part).is_file() || !tracked.contains(part) {
                return refuse(format!("native pytest scope is not a tracked file: {part}"));
            }
            paths.insert(part.to_owned());
        }
    }
    Ok(paths.into_iter().collect())
}

pub fn run(
    repo: &Path,
    base: &str,
    candidate: &str,
    provider: &str,
    tool: Option<&str>,
    focus: Option<&str>,
    trusted_local: &str,
) -> Result<Value> {
    require_native_execution(trusted_local)?;
    if !matches!(provider, "jest" | "pytest-testmon") {
        return refuse(format!("unsupported native test selector {provider:?}"));
    }
    let discovered = discovery::inspect(repo, candidate)?;
    let root = &discovered.root;
    let changes = change_inventory::inventory(root, base, candidate)?;
    let commit = changes.get("head_commit")?.str()?;
    let tree = changes.get("head_tree")?.str()?;
    clean_checkout(root, commit)?;
    let changed = changed_sources(&changes)?;
    let tool_path = executable(root, provider, tool);
    let scope = if provider == "pytest-testmon" {
        checked_scope(root, commit, focus)?
    } else {
        Vec::new()
    };
    let native_report = if provider == "pytest-testmon" {
        Some(NativeReport::new()?)
    } else {
        None
    };
    let mut command = Command::new(&tool_path);
    command.current_dir(root);

    if provider == "jest" {
        if focus.is_some() {
            return refuse("--focus is only supported by pytest-testmon");
        }
        command.args(["--listTests", "--json", "--findRelatedTests"]);
        command.args(&changed);
    } else {
        if !root.join(".testmondata").is_file() {
            return refuse("pytest-testmon requires an existing .testmondata coverage baseline");
        }
        command.args(["-m", "pytest", "--testmon", "--testmon-forceselect", "-q"]);
        let report = native_report
            .as_ref()
            .ok_or("missing native pytest report path")?;
        command.arg("--json-report");
        command.arg(format!("--json-report-file={}", report.file.display()));
        command.args(&scope);
    }

    let output = command.output().map_err(|error| {
        format!("invoke explicit trusted-local native test selector {provider}: {error}")
    })?;
    // A native project tool can mutate tracked source during execution.
    // Never source-bind its observations to the previous commit in that case.
    clean_checkout(root, commit)?;
    if output.stdout.len() + output.stderr.len() > MAX_LOG_BYTES {
        return refuse("native selector output exceeds 16 MiB; refusing partial evidence");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let failure_detail = stderr.chars().take(500).collect::<String>();
    let tool_name = tool_path.to_string_lossy().into_owned();

    let mut selected = Vec::new();
    let mut selected_ids = Vec::new();
    let mut executed_count = 0usize;
    let mut receipts = Vec::new();
    let outcome = if provider == "jest" {
        if !output.status.success() {
            return refuse(format!(
                "Jest related-test selection failed: {failure_detail}"
            ));
        }
        selected = tracked_selection(root, commit, &output.stdout)?;
        if selected.is_empty() {
            "EMPTY_UNPROVEN"
        } else {
            "SELECTED"
        }
    } else {
        let code = output
            .status
            .code()
            .ok_or("pytest-testmon exited without a numeric status")?;
        if !matches!(code, 0 | 1) {
            return refuse(format!(
                "pytest-testmon did not complete normal test execution (exit {code}): {failure_detail}"
            ));
        }
        let native_report = native_report
            .as_ref()
            .ok_or("missing native pytest execution report")?;
        let native_json = fs::read(&native_report.file).map_err(|error| {
            format!("pytest-json-report is required for verifiable test execution: {error}")
        })?;
        let (files, ids, total, passed) = reported_pytest_tests(root, commit, &native_json, code)?;
        selected = files;
        selected_ids = ids;
        executed_count = total;
        // A zero-test run cannot produce a passing execution receipt.
        let selected_outcome = pytest_execution_outcome(code, total, passed);
        if selected_outcome == "EMPTY_UNPROVEN" {
            selected_outcome
        } else {
            let mut log = output.stdout.clone();
            log.extend_from_slice(&output.stderr);
            let execution = obj(&[
                ("kind", Value::string("TEST")),
                (
                    "status",
                    Value::string(if code == 0 { "PASS" } else { "FAIL" }),
                ),
                ("source_commit", Value::string(commit)),
                ("source_tree", Value::string(tree)),
                ("producer", Value::string("pytest-testmon")),
                (
                    "command",
                    Value::string(
                        format!(
                            "{tool_name} -m pytest --testmon --testmon-forceselect -q --json-report {}",
                            scope
                                .iter()
                                .map(|path| format!("{path:?}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                        .trim_end()
                        .to_owned(),
                    ),
                ),
                ("exit_code", Value::number(code as i64)),
                ("log_sha256", Value::string(digest::sha256(&log))),
            ]);
            receipts.push(execution);
            selected_outcome
        }
    };
    let digest_value = obj(&[
        ("schema", Value::string("excession.execution-receipts.v1")),
        ("receipts", Value::array(receipts)),
    ]);
    let mut report = obj(&[
        ("schema", Value::string("excession.native-test-selector.v1")),
        ("provider", Value::string(provider)),
        ("tool", Value::string(tool_name)),
        ("base_commit", changes.get("base_commit")?.clone()),
        ("candidate_commit", Value::string(commit)),
        ("candidate_tree", Value::string(tree)),
        (
            "changed_sources",
            Value::array(changed.into_iter().map(Value::string)),
        ),
        (
            "selected_test_files",
            Value::array(selected.into_iter().map(Value::string)),
        ),
        ("outcome", Value::string(outcome)),
        ("executed_test_count", Value::number(executed_count)),
        (
            "selected_test_ids",
            Value::array(selected_ids.into_iter().map(Value::string)),
        ),
        (
            "scope_files",
            Value::array(scope.into_iter().map(Value::string)),
        ),
        (
            "native_exit_code",
            Value::number(output.status.code().unwrap_or(-1)),
        ),
        (
            "native_stdout_sha256",
            Value::string(digest::sha256(&output.stdout)),
        ),
        (
            "native_stderr_sha256",
            Value::string(digest::sha256(&output.stderr)),
        ),
        ("execution_receipts", digest_value),
        ("receipt_trust", Value::string("LOCAL_COMMAND_UNATTESTED")),
        ("coverage", Value::string("PARTIAL")),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
        (
            "selector_policy",
            Value::string(
                "NATIVE_TOOL_AUTHORITATIVE_FOR_REPORTED_SELECTION; NO_COMPLETENESS_GUARANTEE",
            ),
        ),
    ]);
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(report_id));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CASE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn execution_requires_explicit_trust() {
        assert!(require_native_execution("false").is_err());
        assert!(require_native_execution("yes").is_err());
        assert!(require_native_execution("true").is_ok());
    }

    #[test]
    fn only_tracked_absolute_jest_test_paths_are_accepted() {
        let root = std::env::temp_dir().join(format!(
            "excession-native-tests-{}-{}",
            std::process::id(),
            CASE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("testing")).unwrap();
        fs::write(root.join("src/module.js"), "module.exports = 1;\n").unwrap();
        fs::write(root.join("testing/module.test.js"), "test('ok', ()=>{});\n").unwrap();
        fs::write(root.join("testing/test_example.py"), "def test_ok(): pass\n").unwrap();

        let git_call = |args: &[&str]| {
            let result = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "git {:?}: {}",
                args,
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git_call(&["init", "-q"]);
        git_call(&["add", "."]);
        git_call(&[
            "-c",
            "user.name=Excession Test",
            "-c",
            "user.email=excession@example.invalid",
            "commit",
            "-qm",
            "base",
        ]);
        let commit = git::rev(&root, "HEAD").unwrap();
        let path = root.join("testing/module.test.js");
        let listed = serde_json::to_vec(&vec![path.to_str().unwrap()]).unwrap();
        assert_eq!(
            tracked_selection(&root, &commit, &listed).unwrap(),
            vec!["testing/module.test.js".to_owned()]
        );
        assert!(tracked_selection(&root, &commit, br#"["relative.js"]"#).is_err());
        assert!(tracked_selection(&root, &commit, br#"{"wrong": true}"#).is_err());

        fs::write(root.join("testing/untracked.test.js"), "").unwrap();
        let untracked = serde_json::to_vec(&vec![
            root.join("testing/untracked.test.js").to_str().unwrap(),
        ])
        .unwrap();
        assert!(tracked_selection(&root, &commit, &untracked).is_err());
        let native = br#"{"exitcode":0,"summary":{"total":1,"passed":1},"tests":[{"nodeid":"testing/test_example.py::test_ok","outcome":"passed"}]}"#;
        let (files, ids, total, passed) =
            reported_pytest_tests(&root, &commit, native, 0).unwrap();
        assert_eq!(files, vec!["testing/test_example.py"]);
        assert_eq!(ids, vec!["testing/test_example.py::test_ok"]);
        assert_eq!((total, passed), (1, 1));

        let empty = br#"{"exitcode":0,"summary":{"total":0},"tests":[]}"#;
        let (_, _, total, passed) =
            reported_pytest_tests(&root, &commit, empty, 0).unwrap();
        assert_eq!(pytest_execution_outcome(0, total, passed), "EMPTY_UNPROVEN");
        assert_eq!(pytest_execution_outcome(0, 1, 1), "EXECUTED_PASS");
        assert_eq!(pytest_execution_outcome(1, 1, 0), "EXECUTED_FAIL");

        let inconsistent = br#"{"exitcode":0,"summary":{"total":1},"tests":[]}"#;
        assert!(reported_pytest_tests(&root, &commit, inconsistent, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
