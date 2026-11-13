//! Bounded coding-agent adapter over existing Git, analysis, and impact queries.
//! Source identity is authenticated; candidate edges and caller-supplied receipts
//! are never promoted to verified program behavior.

use crate::json::{self, Value, obj};
use crate::{Result, analyze, change_inventory, discovery, git, impact_query, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const MAX_LIMIT: usize = 64;
const MAX_FOCUS: usize = 16;
const MAX_RECEIPT_BYTES: u64 = 1024 * 1024;
const MAX_RECEIPTS: usize = 32;

fn bounded_limit(limit: usize) -> Result<usize> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return refuse("agent --limit must be in 1..=64");
    }
    Ok(limit)
}

fn source(commit: &str, tree: &str, entry: &git::Entry) -> Value {
    obj(&[
        ("source_commit", Value::string(commit)),
        ("source_tree", Value::string(tree)),
        ("path", Value::string(&entry.path)),
        ("blob", Value::string(&entry.oid)),
        ("mode", Value::string(&entry.mode)),
    ])
}

fn gap(code: &str) -> Value {
    obj(&[("reason", Value::string(code))])
}

fn report_id(mut value: Value) -> Result<Value> {
    let id = value.sha256();
    value
        .entries_mut()?
        .insert("report_id".to_owned(), Value::string(id));
    Ok(value)
}

fn is_test(path: &str) -> bool {
    crate::source_capabilities::test_candidate(path)
}

fn matches_focus(test: &str, focus: &BTreeSet<String>) -> bool {
    focus.iter().any(|path| {
        let stem = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .split('.')
            .next()
            .unwrap_or("");
        stem.len() >= 3 && test.contains(stem)
    })
}

/// The task description is metadata only; lexical matching uses explicit Git
/// paths, not an invented semantic similarity score.
pub fn context(
    repo: &Path,
    rev: &str,
    focus_arg: &str,
    task: Option<&str>,
    limit: usize,
    roots: Option<&str>,
    database: Option<&str>,
) -> Result<Value> {
    let limit = bounded_limit(limit)?;
    if task.is_some_and(|s| s.len() > 4096) {
        return refuse("agent task description exceeds 4096 bytes");
    }
    let focus = focus_arg.split(',').map(str::trim).collect::<Vec<_>>();
    if focus.is_empty() || focus.len() > MAX_FOCUS || focus.iter().any(|p| p.is_empty()) {
        return refuse("agent context requires 1..=16 nonempty --focus Git paths");
    }
    let focus = focus
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for path in &focus {
        git::validate_path(path)?;
    }

    let discovered = discovery::inspect(repo, rev)?;
    let entries = git::entries(&discovered.root, &discovered.commit)?;
    let blobs = entries
        .iter()
        .filter(|entry| entry.kind == "blob" && matches!(entry.mode.as_str(), "100644" | "100755"))
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();

    let mut sources = Vec::new();
    for path in &focus {
        let entry = blobs
            .get(path.as_str())
            .ok_or_else(|| format!("agent focus is not a tracked regular Git source: {path}"))?;
        sources.push(source(&discovered.commit, &discovered.tree, entry));
    }

    let mut gaps = Vec::new();
    let mut relations = Vec::new();
    let mut total_relations = 0usize;
    let mut truncated = false;
    match analyze::run_if_supported(&discovered.root, &discovered.commit, roots, database)? {
        Some((analysis, _)) => {
            for relationship in analysis.get("relationships")?.array_ref()? {
                let from = relationship.get("source")?.get("path")?.str()?;
                let to = relationship.get("target")?.get("path")?.str()?;
                if focus.contains(from) || focus.contains(to) {
                    total_relations += 1;
                    if relations.len() < limit {
                        relations.push(relationship.clone());
                    } else {
                        truncated = true;
                    }
                }
            }
            for item in analysis.get("coverage")?.get("gaps")?.array_ref()? {
                if gaps.len() < limit {
                    gaps.push(item.clone());
                } else {
                    truncated = true;
                }
            }
        }
        None => gaps.push(gap("NO_SUPPORTED_STATIC_MODULE_INDEX")),
    }

    let mut test_candidates = Vec::new();
    let mut total_tests = 0usize;
    for entry in &entries {
        if entry.kind != "blob"
            || !matches!(entry.mode.as_str(), "100644" | "100755")
            || !is_test(&entry.path)
            || !matches_focus(&entry.path, &focus)
        {
            continue;
        }
        total_tests += 1;
        if test_candidates.len() < limit {
            test_candidates.push(obj(&[
                (
                    "source",
                    source(&discovered.commit, &discovered.tree, entry),
                ),
                ("reason", Value::string("PATH_NAME_HEURISTIC")),
                ("selection_verified", Value::Bool(false)),
            ]));
        } else {
            truncated = true;
        }
    }
    gaps.push(gap("TEST_SELECTION_NOT_PROVEN"));
    gaps.push(gap("TASK_SEMANTICS_NOT_INTERPRETED"));
    if truncated {
        gaps.push(gap("TRUNCATED"));
    }
    report_id(obj(&[
        ("schema", Value::string("excession.agent-context.v1")),
        ("source_commit", Value::string(&discovered.commit)),
        ("source_tree", Value::string(&discovered.tree)),
        ("task", task.map(Value::string).unwrap_or(Value::Null)),
        ("sources", Value::array(sources)),
        ("relationships", Value::array(relations)),
        ("relationship_count", Value::number(total_relations)),
        ("test_candidates", Value::array(test_candidates)),
        ("test_candidate_count", Value::number(total_tests)),
        (
            "coverage",
            obj(&[
                ("status", Value::string("PARTIAL")),
                ("gaps", Value::array(gaps)),
                ("truncated", Value::Bool(truncated)),
            ]),
        ),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]))
}

