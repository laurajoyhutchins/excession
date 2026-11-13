//! A usable test workflow built from native providers, pinned Git source, and
//! optional Excession verification. Planning never launches project code.
//! Running is explicit; no homegrown dependency graph is introduced.

use crate::json::{Value, obj};
use crate::{
    Result, agent, change_inventory, digest, discovery, git, native_test_selectors, refuse,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_NATIVE_OUTPUT: usize = 16 * 1024 * 1024;
static NEXT_REPORT: AtomicUsize = AtomicUsize::new(0);

struct Temporary {
    directory: PathBuf,
}
impl Temporary {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "excession-test-workflow-{}-{}",
            std::process::id(),
            NEXT_REPORT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)
            .map_err(|error| format!("create test evidence directory: {error}"))?;
        Ok(Self { directory: path })
    }
    fn file(&self, filename: &str) -> PathBuf {
        self.directory.join(filename)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn default_base(root: &Path, candidate: &str, requested: Option<&str>) -> Result<(String, String)> {
    if let Some(spec) = requested {
        let base = git::rev(root, spec)?;
        if base == candidate {
            return refuse(
                "test base and candidate are the same commit; provide an earlier --base",
            );
        }
        return Ok((base, format!("explicit:{spec}")));
    }
    for spec in ["origin/main", "origin/master", "main", "master"] {
        let Ok(upstream) = git::rev(root, spec) else {
            continue;
        };
        let Ok(base) = git::text(root, &["merge-base", candidate, &upstream]) else {
            continue;
        };
        if base != candidate {
            return Ok((git::oid(&base)?, format!("merge-base:{spec}")));
        }
    }
    let previous = git::rev(root, &format!("{candidate}^"))
        .map_err(|_| "cannot choose a test base for the first commit; supply --base".to_owned())?;
    Ok((previous, "first-parent".to_owned()))
}

fn has_jest_config(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename.starts_with("jest.config.")
}

fn declares_jest(bytes: &[u8]) -> bool {
    let Ok(package) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    for key in ["dependencies", "devDependencies"] {
        if package
            .get(key)
            .and_then(serde_json::Value::as_object)
            .is_some_and(|dependencies| dependencies.contains_key("jest"))
        {
            return true;
        }
    }
    package
        .get("scripts")
        .and_then(|scripts| scripts.get("test"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|script| script.split_whitespace().any(|token| token == "jest"))
}

fn has_python_tests(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    path.ends_with(".py")
        && (filename.starts_with("test_")
            || path.starts_with("tests/")
            || path.starts_with("testing/")
            || path.contains("/tests/"))
}

fn detect_provider(
    root: &Path,
    commit: &str,
    requested: Option<&str>,
) -> Result<(String, Vec<String>)> {
    if let Some(name) = requested {
        if !matches!(name, "auto" | "jest" | "pytest-testmon") {
            return refuse(format!(
                "unsupported test provider {name:?}; choose jest, pytest-testmon, or auto"
            ));
        }
        if name != "auto" {
            return Ok((name.to_owned(), vec![name.to_owned()]));
        }
    }

    let paths = git::entries(root, commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.path)
        .collect::<BTreeSet<_>>();
    let package_declares_jest = if paths.contains("package.json") {
        let blob_ref = format!("{commit}:package.json");
        git::command(root, &["show", &blob_ref])
            .ok()
            .filter(|bytes| bytes.len() <= 1024 * 1024)
            .is_some_and(|bytes| declares_jest(&bytes))
    } else {
        false
    };
    let jest = root.join("node_modules/.bin/jest").exists()
        || paths.iter().any(|path| has_jest_config(path))
        || package_declares_jest;
    let python = root.join(".testmondata").is_file()
        || paths.contains("pytest.ini")
        || paths.iter().any(|path| has_python_tests(path));
    let mut detected = Vec::new();
    if jest {
        detected.push("jest".to_owned());
    }
    if python {
        detected.push("pytest-testmon".to_owned());
    }
    Ok((
        if detected.len() == 1 {
            detected[0].clone()
        } else {
            "UNRESOLVED".to_owned()
        },
        detected,
    ))
}

fn checkout_is_clean(root: &Path, commit: &str) -> Result<bool> {
    if git::rev(root, "HEAD")? != commit {
        return Ok(false);
    }
    Ok(git::command(root, &["status", "--porcelain", "--untracked-files=no"])?.is_empty())
}

fn test_command(provider: &str, blockers: &BTreeSet<String>) -> &'static str {
    if blockers.contains("CANDIDATE_NOT_CLEAN_HEAD") {
        return "Commit tracked edits, then run excession tests --run";
    }
    if blockers.contains("MULTIPLE_NATIVE_PROVIDERS") {
        return "excession tests --provider jest --run  # or --provider pytest-testmon";
    }
    if blockers.contains("NO_NATIVE_PROVIDER_DETECTED") {
        return "Install Jest or pytest-testmon; then run excession tests";
    }
    if blockers.contains("JEST_NOT_INSTALLED") {
        return "npm install && excession tests --run";
    }
    if blockers.contains("TESTMON_BASELINE_MISSING") {
        return "python3 -m pytest --testmon --testmon-noselect && excession tests --run";
    }
    if blockers.contains("NO_CHANGED_SOURCES") {
        return "Commit a candidate change, or supply --base to compare older revisions";
    }
    if blockers.contains("DELETED_SOURCE_NOT_SUPPORTED")
        || blockers.contains("RENAMED_SOURCE_NOT_SUPPORTED")
    {
        return "Use the full native test suite for deletions or renames";
    }
    if provider == "UNRESOLVED" {
        return "excession tests --provider jest  # or --provider pytest-testmon";
    }
    "excession tests --run"
}

pub struct Request<'a> {
    pub repo: &'a Path,
    pub base: Option<&'a str>,
    pub candidate: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub tool: Option<&'a str>,
    pub focus: Option<&'a str>,
    pub run: bool,
    pub verify: bool,
}

