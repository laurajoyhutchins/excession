//! End-to-end acceptance for snapshot-bound, lossless lexical paging.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checked(mut command: Command) -> String {
    let output = command.output().expect("execute command");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    checked(command).trim().to_owned()
}

fn commit(repo: &Path) -> String {
    git(repo, &["add", "-A"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Index Test",
            "-c",
            "user.email=index@example.invalid",
            "commit",
            "-qm",
            "pinned source",
        ],
    );
    git(repo, &["rev-parse", "HEAD"])
}

fn output(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["agent", "locate-page", "--repo"])
        .arg(repo)
        .args(args)
        .output()
        .expect("run agent locate-page")
}

fn page(repo: &Path, args: &[&str]) -> Value {
    let result = output(repo, args);
    assert!(
        result.status.success(),
        "locate-page failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).expect("canonical page JSON")
}

#[test]
fn pages_cover_all_ranked_sources_without_duplicates_or_silent_truncation() {
    let root = Scratch(std::env::temp_dir().join(format!(
        "excession-locate-page-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    fs::create_dir_all(root.0.join("src")).unwrap();
    git(&root.0, &["init", "-q"]);
    for index in 0..9 {
        fs::write(
            root.0.join(format!("src/module_{index:02}.rs")),
            format!("pub fn module_{index}() {{ let needle = {index}; }}\n"),
        )
        .unwrap();
    }
    let original = commit(&root.0);
    let mut cursor: Option<String> = None;
    let mut paths = Vec::new();
    let mut report_ids = Vec::new();
    let mut snapshot = String::new();
    for turn in 0..9 {
        let mut args = vec![
            "--rev",
            original.as_str(),
            "--task",
            "needle",
            "--limit",
            if turn == 0 { "2" } else { "3" },
            "--index-segments",
            "256",
        ];
        if let Some(ref after) = cursor {
            args.extend(["--after", after.as_str()]);
        }
        let result = page(&root.0, &args);
        assert_eq!(result["schema"], "excession.agent-locate-page.v1");
        let contract: Value = serde_json::from_str(include_str!(
            "../../contracts/agent/locate-page.schema.json"
        ))
        .unwrap();
        assert!(
            jsonschema::draft202012::new(&contract)
                .unwrap()
                .is_valid(&result),
            "page contract mismatch: {result}"
        );
        assert_eq!(result["source_commit"], original);
        assert_eq!(result["index"]["complete"], true);
        assert_eq!(result["indexed_source_count"], Value::Null);
        assert_eq!(result["index"]["indexed_source_count"], 9);
        assert_eq!(result["safe_to_skip_unlisted_tests"], false);
        assert_eq!(result["coverage"]["status"], "PARTIAL");
        if turn == 0 {
            snapshot = result["page"]["snapshot_id"].as_str().unwrap().to_owned();
            assert_eq!(result["page"]["offset"], 0);
            assert_eq!(result["candidate_count"], 2);
        } else {
            assert_eq!(result["page"]["snapshot_id"], snapshot);
        }
        for item in result["candidates"].as_array().unwrap() {
            assert_eq!(item["rank"].as_u64().unwrap() as usize, paths.len() + 1);
            paths.push(item["source"]["path"].as_str().unwrap().to_owned());
        }
        report_ids.push(result["report_id"].as_str().unwrap().to_owned());
        cursor = result["page"]["next_cursor"].as_str().map(str::to_owned);
        assert_eq!(result["page"]["has_more"], cursor.is_some());
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(paths.len(), 9);
    let unique = paths.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 9);
    assert!(report_ids.len() > 2);
    assert_eq!(
        report_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        report_ids.len()
    );

    // The first continuation is deterministic across warm invocations and
    // does not depend on how many segments may be refreshed per invocation.
    let warm = page(
        &root.0,
        &[
            "--rev",
            &original,
            "--task",
            "needle",
            "--limit",
            "2",
            "--index-segments",
            "1",
        ],
    );
    assert_eq!(warm["page"]["snapshot_id"], snapshot);
    assert_eq!(warm["report_id"], report_ids[0]);

    // The cursor must not be silently accepted for a different query.
    let first = page(
        &root.0,
        &[
            "--rev",
            &original,
            "--task",
            "needle",
            "--limit",
            "2",
            "--index-segments",
            "1",
        ],
    );
    let first_cursor = first["page"]["next_cursor"].as_str().unwrap();
    let query_mismatch = output(
        &root.0,
        &[
            "--rev",
            &original,
            "--task",
            "other",
            "--limit",
            "2",
            "--after",
            first_cursor,
        ],
    );
    assert_eq!(query_mismatch.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&query_mismatch.stderr).contains("REFUSED:"));

    // The same unchanged Git tree under a different index policy is also
    // a different ranking snapshot, even when every file fits either policy.
    let policy_mismatch = output(
        &root.0,
        &[
            "--rev",
            &original,
            "--task",
            "needle",
            "--limit",
            "2",
            "--after",
            first_cursor,
            "--max-file-bytes",
            "256",
            "--max-segment-bytes",
            "32768",
        ],
    );
    assert_eq!(policy_mismatch.status.code(), Some(2));

    fs::write(
        root.0.join("src/new.rs"),
        "pub fn new() { let needle = 1; }\n",
    )
    .unwrap();
    let changed = commit(&root.0);
    assert_ne!(original, changed);
    let stale = output(
        &root.0,
        &[
            "--rev",
            "HEAD",
            "--task",
            "needle",
            "--limit",
            "2",
            "--after",
            first_cursor,
            "--index-segments",
            "256",
        ],
    );
    assert_eq!(stale.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&stale.stderr).contains("REFUSED:"));

    // The earlier immutable revision remains queryable with its own cursor.
    let original_again = page(
        &root.0,
        &[
            "--rev",
            &original,
            "--task",
            "needle",
            "--limit",
            "2",
            "--after",
            first_cursor,
            "--index-segments",
            "256",
        ],
    );
    assert_eq!(original_again["source_commit"], original);
    assert_eq!(original_again["page"]["offset"], 2);
}

#[test]
fn partial_index_paging_never_claims_missing_sources_are_absent() {
    let root = Scratch(std::env::temp_dir().join(format!(
        "excession-locate-page-partial-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    fs::create_dir_all(root.0.join("src")).unwrap();
    git(&root.0, &["init", "-q"]);
    for index in 0..60 {
        fs::write(
            root.0.join(format!("src/file_{index:03}.rs")),
            format!("pub fn f_{index}() {{ let needle = 5; }}\n"),
        )
        .unwrap();
    }
    let rev = commit(&root.0);
    let first = page(
        &root.0,
        &[
            "--rev",
            &rev,
            "--task",
            "needle",
            "--limit",
            "2",
            "--index-segments",
            "1",
        ],
    );
    assert_eq!(first["index"]["complete"], false);
    assert!(first["index"]["pending_segments"].as_u64().unwrap() > 0);
    assert!(
        first["coverage"]["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["reason"] == "SEGMENT_INDEX_INCOMPLETE")
    );
    if let Some(cursor) = first["page"]["next_cursor"].as_str() {
        let after = output(
            &root.0,
            &[
                "--rev",
                &rev,
                "--task",
                "needle",
                "--limit",
                "2",
                "--after",
                cursor,
                "--index-segments",
                "1",
            ],
        );
        assert_eq!(
            after.status.code(),
            Some(2),
            "a new completed partition must invalidate the old cursor"
        );
    }
}