fn lower_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Import external execution observations as *unattested reports*. The
/// producer must independently execute tests/builds in its own isolated
/// environment. Excession does not run untrusted commands.
fn receipts(path: &Path, commit: &str, tree: &str) -> Result<Vec<Value>> {
    let size = fs::metadata(path)
        .map_err(|e| format!("agent receipt metadata: {e}"))?
        .len();
    if size > MAX_RECEIPT_BYTES {
        return refuse("agent receipts exceed 1 MiB");
    }
    let data = json::parse(&fs::read(path).map_err(|e| format!("agent receipts: {e}"))?)?;
    if data.get("schema")?.str()? != "excession.execution-receipts.v1" {
        return refuse("unsupported execution receipt schema");
    }
    if data.entries()?.len() != 2 {
        return refuse("execution receipts require only schema and receipts");
    }
    let rows = data.get("receipts")?.array_ref()?;
    if rows.len() > MAX_RECEIPTS {
        return refuse("agent receipts exceed 32 records");
    }
    let expected = BTreeSet::from([
        "kind",
        "status",
        "source_commit",
        "source_tree",
        "producer",
        "command",
        "exit_code",
        "log_sha256",
    ]);
    let mut imported = Vec::new();
    let mut unique = BTreeSet::new();
    for row in rows {
        let actual = row
            .entries()?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return refuse("execution receipt has missing or unsupported fields");
        }
        let kind = row.get("kind")?.str()?;
        let status = row.get("status")?.str()?;
        if !matches!(kind, "TEST" | "BUILD") || !matches!(status, "PASS" | "FAIL" | "SKIPPED") {
            return refuse("unsupported execution receipt kind or status");
        }
        if row.get("source_commit")?.str()? != commit || row.get("source_tree")?.str()? != tree {
            return refuse("execution receipt does not bind to candidate commit/tree");
        }
        let producer = row.get("producer")?.str()?;
        let command = row.get("command")?.str()?;
        if producer.is_empty() || producer.len() > 128 || command.is_empty() || command.len() > 2048
        {
            return refuse("execution receipt producer/command length invalid");
        }
        if !lower_sha256(row.get("log_sha256")?.str()?) {
            return refuse("execution receipt requires lowercase log SHA-256");
        }
        let code = row.get("exit_code")?.integer()?;
        if code < 0 || (status == "PASS" && code != 0) || (status == "FAIL" && code == 0) {
            return refuse("execution receipt exit code contradicts reported status");
        }
        let id = row.sha256();
        if !unique.insert(id.clone()) {
            return refuse("duplicate execution receipt");
        }
        imported.push(obj(&[
            ("receipt_id", Value::string(id)),
            ("observation", row.clone()),
            ("trust", Value::string("CALLER_SUPPLIED_UNATTESTED")),
        ]));
    }
    Ok(imported)
}

