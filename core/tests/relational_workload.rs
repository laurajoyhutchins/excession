//! End-to-end native relational benchmark acceptance, including tamper refusal.
#[cfg(unix)]
#[test]
fn rust_relational_runner_produces_auditable_receipts() {
    use std::fs;
    use std::io::Write;
    use std::process::Command;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    let root = Scratch(std::env::temp_dir().join(format!(
        "excession-rust-bench-smoke-{}", std::process::id()
    )));
    fs::create_dir(&root.0).unwrap();
    let output = root.0.join("result");
    let binary = env!("CARGO_BIN_EXE_excession");
    let status = Command::new(binary)
        .args(["benchmark", "run-relational", "--out"])
        .arg(&output)
        .args(["--sizes", "8", "--fanouts", "1", "--revisions", "1", "--repetitions", "1"])
        .output().unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    let summary: serde_json::Value = serde_json::from_slice(
        &fs::read(output.join("summary.json")).unwrap()
    ).unwrap();
    assert_eq!(summary["query_count_per_arm"], 12);
    assert_eq!(summary["status_counts"]["graph"]["ok"], 12);
    assert_eq!(summary["status_counts"]["view"]["ok"], 12);
    assert!(summary["workloads"][0]["revisions"][0]["independently_specified_edges_equal"] == true);
    assert!(output.join("modules8_fanout1.bundle").is_file());
    assert!(output.join("trials.jsonl").is_file());

    let report = output.join("report.json");
    let verified = Command::new(binary).args(["benchmark", "validate", "--report"])
        .arg(&report).output().unwrap();
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));

    // Source-bound measurements are not valid after raw observations change.
    let mut raw = fs::OpenOptions::new().append(true)
        .open(output.join("trials.jsonl")).unwrap();
    raw.write_all(b"{}\n").unwrap();
    let tampered = Command::new(binary).args(["benchmark", "validate", "--report"])
        .arg(&report).output().unwrap();
    assert!(!tampered.status.success());
}