/// Return a stable JSON decision for programmers and coding agents.
/// 'ready to run' is never equivalent to complete test coverage.
pub fn execute(request: Request<'_>) -> Result<Value> {
    if request.verify && !request.run {
        return refuse("--verify requires --run; planning never executes native tools");
    }
    let candidate_spec = request.candidate.unwrap_or("HEAD");
    let discovered = discovery::inspect(request.repo, candidate_spec)?;
    let root = discovered.root;
    let commit = discovered.commit;
    let tree = discovered.tree;
    let (base, base_strategy) = default_base(&root, &commit, request.base)?;
    let changes = change_inventory::inventory(&root, &base, &commit)?;
    let (provider, detected) = detect_provider(&root, &commit, request.provider)?;
    let mut blockers = BTreeSet::<String>::new();
    if provider == "UNRESOLVED" {
        blockers.insert(
            if detected.is_empty() {
                "NO_NATIVE_PROVIDER_DETECTED"
            } else {
                "MULTIPLE_NATIVE_PROVIDERS"
            }
            .to_owned(),
        );
    }
    let changed = match native_changed_paths(&changes) {
        Ok(paths) => paths,
        Err(reason) => {
            blockers.insert(reason);
            Vec::new()
        }
    };
    if !checkout_is_clean(&root, &commit)? {
        blockers.insert("CANDIDATE_NOT_CLEAN_HEAD".to_owned());
    }
    if provider == "jest" && request.focus.is_some() {
        blockers.insert("JEST_DOES_NOT_ACCEPT_FOCUS".to_owned());
    }
    if provider == "jest" {
        let path = request
            .tool
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("node_modules/.bin/jest"));
        if !path.is_file() {
            blockers.insert("JEST_NOT_INSTALLED".to_owned());
        }
    }
    if provider == "pytest-testmon" && !root.join(".testmondata").is_file() {
        blockers.insert("TESTMON_BASELINE_MISSING".to_owned());
    }
    let phase = if blockers.is_empty() {
        "READY"
    } else {
        "BLOCKED"
    };
    if request.run && !blockers.is_empty() {
        return refuse(format!(
            "native test execution blocked: {}; next: {}",
            blockers.iter().cloned().collect::<Vec<_>>().join(", "),
            test_command(&provider, &blockers)
        ));
    }

    let mut selection = Value::Null;
    let mut execution = Value::Null;
    let mut verification = Value::Null;
    let mut status = phase.to_owned();
    let mut selected = Vec::<String>::new();
    let mut executed = 0usize;
    let mut failed = 0usize;
    if request.run {
        let chosen = native_test_selectors::run(
            &root,
            &base,
            &commit,
            &provider,
            request.tool,
            request.focus,
            "true",
        )?;
        selected = chosen
            .get("selected_test_files")?
            .array_ref()?
            .iter()
            .map(|v| v.str().map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        let mut receipts = chosen.get("execution_receipts")?.clone();
        let outcome = chosen.get("outcome")?.str()?;
        if provider == "jest" && outcome != "EMPTY_UNPROVEN" {
            let jest = execute_jest(&root, &commit, &tree, request.tool, &selected)?;
            status = jest.get("status")?.str()?.to_owned();
            executed = jest.get("executed_test_count")?.integer()? as usize;
            failed = jest.get("failed_test_count")?.integer()? as usize;
            receipts = jest.get("execution_receipts")?.clone();
            execution = jest;
        } else if provider == "pytest-testmon" {
            executed = chosen.get("executed_test_count")?.integer()? as usize;
            status = match outcome {
                "EXECUTED_PASS" => "PASS",
                "EXECUTED_FAIL" => "FAIL",
                _ => "INCONCLUSIVE",
            }
            .to_owned();
            if status == "FAIL" {
                // The native plugin did not promise a specific failed-test count.
                failed = 1;
            }
        } else {
            status = "INCONCLUSIVE".to_owned();
        }
        if request.verify {
            let temporary = Temporary::new()?;
            let receipt_file = temporary.file("receipts.json");
            fs::write(&receipt_file, receipts.canonical()).map_err(|error| {
                format!("write isolated native receipt for verification: {error}")
            })?;
            verification =
                agent::verify(&root, &base, &commit, 32, None, None, Some(&receipt_file))?;
        }
        selection = chosen;
    }

    let mut report = obj(&[
        ("schema", Value::string("excession.test-workflow.v1")),
        (
            "mode",
            Value::string(if request.run { "RUN" } else { "PLAN" }),
        ),
        ("status", Value::string(status)),
        (
            "provider",
            if provider == "UNRESOLVED" {
                Value::Null
            } else {
                Value::string(provider.clone())
            },
        ),
        (
            "detected_providers",
            Value::array(detected.into_iter().map(Value::string)),
        ),
        ("repo", Value::string(root.to_string_lossy())),
        ("base_commit", Value::string(base)),
        ("base_strategy", Value::string(base_strategy)),
        ("candidate_commit", Value::string(commit)),
        ("candidate_tree", Value::string(tree)),
        (
            "changed_sources",
            Value::array(changed.into_iter().map(Value::string)),
        ),
        (
            "blockers",
            Value::array(blockers.into_iter().map(Value::string)),
        ),
        ("next_command", Value::string(test_command(&provider, &blockers))),
        (
            "selected_test_files",
            Value::array(selected.into_iter().map(Value::string)),
        ),
        ("executed_test_count", Value::number(executed)),
        ("failed_test_count_lower_bound", Value::number(failed)),
        ("selection", selection),
        ("native_execution", execution),
        ("verification", verification),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]);
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(id));
    Ok(report)
}