fn take_bounded(source: &[Value], limit: usize) -> Vec<Value> {
    source.iter().take(limit).cloned().collect()
}

/// Recomputes the existing bounded impact analysis against exact base and
/// candidate Git objects, then accounts for external test/build observations.
/// The result never promotes passing tests to a complete safety guarantee.
pub fn verify(
    repo: &Path,
    base: &str,
    candidate: &str,
    limit: usize,
    roots: Option<&str>,
    database: Option<&str>,
    receipts_path: Option<&Path>,
) -> Result<Value> {
    let limit = bounded_limit(limit)?;
    let changes = change_inventory::inventory(repo, base, candidate)?;
    let base_commit = changes.get("base_commit")?.str()?;
    let candidate_commit = changes.get("head_commit")?.str()?;
    if base_commit == candidate_commit {
        return refuse("agent verify requires two distinct Git commits");
    }
    let candidate_tree = changes.get("head_tree")?.str()?;
    let impact = impact_query::run(repo, base_commit, candidate_commit, roots, database)?;
    if impact.get("change_inventory_id")? != changes.get("report_id")? {
        return refuse("impact replay does not match exact Git change inventory");
    }
    let imported = match receipts_path {
        Some(path) => receipts(path, candidate_commit, candidate_tree)?,
        None => Vec::new(),
    };

    let mut obligations = BTreeSet::<String>::new();
    obligations.insert("SEMANTIC_CORRECTNESS_NOT_EVALUATED".into());
    obligations.insert("INCOMPLETE_DEPENDENCY_COVERAGE".into());
    obligations.insert("TEST_SELECTION_NOT_PROVEN".into());
    let mut kinds = BTreeSet::new();
    let mut failure = false;
    for item in &imported {
        let row = item.get("observation")?;
        kinds.insert(row.get("kind")?.str()?);
        failure |= row.get("status")?.str()? == "FAIL";
        if row.get("status")?.str()? == "SKIPPED" {
            obligations.insert("REPORTED_EXECUTION_SKIPPED".into());
        }
    }
    if !kinds.contains("TEST") {
        obligations.insert("INDEPENDENT_TEST_RECEIPT_MISSING".into());
    }
    if !kinds.contains("BUILD") {
        obligations.insert("INDEPENDENT_BUILD_RECEIPT_MISSING".into());
    }
    if !imported.is_empty() {
        obligations.insert("RECEIPT_AUTHENTICITY_UNVERIFIED".into());
    }
    if failure {
        obligations.insert("REPORTED_TEST_OR_BUILD_FAILURE".into());
    }

    let all_changes = changes.get("changes")?.array_ref()?;
    let effects = impact.get("effects")?.array_ref()?;
    let unknown = impact.get("unknown")?.array_ref()?;
    if !unknown.is_empty() {
        obligations.insert("UNRESOLVED_SOURCE_COVERAGE".into());
    }
    let truncated = all_changes.len() > limit || effects.len() > limit || unknown.len() > limit;
    if truncated {
        obligations.insert("TRUNCATED_OUTPUT".into());
    }
    report_id(obj(&[
        ("schema", Value::string("excession.agent-verification.v1")),
        ("base_commit", changes.get("base_commit")?.clone()),
        ("base_tree", changes.get("base_tree")?.clone()),
        ("candidate_commit", changes.get("head_commit")?.clone()),
        ("candidate_tree", changes.get("head_tree")?.clone()),
        ("change_inventory_id", changes.get("report_id")?.clone()),
        ("impact_report_id", impact.get("report_id")?.clone()),
        ("changes", Value::array(take_bounded(all_changes, limit))),
        ("change_count", Value::number(all_changes.len())),
        (
            "candidate_impacts",
            Value::array(take_bounded(effects, limit)),
        ),
        ("candidate_impact_count", Value::number(effects.len())),
        ("unknown", Value::array(take_bounded(unknown, limit))),
        ("receipts", Value::array(imported)),
        (
            "replay",
            obj(&[
                ("source", Value::string("RECOMPUTED_FROM_PINNED_GIT")),
                (
                    "supported_evidence",
                    Value::string("RECOMPUTED_NOT_INDEPENDENTLY_CERTIFIED"),
                ),
                ("semantic_correctness", Value::string("NOT_EVALUATED")),
            ]),
        ),
        (
            "coverage",
            obj(&[
                ("status", Value::string("PARTIAL")),
                ("truncated", Value::Bool(truncated)),
            ]),
        ),
        (
            "obligations",
            Value::array(obligations.into_iter().map(Value::string)),
        ),
        (
            "status",
            Value::string(if failure {
                "REPORTED_FAILURE"
            } else {
                "OPEN_OBLIGATIONS"
            }),
        ),
        ("ready_to_merge", Value::Bool(false)),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidates_are_explicit_path_heuristics() {
        let focus = BTreeSet::from(["core/src/impact_query.rs".into()]);
        assert!(is_test("core/tests/test_impact_query.rs"));
        assert!(matches_focus("core/tests/test_impact_query.rs", &focus));
        assert!(!matches_focus("core/tests/test_security.rs", &focus));
        assert!(!is_test("core/src/impact_query.rs"));
    }

    #[test]
    fn agent_outputs_have_strict_nonzero_bounds() {
        assert!(bounded_limit(0).is_err());
        assert!(bounded_limit(65).is_err());
        assert_eq!(bounded_limit(64).unwrap(), 64);
    }

    #[test]
    fn report_identity_commits_to_every_obligation() {
        let a = report_id(obj(&[("obligations", Value::array(vec![gap("MISSING")]))])).unwrap();
        let b = report_id(obj(&[("obligations", Value::array(vec![gap("PRESENT")]))])).unwrap();
        assert_ne!(a.get("report_id").unwrap(), b.get("report_id").unwrap());
    }

    fn git_fixture() -> (std::path::PathBuf, String, String) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-agent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        let command = |args: &[&str]| {
            let result = std::process::Command::new("git")
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
        command(&["init", "-q"]);
        fs::write(root.join("src/service.rs"), "fn service() {}\n").unwrap();
        fs::write(
            root.join("tests/test_service.rs"),
            "#[test] fn service() {}\n",
        )
        .unwrap();
        command(&["add", "."]);
        command(&[
            "-c",
            "user.name=Excession Test",
            "-c",
            "user.email=excession@example.invalid",
            "commit",
            "-qm",
            "base",
        ]);
        let base = git::rev(&root, "HEAD").unwrap();
        fs::write(
            root.join("src/service.rs"),
            "fn service() { println!(\"test\"); }\n",
        )
        .unwrap();
        command(&["add", "."]);
        command(&[
            "-c",
            "user.name=Excession Test",
            "-c",
            "user.email=excession@example.invalid",
            "commit",
            "-qm",
            "candidate",
        ]);
        let candidate = git::rev(&root, "HEAD").unwrap();
        (root, base, candidate)
    }

    #[test]
    fn pinned_context_and_verify_keep_unsupported_work_explicit() {
        let (root, base, candidate) = git_fixture();
        let ctx = context(
            &root,
            &base,
            "src/service.rs",
            Some("change service"),
            4,
            None,
            None,
        )
        .unwrap();
        assert_eq!(ctx.get("source_commit").unwrap().str().unwrap(), base);
        assert_eq!(
            ctx.get("relationships").unwrap().array_ref().unwrap().len(),
            0
        );
        assert_eq!(
            ctx.get("test_candidates")
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            1
        );
        assert!(
            !ctx.get("safe_to_skip_unlisted_tests")
                .unwrap()
                .bool()
                .unwrap()
        );
        assert!(context(&root, &base, "../outside", None, 4, None, None).is_err());
        assert!(context(&root, &base, "missing.rs", None, 4, None, None).is_err());

        let verified = verify(&root, &base, &candidate, 4, None, None, None).unwrap();
        assert_eq!(
            verified.get("status").unwrap().str().unwrap(),
            "OPEN_OBLIGATIONS"
        );
        assert_eq!(verified.get("change_count").unwrap().integer().unwrap(), 1);
        assert!(!verified.get("ready_to_merge").unwrap().bool().unwrap());
        let obligations = verified.get("obligations").unwrap().array_ref().unwrap();
        assert!(
            obligations
                .iter()
                .any(|x| { x.str().unwrap() == "INDEPENDENT_TEST_RECEIPT_MISSING" })
        );
        assert!(
            obligations
                .iter()
                .any(|x| { x.str().unwrap() == "INCOMPLETE_DEPENDENCY_COVERAGE" })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_pinned_external_receipts_remain_unattested_and_refuse_stale_tree() {
        let (root, base, candidate) = git_fixture();
        let tree = git::rev(&root, &format!("{candidate}^{{tree}}")).unwrap();
        let one = obj(&[
            ("kind", Value::string("TEST")),
            ("status", Value::string("PASS")),
            ("source_commit", Value::string(&candidate)),
            ("source_tree", Value::string(&tree)),
            ("producer", Value::string("isolated-ci")),
            ("command", Value::string("cargo test --locked")),
            ("exit_code", Value::number(0)),
            ("log_sha256", Value::string("a".repeat(64))),
        ]);
        let document = |row: Value| {
            obj(&[
                ("schema", Value::string("excession.execution-receipts.v1")),
                ("receipts", Value::array(vec![row])),
            ])
        };
        let receipt_path = std::env::temp_dir().join(format!(
            "excession-agent-receipts-{}.json",
            &candidate[..16],
        ));
        fs::write(&receipt_path, document(one.clone()).canonical()).unwrap();
        let report = verify(&root, &base, &candidate, 8, None, None, Some(&receipt_path)).unwrap();
        let obligations = report.get("obligations").unwrap().array_ref().unwrap();
        assert!(
            obligations
                .iter()
                .any(|x| { x.str().unwrap() == "RECEIPT_AUTHENTICITY_UNVERIFIED" })
        );
        assert!(!report.get("ready_to_merge").unwrap().bool().unwrap());
        let mut wrong = one;
        wrong
            .entries_mut()
            .unwrap()
            .insert("source_tree".into(), Value::string("f".repeat(40)));
        fs::write(&receipt_path, document(wrong).canonical()).unwrap();
        assert!(verify(&root, &base, &candidate, 8, None, None, Some(&receipt_path),).is_err());
        fs::remove_file(receipt_path).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn receipts_do_not_promote_unattested_pass_to_proof() {
        assert!(lower_sha256(&"a".repeat(64)));
        assert!(!lower_sha256(&"A".repeat(64)));
        assert!(!lower_sha256("abc"));
        let row = obj(&[
            ("kind", Value::string("TEST")),
            ("status", Value::string("PASS")),
            ("source_commit", Value::string("c".repeat(40))),
            ("source_tree", Value::string("d".repeat(40))),
            ("producer", Value::string("ci")),
            ("command", Value::string("cargo test")),
            ("exit_code", Value::number(0)),
            ("log_sha256", Value::string("e".repeat(64))),
        ]);
        let mut mismatched = row.clone();
        mismatched
            .entries_mut()
            .unwrap()
            .insert("source_tree".into(), Value::string("f".repeat(40)));
        assert_ne!(mismatched.sha256(), row.sha256());
    }
}
