//! Executable CLI acceptance for source-bound history planning and replay.
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn git(repo: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn history_cli_replays_unchanged_git_dag_and_rejects_altered_report() {
    let repo = std::env::temp_dir().join(format!(
        "excession-history-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    for (name, msg) in [
        ("a", "initial"),
        ("b", "feat(core): add"),
        ("c", "feat(core): extend"),
    ] {
        fs::write(repo.join(name), name).unwrap();
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-qm",
                msg,
            ],
        );
    }
    let path = repo.to_string_lossy().to_string();
    let output = cli(&["history", "plan", "--repo", &path, "--rev", "HEAD"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "excession.history-plan.v1");
    assert_eq!(report["commit_count"], 3);
    assert_eq!(report["candidate_groups"].as_array().unwrap().len(), 1);
    assert_eq!(report["safety"]["rewrite_authorized"], false);
    let saved = repo.join("plan.json");
    fs::write(&saved, &output.stdout).unwrap();
    let audit = cli(&[
        "history",
        "audit",
        "--repo",
        &path,
        "--plan",
        saved.to_str().unwrap(),
    ]);
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let proof: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(proof["status"], "SOURCE_DAG_REPLAY_VERIFIED");

    fs::write(repo.join("a"), "dirty edit ignored by the history parser").unwrap();
    assert_eq!(
        output.stdout,
        cli(&["history", "plan", "--repo", &path]).stdout
    );

    let bounded = cli(&["history", "plan", "--repo", &path, "--max-commits", "1"]);
    assert!(
        bounded.status.success(),
        "{}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    let partial: serde_json::Value = serde_json::from_slice(&bounded.stdout).unwrap();
    assert_eq!(partial["coverage"]["truncated"], true);
    assert_eq!(partial["commits"][0]["commit"], report["original_tip"]);

    let mut forged = report;
    forged["commit_count"] = serde_json::json!(22);
    fs::write(&saved, forged.to_string()).unwrap();
    assert!(
        !cli(&[
            "history",
            "audit",
            "--repo",
            &path,
            "--plan",
            saved.to_str().unwrap()
        ])
        .status
        .success()
    );
    assert!(
        !cli(&["history", "plan", "--repo", &path, "--max-commits", "0"])
            .status
            .success()
    );
    fs::remove_dir_all(repo).unwrap();
}
