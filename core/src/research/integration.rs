//! Evidence carry across a *pinned*, two-parent h11 merge. Unknown cases refuse.
//! This is not general proof or signed cross-agent attestation.
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use crate::{git, index, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const UPSTREAM: [(&str, &str, &str); 3] = [
    (
        "h11/_headers.py",
        "b97d020b634a9f47f5ae6aa3b30e2bd13a6c48c4",
        "31da3e2b23b55a624b36f105e62a6902e63286aa",
    ),
    (
        "h11/tests/test_headers.py",
        "ba53d088f6f8d22dede47873a03b70a7103da7ea",
        "b57274cbbfee8fce3e89f012b6d0f4206f768f67",
    ),
    (
        "h11/_readers.py",
        "08a9574da4a89d82dfb71b3087b14c8644102dd6",
        "1348565a62c3f3e5fb67497f986cda7e3cd3f680",
    ),
];
// This separately authored reader regression file is NOT an upstream blob.
// Pin it nonetheless: altered test source must never inherit earlier evidence.
const READER_TEST_OID: &str = "e92c6c6c74b38314e6714862559532e228ef5d55";

const ASSUMPTIONS: [&str; 4] = [
    "Finite deterministic execution and same interpreter/toolchain",
    "No untracked runtime influences, unexpected imports or hidden side effects",
    "Class method-body changes are inert until called",
    "Source and observation authenticity depend on pinned Git and local execution, not signed attestations",
];
const PREMISES: [&str; 8] = [
    "Both agent commits share one exact pinned parent",
    "The three historical implementation and regression blobs match GitHub upstream OIDs",
    "Native Git two-parent merge retains exact branch source and test blobs",
    "Reader change is confined to two undecorated class method bodies",
    "Header claim observed no calls to changed reader methods",
    "Reader claim observed exactly four calls with empty headers and _parsed=False",
    "Old and new header implementations return equal results on the witnessed empty input",
    "Both branch-native test groups passed without failures or skips in the pinned environment",
];

fn string_list(xs: &[&str]) -> Value {
    Value::array(xs.iter().map(|x| Value::string(*x)))
}

/// Run the native header interface and environment checks concurrently in
/// isolated interpreter processes. Combining the environment check with the
/// interface check removes one redundant Python interpreter startup.
fn header_probe_and_environment(repo: &Path) -> Result<(String, String, String)> {
    const CODE: &str = "import json,sys,pytest,h11\nfrom pathlib import Path\nfrom h11._headers import normalize_and_validate\nassert Path(h11.__file__).resolve().parents[1] == Path.cwd().resolve()\nprint(json.dumps([[v.hex() for v in triple] for triple in normalize_and_validate([])], sort_keys=True))\nprint(sys.version.split()[0])\nprint(pytest.__version__)";
    let out = crate::adapter::python_command()
        .arg("-c")
        .arg(CODE)
        .current_dir(repo)
        .env("PYTHONPATH", repo)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| format!("Python combined interface/env probe launch: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "Python combined interface/env probe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout =
        String::from_utf8(out.stdout).map_err(|_| "invalid UTF-8 interface/env probe output")?;
    let lines = stdout.lines().collect::<Vec<_>>();
    if lines.len() != 3 || lines[0] != "[]" || lines[1].is_empty() || lines[2].is_empty() {
        return refuse("unexpected Python interface/environment probe output");
    }
    Ok((
        lines[0].to_owned(),
        lines[1].to_owned(),
        lines[2].to_owned(),
    ))
}

#[derive(Clone)]
struct LocalWitness {
    prior_empty: String,
    updated_empty: String,
    python: String,
    pytest: String,
}

/// Execute the pinned native branch tests with constrained CPython code-object
/// monitoring. The results are captured from this child process, never loaded
/// as externally supplied unsigned evidence. This is a LOCAL-ONLY mode.
fn run_local_branch_probe(repo: &Path, claim: &str) -> Result<(Value, String)> {
    const CODE: &str = include_str!("../../adapters/probe_monitor.py");
    let out = crate::adapter::python_command()
        .arg("-c")
        .arg(CODE)
        .arg(claim)
        .arg("--stdout-only")
        .current_dir(repo)
        .env("PYTHONPATH", repo)
        .env("PYTEST_DISABLE_PLUGIN_AUTOLOAD", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| format!("native branch test launch: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "native {claim} tests failed: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let output = String::from_utf8(out.stdout).map_err(|_| "native test output was not UTF-8")?;
    let prefix = "RELGIT_OBSERVATION:";
    let lines = output
        .lines()
        .filter(|s| s.starts_with(prefix))
        .collect::<Vec<_>>();
    if lines.len() != 1 || !output.trim_end().ends_with(lines[0]) {
        return refuse("native test did not produce exactly one terminal observation");
    }
    let value = json::parse(&lines[0].as_bytes()[prefix.len()..])?;
    let empty = value.get("empty_header_result")?.str()?.to_owned();
    if empty != "[]" {
        return refuse("unexpected empty header observation in native branch tests");
    }
    Ok((value, empty))
}

/// Execute two test groups concurrently, then validate both against the exact
/// Git revisions and their observed interface traces in the same Rust call.
/// No user-provided evidence file or previously signed result is trusted.
pub fn carry_local(root: &Path, db_path: Option<&Path>) -> Result<Value> {
    let a = root.join("agent-A-headers");
    let b = root.join("agent-B-reader");
    // This local in-process optimization is intentionally restricted to the
    // exact known testable Git revisions. Do not execute arbitrary agent code
    // with the local runner; new revisions need a reviewed trust policy or
    // a separately isolated executor.
    for (folder, expected) in [
        ("h11-base", "92f17988e2b36b8da77570c63465a20b3ca2d410"),
        (
            "agent-A-headers",
            "4889db56f340433f4601b7729440d84987ab2747",
        ),
        ("agent-B-reader", "332a2a2f4d62401f1b7d73fa875243f7398cf0e6"),
    ] {
        let path = root.join(folder);
        git::clean(&path)?;
        if git::rev(&path, "HEAD")? != expected {
            return refuse(format!("untrusted local test revision: {folder}"));
        }
    }
    let ((headers, updated_empty), (reader, prior_empty)) = std::thread::scope(|scope| {
        let test_a = scope.spawn(|| run_local_branch_probe(&a, "headers"));
        let test_b = scope.spawn(|| run_local_branch_probe(&b, "reader"));
        let obs_a = test_a
            .join()
            .map_err(|_| "native header worker panicked")??;
        let obs_b = test_b
            .join()
            .map_err(|_| "native reader worker panicked")??;
        Ok::<_, String>((obs_a, obs_b))
    })?;
    let python = headers.get("python")?.str()?.to_owned();
    let pytest = headers.get("pytest")?.str()?.to_owned();
    if python != reader.get("python")?.str()? || pytest != reader.get("pytest")?.str()? {
        return refuse("branch tests used different Python environments");
    }
    let evidence = Observations { headers, reader };
    let witness = LocalWitness {
        prior_empty,
        updated_empty,
        python,
        pytest,
    };
    bounded_carry_inner(root, &evidence, db_path, Some(&witness))
}

#[derive(Clone)]
pub struct Observations {
    pub headers: Value,
    pub reader: Value,
}

impl Observations {
    pub fn from_files(root: &Path) -> Result<Self> {
        let load = |p: &str| -> Result<Value> {
            let contents = std::fs::read(root.join(p)).map_err(|e| format!("{p}: {e}"))?;
            json::parse(&contents)
        };
        Ok(Self {
            headers: load("results/agent-A-headers.json")?,
            reader: load("results/agent-B-reader.json")?,
        })
    }
}

fn validate_observation(
    obs: &Value,
    claim: &str,
    repo: &Path,
    python: &str,
    pytest: &str,
) -> Result<(i64, usize)> {
    if obs.get("claim")?.str()? != claim
        || obs.get("exit_code")?.integer()? != 0
        || obs.get("failed")?.integer()? != 0
        || obs.get("skipped")?.integer()? != 0
    {
        return refuse("failed, skipped or mislabeled evidence");
    }
    if obs.get("python")?.str()? != python || obs.get("pytest")?.str()? != pytest {
        return refuse("execution environment mismatch");
    }
    let passed = obs.get("passed")?.integer()?;
    if passed != if claim == "headers" { 3 } else { 13 } {
        return refuse("wrong test-case count");
    }
    let recorded = obs.get("module_origin")?.str()?;
    let actual = std::fs::canonicalize(recorded).map_err(|_| "recorded module origin absent")?;
    let expected = std::fs::canonicalize(repo.join("h11/__init__.py"))
        .map_err(|_| "expected module origin absent")?;
    if actual != expected {
        return refuse("evidence imported from wrong checkout");
    }
    let boundary = obs.get("cross_boundary")?.array_ref()?;
    let mut headers = 0usize;
    let mut readers = 0usize;
    for item in boundary {
        let kind = item.get("kind")?.str()?;
        match kind {
            "header-interface" => {
                headers += 1;
                // Boolean and exact field shape prevent false reuse from malformed evidence.
                let parsed = item.get("_parsed")?.bool()?;
                if claim == "reader" && (item.get("headers")?.str()? != "empty" || parsed) {
                    return refuse("changed header boundary was called with unverified inputs");
                }
                if claim == "reader" && item.entries()?.len() != 3 {
                    return refuse("unexpected header observation fields");
                }
            }
            "reader-method" => {
                readers += 1;
                if claim == "headers" {
                    return refuse("header tests invoked changed reader methods");
                }
                let method = item.get("name")?.str()?;
                if !matches!(method, "ChunkedReader.__init__" | "ChunkedReader.__call__")
                    || item.entries()?.len() != 2
                {
                    return refuse("unexpected reader method trace");
                }
            }
            _ => return refuse("unrecognized cross-boundary event"),
        }
    }
    if claim == "reader" && (headers != 4 || readers == 0) {
        return refuse("missing cross-boundary interface observations");
    }
    Ok((passed, headers))
}

fn trace_digest(obs: &Value) -> Result<String> {
    let mut fields = BTreeMap::new();
    for field in [
        "claim",
        "passed",
        "failed",
        "skipped",
        "exit_code",
        "cross_boundary",
        "python",
        "pytest",
    ] {
        fields.insert(field.to_owned(), obs.get(field)?.clone());
    }
    Ok(Value::Object(fields).sha256())
}

pub fn bounded_carry(
    root: &Path,
    observations: &Observations,
    db_path: Option<&Path>,
) -> Result<Value> {
    bounded_carry_inner(root, observations, db_path, None)
}

fn bounded_carry_inner(
    root: &Path,
    observations: &Observations,
    db_path: Option<&Path>,
    local: Option<&LocalWitness>,
) -> Result<Value> {
    let dirs = [
        ("base", "h11-base"),
        ("A", "agent-A-headers"),
        ("B", "agent-B-reader"),
        ("merged", "integration"),
    ];
    let mut paths = BTreeMap::<String, PathBuf>::new();
    let mut rev = BTreeMap::<String, String>::new();
    for (name, folder) in dirs {
        let path = root.join(folder);
        git::clean(&path)?;
        rev.insert(name.into(), git::rev(&path, "HEAD")?);
        paths.insert(name.into(), path);
    }
    let get = |k: &str| paths.get(k).ok_or(format!("missing path: {k}"));
    let revision = |k: &str| rev.get(k).ok_or(format!("missing revision: {k}"));
    if git::rev(get("A")?, "HEAD^")? != *revision("base")?
        || git::rev(get("B")?, "HEAD^")? != *revision("base")?
    {
        return refuse("agent changes do not share exact parent");
    }
    let parents = git::text(
        get("merged")?,
        &["rev-list", "--parents", "-n", "1", revision("merged")?],
    )?;
    let parts = parents.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != revision("merged")? || parts[2] != revision("B")? {
        return refuse("merge is not a two-parent integration with B as second parent");
    }
    git::command(
        get("merged")?,
        &[
            "merge-base",
            "--is-ancestor",
            revision("A")?,
            revision("merged")?,
        ],
    )?;
    if !git::changed(get("merged")?, revision("A")?, parts[1])?.is_empty() {
        return refuse("unexpected changes between A and integration first parent");
    }
    let a_paths = git::changed(get("A")?, revision("base")?, revision("A")?)?;
    let b_paths = git::changed(get("B")?, revision("base")?, revision("B")?)?;
    if a_paths != ["h11/_headers.py", "h11/tests/test_headers.py"] {
        return refuse("unrecognized agent A changed paths");
    }
    if b_paths != ["h11/_readers.py", "tests/test_reader_contract.py"] {
        return refuse("unrecognized agent B changed paths");
    }
    for (path, pre, post) in UPSTREAM {
        if git::blob(get("base")?, "HEAD", path)? != pre {
            return refuse(format!("upstream before-blob mismatch: {path}"));
        }
        let branch = if path.contains("headers") {
            get("A")?
        } else {
            get("B")?
        };
        if git::blob(branch, "HEAD", path)? != post {
            return refuse(format!("upstream after-blob mismatch: {path}"));
        }
        if git::blob(get("merged")?, "HEAD", path)? != post {
            return refuse(format!("merge lost implementation change: {path}"));
        }
    }
    for (path, src) in [
        ("h11/tests/test_headers.py", "A"),
        ("tests/test_reader_contract.py", "B"),
    ] {
        if git::blob(get("merged")?, "HEAD", path)? != git::blob(get(src)?, "HEAD", path)? {
            return refuse("merge altered branch test source");
        }
    }
    if git::blob(get("B")?, "HEAD", "tests/test_reader_contract.py")? != READER_TEST_OID {
        return refuse("reader contract regression source differs from pinned fixture");
    }
    // Fixed exact upstream OIDs in the specification above serve as the
    // stronger pinned-source guard for this particular pair: no inferred AST
    // equivalence is claimed for arbitrary Python edits.
    let (old_empty, new_empty, python_version, pytest_version) = if let Some(w) = local {
        // The environment and empty-header observations were obtained while
        // running the native branch tests under this Rust process.
        (
            w.prior_empty.clone(),
            w.updated_empty.clone(),
            w.python.clone(),
            w.pytest.clone(),
        )
    } else {
        // Detached unsigned evidence must still be independently probed.
        let (prior, updated) = std::thread::scope(|scope| {
            let old = scope.spawn(|| header_probe_and_environment(get("B")?));
            let new = scope.spawn(|| header_probe_and_environment(get("A")?));
            let old = old.join().map_err(|_| "prior Python probe panicked")??;
            let new = new.join().map_err(|_| "updated Python probe panicked")??;
            Ok::<_, String>((old, new))
        })?;
        if prior.1 != updated.1 || prior.2 != updated.2 {
            return refuse("inconsistent independent Python environments");
        }
        (prior.0, updated.0, prior.1, prior.2)
    };
    let python = python_version.as_str();
    let pytest = pytest_version.as_str();
    let (a_count, a_headers) =
        validate_observation(&observations.headers, "headers", get("A")?, python, pytest)?;
    let (b_count, b_headers) =
        validate_observation(&observations.reader, "reader", get("B")?, python, pytest)?;
    if a_headers == 0 {
        return refuse("header test call trace missing");
    }
    if old_empty != new_empty {
        return refuse("changed header boundary result");
    }
    let mut source_oids = BTreeMap::new();
    for (path, _, _) in UPSTREAM {
        source_oids.insert(
            path.to_owned(),
            Value::string(git::blob(get("merged")?, "HEAD", path)?),
        );
    }
    let src_oids = Value::Object(source_oids);
    let revisions = Value::object(
        rev.iter()
            .map(|(k, v)| (k.clone(), Value::string(v.clone()))),
    );
    let guard = obj(&[
        ("revision", revisions.clone()),
        ("source_oids", src_oids),
        ("premises", string_list(&PREMISES)),
        (
            "trace_a",
            Value::string(trace_digest(&observations.headers)?),
        ),
        (
            "trace_b",
            Value::string(trace_digest(&observations.reader)?),
        ),
        ("old_empty", Value::string(old_empty)),
        ("new_empty", Value::string(new_empty)),
    ])
    .sha256();
    let merge_tree = git::rev(get("merged")?, "HEAD^{tree}")?;
    let report = obj(&[
        ("outcome", Value::string("BOUNDED_CARRY_ELIGIBLE")),
        ("revision", revisions.clone()),
        ("merge_tree", Value::string(merge_tree)),
        ("guard_sha256", Value::string(guard.clone())),
        ("carried_tests", Value::number(a_count + b_count)),
        ("new_joint_tests_required", Value::number(2)),
        (
            "ordinary_git_merge",
            Value::string("clean_no_relgit_needed"),
        ),
        ("premises", string_list(&PREMISES)),
        ("assumptions", string_list(&ASSUMPTIONS)),
        ("python", Value::string(python)),
        ("pytest", Value::string(pytest)),
        (
            "agent_observations",
            obj(&[
                (
                    "headers",
                    obj(&[
                        ("cases", Value::number(a_count)),
                        ("cross_reader_calls", Value::number(0)),
                    ]),
                ),
                (
                    "reader",
                    obj(&[
                        ("cases", Value::number(b_count)),
                        ("cross_header_calls", Value::number(b_headers)),
                        ("cross_header_inputs", Value::string("[]")),
                    ]),
                ),
            ]),
        ),
        (
            "scope",
            Value::string(
                "Only the executed independent regression domains, conditional on trace and source-shape premises; no unrestricted semantic proof",
            ),
        ),
        (
            "caution",
            Value::string(
                "Original tests must be audited independently; these bounded observations are not signed external evidence",
            ),
        ),
    ]);
    if let Some(location) = db_path {
        let db = Db::open(location)?;
        db.init_schema()?;
        let mut ids = BTreeMap::new();
        for (name, folder) in [
            ("base", "h11-base"),
            ("A", "agent-A-headers"),
            ("B", "agent-B-reader"),
            ("merged", "integration"),
        ] {
            let id = index::index_revision_for_evidence(
                &db,
                &root.join(folder),
                name,
                revision(name)?,
                "two-independent-upstream-h11-changes",
            )?;
            ids.insert(name.to_owned(), id);
        }
        db.transaction(|db| {
            for (claim, branch, suite, observation, paths) in [
                (
                    "header.limit",
                    "A",
                    "h11/tests/test_headers.py",
                    &observations.headers,
                    ["h11/_headers.py", "h11/tests/test_headers.py"],
                ),
                (
                    "reader.footer",
                    "B",
                    "tests/test_reader_contract.py",
                    &observations.reader,
                    ["h11/_readers.py", "tests/test_reader_contract.py"],
                ),
            ] {
                let id = ids[branch];
                let count = observation.get("passed")?.integer()?;
                db.execute(
                    "INSERT INTO independent_claim VALUES(?,?,?,?,?,?)",
                    &[
                        Param::Text(claim),
                        Param::Integer(id),
                        Param::Text(revision(branch)?),
                        Param::Text(suite),
                        Param::Integer(count),
                        Param::Text(&observation.sha256()),
                    ],
                )?;
                for (index, path) in paths.iter().enumerate() {
                    let origin = git::blob(get(branch)?, "HEAD", path)?;
                    db.execute(
                        "INSERT INTO claim_input VALUES(?,?,?,?)",
                        &[
                            Param::Text(claim),
                            Param::Text(path),
                            Param::Text(&origin),
                            Param::Text(if index == 0 {
                                "defining_source"
                            } else {
                                "test_source"
                            }),
                        ],
                    )?;
                }
            }
            db.execute(
                "INSERT INTO integration_result VALUES(?,?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text("two-real-upstream-fixes"),
                    Param::Integer(ids["merged"]),
                    Param::Text(revision("merged")?),
                    Param::Text("git_native_merge"),
                    Param::Text("header.limit"),
                    Param::Text("reader.footer"),
                    Param::Text("BOUNDED_CARRY_ELIGIBLE"),
                    Param::Text("BOUNDED_CARRY_ELIGIBLE"),
                    Param::Text(&guard),
                    Param::Text(&string_list(&ASSUMPTIONS).canonical()),
                ],
            )?;
            for (i, premise) in PREMISES.iter().enumerate() {
                db.execute(
                    "INSERT INTO premise VALUES(?,?,?,?)",
                    &[
                        Param::Text("two-real-upstream-fixes"),
                        Param::Integer(i as i64),
                        Param::Text(premise),
                        Param::Integer(1),
                    ],
                )?;
            }
            db.foreign_keys_valid()?;
            Ok(())
        })?;
    }
    Ok(report)
}

pub fn compare_reference(report: &Value, reference: &Value) -> Result<()> {
    for key in [
        "revision",
        "merge_tree",
        "guard_sha256",
        "outcome",
        "carried_tests",
        "new_joint_tests_required",
        "ordinary_git_merge",
    ] {
        if report.get(key)? != reference.get(key)? {
            return refuse(format!("Rust/Python differential mismatch: {key}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_source_hashes_are_valid() {
        for (_, a, b) in UPSTREAM {
            git::oid(a).expect("pre");
            git::oid(b).expect("post");
        }
    }
    #[test]
    fn premise_count_is_stable() {
        assert_eq!(PREMISES.len(), 8);
        assert_eq!(ASSUMPTIONS.len(), 4);
    }
}
