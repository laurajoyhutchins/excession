//! Bounded trusted-local oracle replay for structurally reconstructed Git histories.
//! This is a source-bound execution observation, not a proof of semantic equivalence.
use crate::evidence::identity::{SourceRef, Subject};
use crate::evidence_store::{self, Batch, Observation, Receipt};
use crate::json::{self, Value, obj};
use crate::sqlite::Db;
use crate::{Result, digest, git, git_reconstruction, git_reconstruction_evidence, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_MANIFEST: u64 = 1024 * 1024;
const MAX_CHECKS: usize = 256;
const MAX_ARGS: usize = 24;
const MAX_ARG_BYTES: usize = 2048;
const MAX_ORACLE_SOURCE: u64 = 1024 * 1024;
const MAX_PROGRAM: u64 = 32 * 1024 * 1024;
const MAX_OUTPUT: u64 = 64 * 1024;
const MAX_TIMEOUT_MS: u64 = 15_000;
const MAX_SOURCE_OBSERVATIONS: usize = 1024;
const PRODUCER: &str = "excession:git-reconstruction-oracle";

fn exact_fields(item: &Value, fields: &[&str]) -> Result<()> {
    let have = item.entries()?;
    if have.len() != fields.len() || fields.iter().any(|key| !have.contains_key(*key)) {
        return refuse("oracle manifest has missing or unexpected fields");
    }
    Ok(())
}

fn bounded_json(path: &Path) -> Result<Value> {
    let meta = fs::metadata(path).map_err(|e| format!("oracle manifest metadata: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST {
        return refuse("oracle manifest must be a regular file <= 1 MiB");
    }
    let data = fs::read(path).map_err(|e| format!("oracle manifest read: {e}"))?;
    if data.len() as u64 > MAX_MANIFEST {
        return refuse("oracle manifest changed while reading");
    }
    json::parse(&data)
}

fn sha256(value: &str) -> Result<&str> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return refuse("oracle digest must be lowercase SHA-256");
    }
    Ok(value)
}

fn absolute(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return refuse("oracle executable, source, and checkout paths must be absolute");
    }
    fs::canonicalize(path).map_err(|e| format!("oracle path resolution: {e}"))
}