fn native_changed_paths(changes: &Value) -> Result<Vec<String>> {
    let mut set = BTreeSet::new();
    for row in changes.get("changes")?.array_ref()? {
        let before = row.get("before")?;
        let after = row.get("after")?;
        if after == &Value::Null {
            return refuse("DELETED_SOURCE_NOT_SUPPORTED");
        }
        let path = after.get("path")?.str()?;
        if before != &Value::Null && before.get("path")?.str()? != path {
            return refuse("RENAMED_SOURCE_NOT_SUPPORTED");
        }
        set.insert(path.to_owned());
    }
    if set.is_empty() {
        return refuse("NO_CHANGED_SOURCES");
    }
    Ok(set.into_iter().collect())
}

fn execute_jest(
    root: &Path,
    commit: &str,
    tree: &str,
    tool: Option<&str>,
    selected: &[String],
) -> Result<Value> {
    if selected.is_empty() {
        return refuse("cannot execute Jest without selected tests");
    }
    let executable = match tool {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("node_modules/.bin/jest"),
    };
    let temp = Temporary::new()?;
    let report_file = temp.file("jest.json");
    let mut command = Command::new(&executable);
    command.current_dir(root);
    command.args(["--runInBand", "--runTestsByPath", "--json"]);
    command.arg(format!("--outputFile={}", report_file.display()));
    command.args(selected);
    let result = command
        .output()
        .map_err(|error| format!("run selected Jest tests: {error}"))?;
    if !checkout_is_clean(root, commit)? {
        return refuse("Jest changed the candidate checkout during test execution");
    }
    if result.stdout.len() + result.stderr.len() > MAX_NATIVE_OUTPUT {
        return refuse("Jest test output exceeds 16 MiB; refusing partial evidence");
    }
    let raw = fs::read(&report_file)
        .map_err(|error| format!("Jest did not produce its JSON execution report: {error}"))?;
    if raw.len() > MAX_NATIVE_OUTPUT {
        return refuse("Jest JSON report exceeds 16 MiB");
    }
    let result_json: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid Jest JSON result: {error}"))?;
    let count = |name| -> Result<usize> {
        let n = result_json
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("Jest JSON missing {name}"))?;
        usize::try_from(n).map_err(|_| format!("Jest JSON {name} exceeds addressable size"))
    };
    let total = count("numTotalTests")?;
    let passed = count("numPassedTests")?;
    let failed = count("numFailedTests")?;
    let pending = count("numPendingTests")?;
    if total == 0 || total > 100000 || passed + failed + pending > total {
        return refuse("Jest reported no tests or inconsistent execution counts");
    }
    let code = result
        .status
        .code()
        .ok_or("Jest exited without a status code")?;
    let status = match (code, failed) {
        (0, 0) if passed > 0 => "PASS",
        (1, n) if n > 0 => "FAIL",
        _ => "INCONCLUSIVE",
    };
    let mut log = result.stdout.clone();
    log.extend_from_slice(&result.stderr);
    let mut receipts = Vec::new();
    if status != "INCONCLUSIVE" {
        receipts.push(obj(&[
            ("kind", Value::string("TEST")),
            ("status", Value::string(status)),
            ("source_commit", Value::string(commit)),
            ("source_tree", Value::string(tree)),
            ("producer", Value::string("jest")),
            (
                "command",
                Value::string(format!(
                    "jest --runTestsByPath [source-bound {count} test files]",
                    count = selected.len()
                )),
            ),
            ("exit_code", Value::number(code)),
            ("log_sha256", Value::string(digest::sha256(&log))),
        ]));
    }
    Ok(obj(&[
        ("provider", Value::string("jest")),
        ("status", Value::string(status)),
        ("executed_test_count", Value::number(total)),
        ("passed_test_count", Value::number(passed)),
        ("failed_test_count", Value::number(failed)),
        ("pending_test_count", Value::number(pending)),
        ("native_exit_code", Value::number(code)),
        (
            "native_stdout_sha256",
            Value::string(digest::sha256(&result.stdout)),
        ),
        (
            "native_stderr_sha256",
            Value::string(digest::sha256(&result.stderr)),
        ),
        ("native_json_sha256", Value::string(digest::sha256(&raw))),
        (
            "execution_receipts",
            obj(&[
                ("schema", Value::string("excession.execution-receipts.v1")),
                ("receipts", Value::array(receipts)),
            ]),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_selection_refuses_unsupported_explicit_names() {
        // Explicit provider validation happens before inspecting the checkout.
        assert!(detect_provider(Path::new("."), "x", Some("custom")).is_err());
        assert_eq!(
            detect_provider(Path::new("."), "x", Some("jest"))
                .unwrap()
                .0,
            "jest"
        );
    }
    #[test]
    fn framework_hints_do_not_mark_every_python_file_as_a_test() {
        assert!(has_python_tests("tests/test_main.py"));
        assert!(has_python_tests("src/module/tests/foo.py"));
        assert!(!has_python_tests("src/package/module.py"));
        assert!(has_jest_config("packages/web/jest.config.ts"));
        assert!(declares_jest(br#"{"devDependencies":{"jest":"30.2.0"}}"#));
        assert!(!declares_jest(br#"{"devDependencies":{"vitest":"3"}}"#));
    }
}
