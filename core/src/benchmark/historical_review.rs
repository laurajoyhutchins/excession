//! Historical review protocol: blinded Git workspaces, pinned oracle receipts,
//! condition-specific evidence, and paired decision accounting.
//!
//! This deliberately does not execute agents or arbitrary historical tests.
//! A correct gate decision is a proxy, not proof that a valid fix was produced.

use crate::{Result, agent, digest, git, json as strict_json, refuse};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const STUDY: &str = "excession.historical-review-study.v1";
const ORACLE: &str = "excession.historical-review-oracle.v1";
const ASSIGNMENT: &str = "excession.historical-review-assignment.v1";
const ATTEMPT: &str = "excession.historical-review-attempt.v1";
const RESULT: &str = "excession.historical-review-results.v1";
const ARMS: [&str; 4] = ["baseline", "legacy_paths", "graph", "excession"];
const MAX_TRANSCRIPT: u64 = 8 * 1024 * 1024;

fn load(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    strict_json::parse(&bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("JSON {}: {e}", path.display()))
}
fn value<'a>(item: &'a Value, key: &str) -> Result<&'a Value> {
    item.get(key).ok_or_else(|| format!("missing {key}"))
}
fn string<'a>(item: &'a Value, key: &str) -> Result<&'a str> {
    value(item, key)?
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn number(item: &Value, key: &str) -> Result<u64> {
    value(item, key)?
        .as_u64()
        .ok_or_else(|| format!("{key} must be a nonnegative integer"))
}
fn list<'a>(item: &'a Value, key: &str) -> Result<&'a [Value]> {
    value(item, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{key} must be an array"))
}
fn sha(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn bytes_sha(path: &Path) -> Result<String> {
    Ok(digest::sha256(
        &fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?,
    ))
}
fn encoded(item: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(item).map_err(|e| format!("serialize review data: {e}"))
}
fn write_new(path: &Path, item: &Value) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("create {} (no overwrite): {e}", path.display()))?;
    f.write_all(&encoded(item)?)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    f.sync_all()
        .map_err(|e| format!("sync {}: {e}", path.display()))
}
fn git_output(path: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("launch Git: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
fn tree(repo: &Path, rev: &str) -> Result<String> {
    let oid = git::oid(rev)?;
    if git_output(repo, &["cat-file", "-t", &oid])? != "commit" {
        return refuse("review revision is not a commit");
    }
    git::rev(repo, &format!("{oid}^{{tree}}"))
}
fn ancestor(repo: &Path, first: &str, second: &str) -> Result<()> {
    git_output(repo, &["merge-base", "--is-ancestor", first, second])?;
    Ok(())
}
fn receipt(entry: &Value, revision: &str, expected_pass: bool, repo: &Path) -> Result<String> {
    let path = Path::new(string(entry, "path")?);
    let expected_digest = string(entry, "sha256")?;
    if !path.is_absolute() || !sha(expected_digest, 64) || bytes_sha(path)? != expected_digest {
        return refuse("oracle receipt requires an authenticated absolute path and SHA-256");
    }
    let raw = load(path)?;
    if string(&raw, "schema")? != ORACLE
        || string(&raw, "commit")? != revision
        || string(&raw, "tree")? != tree(repo, revision)?
    {
        return refuse("oracle receipt is not bound to the specified Git commit and tree");
    }
    let cmd = list(&raw, "command")?;
    if cmd.is_empty()
        || cmd.len() > 40
        || cmd.iter().any(|x| x.as_str().is_none_or(|s| s.is_empty()))
    {
        return refuse("oracle receipt needs a nonempty argument-vector command");
    }
    if !sha(string(&raw, "stdout_sha256")?, 64)
        || !sha(string(&raw, "stderr_sha256")?, 64)
        || string(&raw, "runner")?.is_empty()
    {
        return refuse("oracle receipt needs output digests and a runner identity");
    }
    let exit = number(&raw, "exit_code")?;
    if exit > 255 || (exit == 0) != expected_pass {
        return refuse("independent oracle receipts contradict the registered behavior");
    }
    serde_json::to_string(cmd).map_err(|e| e.to_string())
}

fn validated(study: &Value) -> Result<()> {
    if string(study, "schema")? != STUDY || !slug(string(study, "id")?) {
        return refuse("unsupported study schema or identifier");
    }
    let repetitions = number(study, "repetitions")?;
    let cases = list(study, "cases")?;
    if !(1..=10).contains(&repetitions) || cases.is_empty() || cases.len() > 1000 {
        return refuse("study requires 1..=10 repetitions and 1..=1000 cases");
    }
    let mut ids = BTreeSet::new();
    for case in cases {
        let id = string(case, "id")?;
        if !slug(id) || !ids.insert(id) {
            return refuse("invalid or repeated case identifier");
        }
        let root = Path::new(string(case, "repo_path")?);
        if !root.is_absolute() {
            return refuse("source repo must be an absolute local path");
        }
        git::verify_repo(root)?;
        let base = string(case, "base")?;
        let candidate = string(case, "candidate")?;
        if !sha(base, 40) || !sha(candidate, 40) || base == candidate {
            return refuse("case requires two distinct pinned Git SHA-1 commits");
        }
        tree(root, base)?;
        tree(root, candidate)?;
        ancestor(root, base, candidate)?;
        if string(case, "task")?.is_empty() || string(case, "task")?.len() > 4096 {
            return refuse("case task must be nonempty and at most 4096 bytes");
        }
        let roots = string(case, "roots")?;
        if roots.is_empty() || roots.len() > 512 {
            return refuse("case roots must be nonempty and bounded");
        }
        let truth = string(case, "truth")?;
        if !matches!(truth, "REGRESSION" | "CLEAN") {
            return refuse("case truth must be REGRESSION or CLEAN");
        }
        let oracles = value(case, "oracles")?;
        let a = receipt(value(oracles, "base")?, base, true, root)?;
        let b = receipt(
            value(oracles, "candidate")?,
            candidate,
            truth == "CLEAN",
            root,
        )?;
        if a != b {
            return refuse("base and candidate oracle commands must match exactly");
        }
        match truth {
            "REGRESSION" => {
                let fix = string(case, "repair")?;
                if !sha(fix, 40) || fix == candidate || fix == base {
                    return refuse("regression requires a distinct pinned repaired commit");
                }
                ancestor(root, candidate, fix)?;
                let c = receipt(value(oracles, "repair")?, fix, true, root)?;
                if b != c {
                    return refuse("repair oracle command differs from baseline");
                }
            }
            "CLEAN" => {
                if case.get("repair").is_some_and(|v| !v.is_null())
                    || oracles.get("repair").is_some()
                {
                    return refuse("clean control cannot contain a repair or repair oracle");
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn find_case<'a>(study: &'a Value, id: &str) -> Result<&'a Value> {
    list(study, "cases")?
        .iter()
        .find(|c| c["id"] == id)
        .ok_or_else(|| format!("unknown case {id}"))
}
fn arm(name: &str) -> Result<()> {
    if !ARMS.contains(&name) {
        return refuse(format!("unknown condition {name}"));
    }
    Ok(())
}
fn evidence(report: &Value, condition: &str) -> Result<Value> {
    let mut common = json!({
        "schema": "excession.historical-review-evidence.v1",
        "condition": condition,
        "base_commit": string(report, "base_commit")?,
        "candidate_commit": string(report, "candidate_commit")?,
        "changes": value(report, "changes")?,
        "change_count": number(report, "change_count")?,
        "coverage": value(report, "coverage")?,
        "safe_to_skip_unlisted_tests": false
    });
    match condition {
        "legacy_paths" => {
            common["candidate_impacts"] = value(report, "candidate_impacts")?.clone();
            common["candidate_impact_count"] = value(report, "candidate_impact_count")?.clone();
        }
        "graph" => {
            let entries: Vec<Value> = list(report, "candidate_source_impacts")?.iter()
                .map(|item| json!({"entity":item["entity"],"distance_to_change":item["distance_to_change"]}))
                .collect();
            common["candidate_source_impacts"] = json!(entries);
            common["candidate_source_count"] = value(report, "candidate_source_count")?.clone();
            common["candidate_source_enumeration_complete"] =
                value(report, "candidate_source_enumeration_complete")?.clone();
        }
        "excession" => {
            common["candidate_source_impacts"] = value(report, "candidate_source_impacts")?.clone();
            common["candidate_source_count"] = value(report, "candidate_source_count")?.clone();
            common["candidate_source_enumeration_complete"] =
                value(report, "candidate_source_enumeration_complete")?.clone();
            common["unknown"] = value(report, "unknown")?.clone();
            common["obligations"] = value(report, "obligations")?.clone();
            common["source_report_id"] = value(report, "report_id")?.clone();
        }
        _ => return refuse("baseline has no evidence file"),
    }
    Ok(common)
}

/// Generate an isolated Git object database containing the candidate and its
/// ancestors only. The hidden study, independent oracle, and repair are never
/// copied into the exported assignment. OS-level workspace isolation is still
/// the responsibility of the agent runner.
pub fn prepare(
    study_path: &Path,
    case_id: &str,
    condition: &str,
    repetition: u64,
    out: &Path,
) -> Result<PathBuf> {
    arm(condition)?;
    let study = load(study_path)?;
    // Validate the full study at scoring time. Per-assignment preparation only
    // checks the selected Git/oracle case to avoid O(cases^2) Git processes.
    let all_cases = list(&study, "cases")?;
    let mut seen = BTreeSet::new();
    for item in all_cases {
        if !seen.insert(string(item, "id")?) {
            return refuse("duplicate study case IDs");
        }
    }
    let mut selected_study = study.clone();
    selected_study["cases"] = json!([find_case(&study, case_id)?]);
    validated(&selected_study)?;
    if repetition == 0 || repetition > number(&study, "repetitions")? {
        return refuse("repetition is outside preregistered study range");
    }
    let case = find_case(&study, case_id)?;
    if out.exists() {
        return refuse("review assignment output already exists");
    }
    let source = Path::new(string(case, "repo_path")?);
    let base = string(case, "base")?;
    let candidate = string(case, "candidate")?;
    let candidate_tree = tree(source, candidate)?;
    fs::create_dir_all(out.join("workspace")).map_err(|e| format!("create assignment: {e}"))?;
    let result = (|| {
        let workspace = out.join("workspace");
        let o = Command::new("git")
            .args(["init", "-q", "--object-format=sha1"])
            .arg(&workspace)
            .output()
            .map_err(|e| format!("initialize blinded Git repository: {e}"))?;
        if !o.status.success() {
            return refuse(format!("git init: {}", String::from_utf8_lossy(&o.stderr)));
        }
        let source_text = source.to_str().ok_or("source path is not UTF-8")?;
        git_output(
            &workspace,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                source_text,
                candidate,
            ],
        )?;
        git_output(&workspace, &["checkout", "-q", "--detach", candidate])?;
        if tree(&workspace, candidate)? != candidate_tree
            || git_output(&workspace, &["rev-parse", "HEAD"])? != candidate
        {
            return refuse("blinded checkout does not match the pinned candidate");
        }
        if let Some(fix) = case.get("repair").and_then(Value::as_str) {
            let accessible = Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(["cat-file", "-e", &format!("{fix}^{{commit}}")])
                .output()
                .map_err(|e| format!("probe hidden repair object: {e}"))?;
            if accessible.status.success() {
                return refuse("hidden repair is accessible inside blinded Git object database");
            }
        }
        let context = if condition == "baseline" {
            None
        } else {
            let native = agent::verify(
                source,
                base,
                candidate,
                64,
                Some(string(case, "roots")?),
                None,
                None,
            )?;
            let report: Value = serde_json::from_str(&native.canonical())
                .map_err(|e| format!("parse native agent evidence: {e}"))?;
            let projected = evidence(&report, condition)?;
            write_new(&out.join("evidence.json"), &projected)?;
            Some(bytes_sha(&out.join("evidence.json"))?)
        };
        let prompt = format!(
            "Review the proposed change from commit {base} to {candidate}.\n\
             Task: {}\n\
             Decide whether to APPROVE, BLOCK, or ABSTAIN. Cite concrete source or test evidence.\n\
             Treat static dependency edges as incomplete. Do not search remotes or access external files.\n\
             If an evidence.json file was supplied, you may inspect it; it does not certify correctness.\n",
            string(case, "task")?
        );
        let prompt_path = out.join("prompt.txt");
        fs::write(&prompt_path, prompt).map_err(|e| format!("write prompt: {e}"))?;
        let mut assignment = json!({
            "schema": ASSIGNMENT,
            "study_id": string(&study, "id")?,
            "study_sha256": bytes_sha(study_path)?,
            "case_id": case_id,
            "condition": condition,
            "repetition": repetition,
            "base_commit": base,
            "candidate_commit": candidate,
            "candidate_tree": candidate_tree,
            "prompt_sha256": bytes_sha(&prompt_path)?,
            "evidence_sha256": context,
            "workspace": "workspace",
            "agent_budget": "EXTERNALLY_CONTROLLED"
        });
        let id = digest::sha256(&serde_json::to_vec(&assignment).map_err(|e| e.to_string())?);
        assignment["assignment_id"] = json!(id);
        let path = out.join("assignment.json");
        write_new(&path, &assignment)?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(out);
    }
    result
}

/// Record a completed externally run review, without manufacturing token
/// measurements or accepting a self-declared ground truth label.
pub fn record(
    assignment_path: &Path,
    transcript: &Path,
    decision: &str,
    out: &Path,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
) -> Result<()> {
    if !matches!(decision, "APPROVE" | "BLOCK" | "ABSTAIN") {
        return refuse("decision must be APPROVE, BLOCK, or ABSTAIN");
    }
    if input_tokens.is_some() != output_tokens.is_some() {
        return refuse("input and output token counts must both be present or both omitted");
    }
    let assignment = load(assignment_path)?;
    if string(&assignment, "schema")? != ASSIGNMENT {
        return refuse("invalid assignment schema");
    }
    let evidence_path = assignment_path
        .parent()
        .ok_or("assignment has no parent")?
        .join("evidence.json");
    let expected_evidence = value(&assignment, "evidence_sha256")?;
    if expected_evidence.is_null() != !evidence_path.exists()
        || (!expected_evidence.is_null()
            && string(&assignment, "evidence_sha256")? != bytes_sha(&evidence_path)?)
    {
        return refuse("assignment evidence changed or is missing");
    }
    let prompt_path = assignment_path
        .parent()
        .ok_or("assignment has no parent")?
        .join("prompt.txt");
    if string(&assignment, "prompt_sha256")? != bytes_sha(&prompt_path)? {
        return refuse("assignment prompt changed");
    }
    let mut unsigned = assignment.clone();
    let id = string(&assignment, "assignment_id")?.to_owned();
    unsigned
        .as_object_mut()
        .ok_or("assignment not an object")?
        .remove("assignment_id");
    if digest::sha256(&serde_json::to_vec(&unsigned).map_err(|e| e.to_string())?) != id {
        return refuse("assignment identity mismatch");
    }
    let size = fs::metadata(transcript)
        .map_err(|e| format!("transcript metadata: {e}"))?
        .len();
    if !(1..=MAX_TRANSCRIPT).contains(&size) {
        return refuse("transcript must be 1 byte..8 MiB");
    }
    let entry = json!({
        "schema": ATTEMPT,
        "study_id": string(&assignment, "study_id")?,
        "study_sha256": string(&assignment, "study_sha256")?,
        "case_id": string(&assignment, "case_id")?,
        "condition": string(&assignment, "condition")?,
        "repetition": number(&assignment, "repetition")?,
        "base_commit": string(&assignment, "base_commit")?,
        "candidate_commit": string(&assignment, "candidate_commit")?,
        "assignment_path": assignment_path.canonicalize().map_err(|e| e.to_string())?.to_string_lossy(),
        "assignment_sha256": bytes_sha(assignment_path)?,
        "assignment_id": id,
        "transcript_path": transcript.canonicalize().map_err(|e| e.to_string())?.to_string_lossy(),
        "transcript_sha256": bytes_sha(transcript)?,
        "transcript_bytes": size,
        "decision": decision,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "token_source": if input_tokens.is_some() { "CALLER_REPORTED" } else { "NOT_MEASURED" },
        "signature_status": "UNATTESTED"
    });
    write_new(out, &entry)
}

fn verified_attempt(attempt: &Value, study: &Value, study_digest: &str) -> Result<()> {
    if string(attempt, "schema")? != ATTEMPT || string(attempt, "study_id")? != string(study, "id")?
    {
        return refuse("attempt has wrong schema or study identity");
    }
    if string(attempt, "study_sha256")? != study_digest {
        return refuse("attempt is bound to a different private study manifest");
    }
    arm(string(attempt, "condition")?)?;
    let case = find_case(study, string(attempt, "case_id")?)?;
    if string(attempt, "base_commit")? != string(case, "base")?
        || string(attempt, "candidate_commit")? != string(case, "candidate")?
        || number(attempt, "repetition")? == 0
        || number(attempt, "repetition")? > number(study, "repetitions")?
    {
        return refuse("attempt is stale or outside the preregistered cohort");
    }
    let assignment_path = Path::new(string(attempt, "assignment_path")?);
    let assignment = load(assignment_path)?;
    if bytes_sha(assignment_path)? != string(attempt, "assignment_sha256")?
        || string(&assignment, "assignment_id")? != string(attempt, "assignment_id")?
        || string(&assignment, "study_id")? != string(study, "id")?
        || string(&assignment, "study_sha256")? != study_digest
        || string(&assignment, "case_id")? != string(attempt, "case_id")?
        || string(&assignment, "condition")? != string(attempt, "condition")?
        || number(&assignment, "repetition")? != number(attempt, "repetition")?
        || string(&assignment, "candidate_commit")? != string(case, "candidate")?
    {
        return refuse("attempt is not bound to the original assignment");
    }
    // Evaluation authenticates the exact evidence and instructions originally
    // offered, not merely the JSON pointer to an assignment.
    let parent = assignment_path.parent().ok_or("assignment has no parent")?;
    if bytes_sha(&parent.join("prompt.txt"))? != string(&assignment, "prompt_sha256")? {
        return refuse("assignment prompt changed after recording");
    }
    let expected_evidence = value(&assignment, "evidence_sha256")?;
    let evidence_path = parent.join("evidence.json");
    if expected_evidence.is_null() != !evidence_path.exists()
        || (!expected_evidence.is_null()
            && string(&assignment, "evidence_sha256")? != bytes_sha(&evidence_path)?)
    {
        return refuse("assignment evidence changed after recording");
    }
    let transcript = Path::new(string(attempt, "transcript_path")?);
    if bytes_sha(transcript)? != string(attempt, "transcript_sha256")?
        || fs::metadata(transcript).map_err(|e| e.to_string())?.len()
            != number(attempt, "transcript_bytes")?
    {
        return refuse("attempt transcript changed");
    }
    if !matches!(
        string(attempt, "decision")?,
        "APPROVE" | "BLOCK" | "ABSTAIN"
    ) {
        return refuse("invalid decision");
    }
    if value(attempt, "input_tokens")?.is_null() != value(attempt, "output_tokens")?.is_null() {
        return refuse("partial token count");
    }
    if !value(attempt, "input_tokens")?.is_null() {
        number(attempt, "input_tokens")?;
        number(attempt, "output_tokens")?;
    }
    Ok(())
}

/// Evaluate gate decisions only. Rejects reused attempts and changed receipts.
/// An independent adjudication is still required before claiming prevention.
pub fn evaluate(study_path: &Path, attempts: &Path, out: &Path) -> Result<()> {
    let study = load(study_path)?;
    validated(&study)?;
    let study_digest = bytes_sha(study_path)?;
    let mut map = BTreeMap::<(String, u64, String), Value>::new();
    for file in fs::read_dir(attempts).map_err(|e| format!("read attempt directory: {e}"))? {
        let file = file.map_err(|e| e.to_string())?.path();
        if file.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let entry = load(&file)?;
        verified_attempt(&entry, &study, &study_digest)?;
        let key = (
            string(&entry, "case_id")?.to_owned(),
            number(&entry, "repetition")?,
            string(&entry, "condition")?.to_owned(),
        );
        if map.insert(key, entry).is_some() {
            return refuse("duplicate attempt for the same case, condition, and repetition");
        }
    }
    let mut observations = Vec::new();
    let mut totals = BTreeMap::<&str, (u64, u64, u64, u64, u64)>::new();
    let mut paired = Vec::new();
    let mut usage = BTreeMap::<&str, (u64, u64, u64)>::new();
    for case in list(&study, "cases")? {
        let case_id = string(case, "id")?;
        let truth = string(case, "truth")?;
        for repetition in 1..=number(&study, "repetitions")? {
            let mut by_condition = BTreeMap::new();
            for condition in ARMS {
                let key = (case_id.to_owned(), repetition, condition.to_owned());
                let attempt = map.get(&key);
                let decision = attempt.map(|r| string(r, "decision")).transpose()?;
                if let Some(attempt) = attempt
                    && !value(attempt, "input_tokens")?.is_null()
                {
                    let row = usage.entry(condition).or_default();
                    row.0 = row
                        .0
                        .checked_add(number(attempt, "input_tokens")?)
                        .ok_or("input token sum overflow")?;
                    row.1 = row
                        .1
                        .checked_add(number(attempt, "output_tokens")?)
                        .ok_or("output token sum overflow")?;
                    row.2 += 1;
                }
                let classification = match (truth, decision) {
                    (_, None) => "MISSING",
                    (_, Some("ABSTAIN")) => "ABSTAIN",
                    ("REGRESSION", Some("BLOCK")) => "CORRECT_GATE",
                    ("REGRESSION", Some("APPROVE")) => "ESCAPED_REGRESSION",
                    ("CLEAN", Some("APPROVE")) => "CORRECT_GATE",
                    ("CLEAN", Some("BLOCK")) => "FALSE_BLOCK",
                    _ => return refuse("unsupported study outcome"),
                };
                let row = totals.entry(condition).or_default();
                match classification {
                    "CORRECT_GATE" => row.0 += 1,
                    "ESCAPED_REGRESSION" => row.1 += 1,
                    "FALSE_BLOCK" => row.2 += 1,
                    "ABSTAIN" => row.3 += 1,
                    "MISSING" => row.4 += 1,
                    _ => unreachable!(),
                }
                observations.push(json!({
                    "case_id": case_id, "repetition": repetition, "condition": condition,
                    "truth": truth, "decision": decision, "classification": classification
                }));
                by_condition.insert(condition, classification);
            }
            if by_condition["baseline"] != "MISSING" && by_condition["excession"] != "MISSING" {
                paired.push(json!({
                    "case_id": case_id, "repetition": repetition,
                    "baseline": by_condition["baseline"], "excession": by_condition["excession"]
                }));
            }
        }
    }
    let total = list(&study, "cases")?.len() as u64 * number(&study, "repetitions")? * 4;
    let arms: Vec<Value> = ARMS
        .iter()
        .map(|name| {
            let (correct, escaped, false_block, abstain, missing) =
                totals.get(name).copied().unwrap_or_default();
            let (input, output, measured) = usage.get(name).copied().unwrap_or_default();
            json!({"condition":name,"correct_gate":correct,"escaped_regressions":escaped,
               "false_blocks":false_block,"abstentions":abstain,"missing":missing,
               "reported_input_tokens":input,"reported_output_tokens":output,
               "token_measurements":measured,
               "tokens_unmeasured":correct + escaped + false_block + abstain - measured})
        })
        .collect();
    let results = json!({
        "schema": RESULT, "study_id": string(&study, "id")?,
        "study_sha256": study_digest,
        "status": if map.len() as u64 == total { "COMPLETE" } else { "PARTIAL" },
        "expected_attempts": total, "received_attempts": map.len(),
        "metric": "GATE_DECISION_PROXY_NOT_DEMONSTRATED_BUG_PREVENTION",
        "by_condition": arms, "paired_baseline_excession": paired,
        "observations": observations,
        "notes": ["No code repairs were executed", "A correct BLOCK is not independently adjudicated",
                  "Token measurements are caller-reported or explicitly unmeasured"]
    });
    write_new(out, &results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn run_git(repo: &Path, args: &[&str]) {
        git_output(repo, args).unwrap();
    }
    fn commit(repo: &Path, source: &str) -> String {
        fs::write(repo.join("module.py"), source).unwrap();
        run_git(repo, &["add", "."]);
        run_git(
            repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "case",
            ],
        );
        git::rev(repo, "HEAD").unwrap()
    }
    #[test]
    fn blinded_workspaces_and_paired_decision_receipts() {
        let temp = std::env::temp_dir().join(format!(
            "excession-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = temp.join("repo");
        fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-q"]);
        let a = commit(&repo, "VALUE = 1\n");
        let b = commit(&repo, "VALUE = 0\n");
        let c = commit(&repo, "VALUE = 1  # restored\n");
        let mut oracles = serde_json::Map::new();
        for (part, revision, pass) in [
            ("base", &a, true),
            ("candidate", &b, false),
            ("repair", &c, true),
        ] {
            let source = git_output(&repo, &["show", &format!("{revision}:module.py")]).unwrap();
            assert_eq!(source != "VALUE = 0", pass);
            let receipt_path = temp.join(format!("{part}-oracle.json"));
            let content = json!({
                "schema": ORACLE, "commit": revision, "tree": tree(&repo, revision).unwrap(),
                "command": ["sh","-c","test \"$(cat module.py)\" != 'VALUE = 0'"],
                "exit_code": if pass { 0 } else { 1 },
                "stdout_sha256": digest::sha256(b""), "stderr_sha256": digest::sha256(b""),
                "runner": "local-test"
            });
            write_new(&receipt_path, &content).unwrap();
            oracles.insert(part.into(), json!({
                "path": receipt_path.to_str().unwrap(), "sha256": bytes_sha(&receipt_path).unwrap()
            }));
        }
        let manifest = temp.join("study.json");
        write_new(
            &manifest,
            &json!({
                "schema": STUDY, "id": "smoke", "repetitions": 1,
                "cases": [{"id":"sample","repo_path":repo.to_str().unwrap(),
                           "base":a,"candidate":b,"repair":c,
                           "truth":"REGRESSION","task":"Review changed default value",
                           "roots":"module.py","oracles":oracles}]
            }),
        )
        .unwrap();
        let attempts = temp.join("attempts");
        fs::create_dir_all(&attempts).unwrap();
        for condition in ARMS {
            let assignment =
                prepare(&manifest, "sample", condition, 1, &temp.join(condition)).unwrap();
            let workspace = temp.join(condition).join("workspace");
            assert_eq!(git::rev(&workspace, "HEAD").unwrap(), b);
            let repair_visible = Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(["cat-file", "-e", &c])
                .status()
                .unwrap();
            assert!(!repair_visible.success());
            let text = fs::read_to_string(&assignment).unwrap();
            assert!(!text.contains(&c));
            assert!(!text.contains("REGRESSION"));
            let transcript = temp.join(format!("{condition}.txt"));
            fs::write(&transcript, "Block: changed behavior needs tests\n").unwrap();
            record(
                &assignment,
                &transcript,
                "BLOCK",
                &attempts.join(format!("{condition}.json")),
                None,
                None,
            )
            .unwrap();
        }
        let result_path = temp.join("results.json");
        evaluate(&manifest, &attempts, &result_path).unwrap();
        let result = load(&result_path).unwrap();
        assert_eq!(result["status"], "COMPLETE");
        assert_eq!(result["by_condition"][0]["correct_gate"], 1);
        fs::write(temp.join("baseline.txt"), "tampered").unwrap();
        assert!(evaluate(&manifest, &attempts, &temp.join("tampered.json")).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn refuses_invalid_case_identifiers_and_decisions() {
        assert!(!slug("../repair"));
        assert!(!slug("white space"));
        assert!(arm("unknown").is_err());
        assert!(!sha("abcd", 40));
    }
}