fn pinned_file(path: &Path, expected: &str, max_size: u64) -> Result<()> {
    sha256(expected)?;
    let meta = fs::metadata(path).map_err(|e| format!("oracle file metadata: {e}"))?;
    if !meta.is_file() || meta.len() > max_size {
        return refuse("oracle file is missing, oversized or not regular");
    }
    let mut file = fs::File::open(path).map_err(|e| format!("oracle file open: {e}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("oracle file read: {e}"))?;
    if bytes.len() as u64 > max_size || digest::sha256(&bytes) != expected {
        return refuse("oracle executable/source digest mismatch");
    }
    Ok(())
}

fn owned_strings(item: &Value) -> Result<Vec<String>> {
    item.array_ref()?
        .iter()
        .map(|v| Ok(v.str()?.to_owned()))
        .collect()
}

fn set(item: &Value) -> Result<BTreeSet<String>> {
    let strings = owned_strings(item)?;
    if strings.is_empty() || strings.len() > 256 {
        return refuse("oracle check needs 1..=256 declared contracts");
    }
    let unique = strings.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != strings.len() {
        return refuse("duplicate contract in oracle check");
    }
    Ok(unique)
}

struct Execution {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn execute(program: &Path, args: &[String], checkout: &Path, timeout: u64) -> Result<Execution> {
    // This uses no shell, but is NOT a sandbox. Opt-in is required at the API boundary.
    let mut child = Command::new(program)
        .args(args)
        .current_dir(checkout)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("trusted oracle launch: {e}"))?;
    let stdout = child.stdout.take().ok_or("missing oracle stdout")?;
    let stderr = child.stderr.take().ok_or("missing oracle stderr")?;
    let stdout_reader = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let stderr_reader = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        stderr.take(MAX_OUTPUT + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|e| format!("oracle wait: {e}"))? {
            Some(status) => break status,
            None if start.elapsed() >= Duration::from_millis(timeout) => {
                timed_out = true;
                let _ = child.kill();
                break child.wait().map_err(|e| format!("oracle kill/wait: {e}"))?;
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "oracle stdout reader panicked")?
        .map_err(|e| format!("oracle stdout read: {e}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "oracle stderr reader panicked")?
        .map_err(|e| format!("oracle stderr read: {e}"))?;
    if timed_out {
        return refuse("oracle timed out in trusted-local unsandboxed execution");
    }
    if stdout.len() as u64 > MAX_OUTPUT || stderr.len() as u64 > MAX_OUTPUT {
        return refuse("oracle output exceeded 64 KiB limit");
    }
    if !status.success() {
        return refuse(format!(
            "independent oracle failed with exit status {status}"
        ));
    }
    Ok(Execution { stdout, stderr })
}

/// Re-execute pinned external oracles against exact, clean Git checkpoints.
/// Mandatory structural/build/REA replays precede any execution. This appends
/// bounded observations to Excession's existing evidence store; it does not
/// establish universal behavioral correctness or execution isolation.
pub fn replay(
    db: &Db,
    repo: &Path,
    plan_path: &Path,
    evidence_path: &Path,
    manifest_path: &Path,
    trusted_local: bool,
) -> Result<Value> {
    if !trusted_local {
        return refuse("oracle execution requires --trusted-local true; no sandbox is provided");
    }
    let repo_canonical = fs::canonicalize(repo).map_err(|e| format!("oracle repo path: {e}"))?;
    let structural = git_reconstruction::verify(repo, plan_path)?;
    let evidence = git_reconstruction_evidence::replay(db, repo, plan_path, evidence_path)?;
    if evidence.get("structural_report_id")? != structural.get("report_id")? {
        return refuse("structural evidence report no longer matches plan");
    }
    if evidence.get("missing_binding_count")?.integer()? != 0 {
        return refuse("all contract checkpoints require replayed build and REA bindings");
    }
    let manifest = bounded_json(manifest_path)?;
    exact_fields(&manifest, &["schema", "checks"])?;
    if manifest.get("schema")?.str()? != "excession.git-reconstruction-oracles.v1" {
        return refuse("unsupported reconstruction oracle manifest");
    }
    let checks = manifest.get("checks")?.array_ref()?;
    if checks.is_empty() || checks.len() > MAX_CHECKS {
        return refuse("oracle manifest must contain 1..=256 checks");
    }
    let mut coverage = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for history in structural.get("histories")?.array_ref()? {
        for checkpoint in history.get("checkpoints")?.array_ref()? {
            let contracts = checkpoint.get("introduced_contracts")?.array_ref()?;
            for contract in contracts {
                coverage
                    .entry((
                        "INTRODUCTION".to_owned(),
                        checkpoint.get("commit")?.str()?.to_owned(),
                    ))
                    .or_default()
                    .insert(contract.str()?.to_owned());
            }
        }
    }
    for integration in structural.get("integration_nodes")?.array_ref()? {
        coverage.insert(
            (
                "INTEGRATION".to_owned(),
                integration.get("commit")?.str()?.to_owned(),
            ),
            set(integration.get("retained_contracts")?)?,
        );
    }
    let replayed = evidence.get("bindings")?.array_ref()?;
    let allowed = replayed
        .iter()
        .map(|binding| {
            Ok((
                binding.get("phase")?.str()?.to_owned(),
                binding.get("checkpoint")?.str()?.to_owned(),
                binding.get("contract")?.str()?.to_owned(),
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    for ((phase, commit), contracts) in &coverage {
        for contract in contracts {
            if !allowed.contains(&(phase.clone(), commit.clone(), contract.clone())) {
                return refuse("an oracle requirement lacks its authenticated build/REA binding");
            }
        }
    }
    if coverage.len() != checks.len() {
        return refuse(
            "oracle checks must cover every introduced checkpoint and integration exactly once",
        );
    }
    let plan = bounded_json(plan_path)?;
    let sources = plan
        .get("contracts")?
        .array_ref()?
        .iter()
        .map(|c| {
            Ok((
                c.get("id")?.str()?.to_owned(),
                set(c.get("source_closure")?)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut observed = BTreeSet::new();
    let mut prepared = Vec::<(String, Receipt, Vec<Observation>, Value)>::new();
    let mut source_observation_count = 0;
    for check in checks {
        exact_fields(
            check,
            &[
                "phase",
                "checkpoint",
                "contracts",
                "checkout",
                "program",
                "program_sha256",
                "oracle_source",
                "oracle_sha256",
                "args",
                "timeout_ms",
                "expected_stdout_sha256",
            ],
        )?;
        let phase = check.get("phase")?.str()?.to_owned();
        let commit = check.get("checkpoint")?.str()?.to_owned();
        let key = (phase.clone(), commit.clone());
        let declared = set(check.get("contracts")?)?;
        if coverage.get(&key) != Some(&declared) || !observed.insert(key) {
            return refuse("oracle phase/checkpoint/contracts mismatch or duplicate execution");
        }
        git::oid(&commit)?;
        let checkout = absolute(check.get("checkout")?.str()?)?;
        let program = absolute(check.get("program")?.str()?)?;
        let oracle = absolute(check.get("oracle_source")?.str()?)?;
        let oracle_str = check.get("oracle_source")?.str()?;
        if oracle.starts_with(&checkout)
            || oracle.starts_with(&repo_canonical)
            || program.starts_with(&checkout)
            || program.starts_with(&repo_canonical)
        {
            return refuse("oracle source must be external to the target checkout and repository");
        }
        let program_id = sha256(check.get("program_sha256")?.str()?)?;
        let oracle_id = sha256(check.get("oracle_sha256")?.str()?)?;
        let expected_stdout = sha256(check.get("expected_stdout_sha256")?.str()?)?;
        let args = owned_strings(check.get("args")?)?;
        if args.is_empty()
            || args.len() > MAX_ARGS
            || args
                .iter()
                .any(|a| a.len() > MAX_ARG_BYTES || a.contains('\0'))
            || args.first().map(String::as_str) != Some(oracle_str)
        {
            return refuse("oracle arguments must include the pinned external source path");
        }
        let timeout = u64::try_from(check.get("timeout_ms")?.integer()?)
            .map_err(|_| "negative oracle timeout")?;
        if !(100..=MAX_TIMEOUT_MS).contains(&timeout) {
            return refuse("oracle timeout must be between 100 and 15000 milliseconds");
        }
        pinned_file(&program, program_id, MAX_PROGRAM)?;
        pinned_file(&oracle, oracle_id, MAX_ORACLE_SOURCE)?;
        git::clean(&checkout)?;
        if git::rev(&checkout, "HEAD")? != commit
            || git::rev(&checkout, "HEAD^{tree}")? != git::rev(repo, &format!("{commit}^{{tree}}"))?
        {
            return refuse("oracle checkout does not match exact declared checkpoint/tree");
        }
        let executed = execute(&program, &args, &checkout, timeout)?;
        git::clean(&checkout)?;
        pinned_file(&program, program_id, MAX_PROGRAM)?;
        pinned_file(&oracle, oracle_id, MAX_ORACLE_SOURCE)?;
        if digest::sha256(&executed.stdout) != expected_stdout {
            return refuse("oracle output does not match independently declared expected bytes");
        }
        let mut source_entries = Vec::new();
        let mut annotations = Vec::new();
        let mut seen_paths = BTreeSet::new();
        for contract in &declared {
            let closure = sources
                .get(contract)
                .ok_or("missing contract source closure")?;
            for path in closure {
                if seen_paths.insert(path.clone()) {
                    let source = SourceRef::from_git(repo, &commit, path)?;
                    source_entries.push(obj(&[
                        ("path", Value::string(&source.path)),
                        ("blob", Value::string(&source.blob)),
                        ("mode", Value::string(&source.mode)),
                    ]));
                    annotations.push((contract.clone(), source));
                }
            }
        }
        source_observation_count += annotations.len();
        if source_observation_count > MAX_SOURCE_OBSERVATIONS {
            return refuse("oracle source-observation budget exceeded");
        }
        let raw = obj(&[
            (
                "schema",
                Value::string("excession.git-reconstruction-oracle-receipt.v1"),
            ),
            ("structural_report_id", structural.get("report_id")?.clone()),
            ("build_rea_report_id", evidence.get("report_id")?.clone()),
            ("phase", Value::string(&phase)),
            ("commit", Value::string(&commit)),
            (
                "tree",
                Value::string(git::rev(repo, &format!("{commit}^{{tree}}"))?),
            ),
            (
                "contracts",
                Value::array(declared.iter().map(Value::string)),
            ),
            ("source_closure", Value::array(source_entries)),
            ("oracle_source_sha256", Value::string(oracle_id)),
            ("program_sha256", Value::string(program_id)),
            ("argv", Value::array(args.iter().map(Value::string))),
            ("expected_stdout_sha256", Value::string(expected_stdout)),
            (
                "observed_stdout_sha256",
                Value::string(digest::sha256(&executed.stdout)),
            ),
            (
                "observed_stderr_sha256",
                Value::string(digest::sha256(&executed.stderr)),
            ),
            (
                "observed_stdout_bytes",
                Value::number(executed.stdout.len()),
            ),
            ("result", Value::string("EXPECTED_OUTPUT_OBSERVED")),
            (
                "execution_boundary",
                Value::string("TRUSTED_LOCAL_UNSANDBOXED"),
            ),
            ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
            ("environment_attestation", Value::string("NOT_ESTABLISHED")),
        ]);
        let receipt = Receipt {
            producer: PRODUCER.to_owned(),
            version: "1".to_owned(),
            native_id: raw.sha256(),
            raw_json: raw.canonical(),
        };
        let receipt_id = receipt.document()?.get("receipt_id")?.str()?.to_owned();
        let observations = annotations
            .into_iter()
            .map(|(contract, source)| {
                Ok(Observation {
                    receipt_id: receipt_id.clone(),
                    subject_id: Subject::Source(source).id()?,
                    native_kind: "BOUNDED_ORACLE_RESULT".to_owned(),
                    payload: obj(&[
                        ("phase", Value::string(&phase)),
                        ("contract", Value::string(contract)),
                        ("checkpoint", Value::string(&commit)),
                        ("oracle_receipt", Value::string(&receipt_id)),
                        ("scope", Value::string("ONE_TRUSTED_LOCAL_EXECUTION")),
                    ]),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let item = obj(&[
            ("phase", Value::string(&phase)),
            ("checkpoint", Value::string(&commit)),
            (
                "contracts",
                Value::array(declared.iter().map(Value::string)),
            ),
            ("receipt_id", Value::string(receipt_id)),
            ("source_closure_count", Value::number(seen_paths.len())),
            ("stdout_sha256", Value::string(expected_stdout)),
            ("oracle_result", Value::string("EXPECTED_OUTPUT_OBSERVED")),
            ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ]);
        prepared.push((commit, receipt, observations, item));
    }
    if observed.len() != coverage.len() {
        return refuse("incomplete oracle coverage");
    }
    if bounded_json(manifest_path)? != manifest || bounded_json(plan_path)? != plan {
        return refuse("oracle manifest or reconstruction plan changed during execution");
    }
    // No observation is persisted until the *entire* declared manifest passed.
    let mut items = Vec::new();
    for (commit, receipt, observations, mut item) in prepared {
        let snapshot = evidence_store::persist(
            db,
            &Batch {
                source_commit: commit,
                receipts: vec![receipt],
                observations,
            },
        )?;
        item.entries_mut()?
            .insert("snapshot_id".to_owned(), Value::string(snapshot));
        items.push(item);
    }
    items.sort_by_key(Value::canonical);
    let mut report = obj(&[
        (
            "schema",
            Value::string("excession.git-reconstruction-oracle-report.v1"),
        ),
        ("structural_report_id", structural.get("report_id")?.clone()),
        ("build_rea_report_id", evidence.get("report_id")?.clone()),
        ("oracle_manifest_sha256", Value::string(manifest.sha256())),
        ("checked_count", Value::number(items.len())),
        ("checks", Value::array(items)),
        (
            "check_status",
            Value::string("ALL_DECLARED_BOUNDED_CHECKS_PASSED"),
        ),
        (
            "build_rea_coverage",
            Value::string("COMPLETE_REPLAYED_BYTES_ONLY"),
        ),
        ("source_identity", Value::string("PINNED_GIT_SOURCE")),
        (
            "runtime_observation",
            Value::string("TRUSTED_LOCAL_UNSANDBOXED"),
        ),
        ("execution_attested", Value::Bool(false)),
        ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ("original_history_recovered", Value::Bool(false)),
    ]);
    let report_id = report.sha256();
    report
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(report_id));
    Ok(report)
}
