//! User-facing task-to-source lookup, against a real committed Git fixture.
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn agent_locate_cli_returns_exact_git_sources_and_excludes_dirty_edits() {
    let root = std::env::temp_dir().join(format!(
        "excession-cli-locate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("src")).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(
        root.join("src/lease.rs"),
        "pub fn renew_lease() { recover_worker(); }\n",
    )
    .unwrap();
    fs::write(root.join("src/other.rs"), "pub fn render_canvas() {}\n").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=test@example.com",
        "-c",
        "user.name=Test",
        "commit",
        "-qm",
        "snapshot",
    ]);
    let execute = || {
        Command::new(env!("CARGO_BIN_EXE_excession"))
            .args([
                "agent",
                "locate",
                "--repo",
                root.to_str().unwrap(),
                "--task",
                "renew lease",
                "--rev",
                "HEAD",
                "--limit",
                "3",
                "--json",
            ])
            .output()
            .unwrap()
    };
    let first = execute();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let before: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(before["schema"], "excession.agent-locate.v1");
    assert_eq!(before["candidates"][0]["source"]["path"], "src/lease.rs");
    assert_eq!(
        before["candidates"][0]["semantic_correctness"],
        "UNVERIFIED"
    );
    assert_eq!(before["context"]["schema"], "excession.agent-context.v1");
    assert_eq!(before["safe_to_skip_unlisted_tests"], false);
    assert_eq!(before["source_commit"].as_str().unwrap().len(), 40);
    assert_eq!(before["source_tree"].as_str().unwrap().len(), 40);

    fs::write(root.join("src/lease.rs"), "pub fn unrelated() {}\n").unwrap();
    fs::write(root.join("src/other.rs"), "pub fn renew_lease() {}\n").unwrap();
    let second = execute();
    assert!(second.status.success());
    let after: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(before["report_id"], after["report_id"]);
    assert_eq!(before["candidates"], after["candidates"]);

    let invalid = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args([
            "agent",
            "locate",
            "--repo",
            root.to_str().unwrap(),
            "--task",
            "renew lease",
            "--limit",
            "0",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    fs::remove_dir_all(root).unwrap();
}
