//! Configurable source budgets must refresh the same Git tree correctly.
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn budget_file_and_cli_override_recover_a_previously_excluded_large_blob() {
    let root = std::env::temp_dir().join(format!(
        "excession-budget-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(&root).args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    };
    git(&["init", "-q"]);
    let body = format!("{}pub fn planner() {{}}\n", "// ordinary source line\n".repeat(17_000));
    assert!(body.len() > 256 * 1024);
    assert!(body.len() < 512 * 1024);
    fs::write(root.join("src/huge.rs"), body).unwrap();
    git(&["add", "."]);
    git(&["-c", "user.email=test@example.com", "-c", "user.name=Test", "commit", "-qm", "source"]);

    let policy = root.join("limits.json");
    fs::write(&policy, r#"{
        "max_file_bytes": 65536,
        "max_bytes_per_segment": 1048576,
        "max_files_per_segment": 1
    }"#).unwrap();
    let execute = |extra: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_excession"));
        cmd.args([
            "agent", "locate", "--repo", root.to_str().unwrap(),
            "--task", "planner", "--index-segments", "256",
            "--budget-file", policy.to_str().unwrap(), "--json",
        ]);
        cmd.args(extra);
        cmd.output().unwrap()
    };
    let low = execute(&[]);
    assert!(low.status.success(), "{}", String::from_utf8_lossy(&low.stderr));
    let low: serde_json::Value = serde_json::from_slice(&low.stdout).unwrap();
    assert_eq!(low["index"]["limits"]["max_file_bytes"], 65536);
    assert_eq!(low["skipped_source_count"], 1);
    assert_eq!(low["index"]["complete"], false);

    let high = execute(&["--max-file-bytes", "524288"]);
    assert!(high.status.success(), "{}", String::from_utf8_lossy(&high.stderr));
    let high: serde_json::Value = serde_json::from_slice(&high.stdout).unwrap();
    assert_eq!(high["index"]["limits"]["max_file_bytes"], 524288);
    assert_eq!(high["skipped_source_count"], 0);
    assert_eq!(high["index"]["complete"], true);
    assert_eq!(high["candidates"][0]["source"]["path"], "src/huge.rs");

    let low_again = execute(&[]);
    assert!(low_again.status.success());
    let low_again: serde_json::Value = serde_json::from_slice(&low_again.stdout).unwrap();
    assert_eq!(low_again["skipped_source_count"], 1);
    assert_eq!(low_again["index"]["complete"], false);

    fs::write(&policy, r#"{"max_file_bytes": 0}"#).unwrap();
    let invalid = execute(&[]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("budgets must be positive"));

    fs::write(&policy, r#"{"unknown_budget": 12}"#).unwrap();
    let unknown = execute(&[]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("budget-file is invalid"));

    let cache = high["cache"]["path"].as_str().unwrap();
    if let Some(parent) = std::path::Path::new(cache).parent() {
        fs::remove_dir_all(parent).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}
