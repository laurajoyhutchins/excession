//! Independent real-Git two-revision change impact acceptance.
use excession::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn reports_importer_as_candidate_without_authorizing_test_skips() {
    let root: PathBuf = std::env::temp_dir().join(format!(
        "excession-real-impact-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("pkg")).unwrap();
    let cmd = |program: &str, args: &[&str]| -> String {
        let output = Command::new(program)
            .current_dir(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {:?}: {}",
            program,
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    cmd("git", &["init", "-q"]);
    fs::write(repo.join("pkg/__init__.py"), "").unwrap();
    fs::write(repo.join("pkg/first.py"), "import pkg.second\n").unwrap();
    fs::write(repo.join("pkg/second.py"), "VALUE = 1\n").unwrap();
    cmd("git", &["add", "."]);
    cmd(
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    );
    let base = cmd("git", &["rev-parse", "HEAD"]);
    fs::write(repo.join("pkg/second.py"), "VALUE = 2\n").unwrap();
    cmd("git", &["add", "-A"]);
    cmd(
        "git",
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "-m",
            "head",
        ],
    );
    let head = cmd("git", &["rev-parse", "HEAD"]);
    let database = root.join("observations.sqlite");
    let bin = env!("CARGO_BIN_EXE_excession");
    let output = cmd(
        bin,
        &[
            "impact",
            ".",
            "--base",
            &base,
            "--head",
            &head,
            "--db",
            database.to_str().unwrap(),
            "--json",
        ],
    );
    let result = json::parse(output.as_bytes()).unwrap();
    assert_eq!(
        result.get("schema").unwrap().str().unwrap(),
        "excession.impact.v1"
    );
    assert_eq!(result.get("base_commit").unwrap().str().unwrap(), base);
    assert_eq!(result.get("head_commit").unwrap().str().unwrap(), head);
    assert!(
        !result
            .get("safe_to_skip_unlisted_tests")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert_eq!(
        result
            .get("coverage")
            .unwrap()
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "PARTIAL"
    );
    let effects = result.get("effects").unwrap().array_ref().unwrap();
    assert!(
        effects
            .iter()
            .all(|effect| { effect.get("claim_id").unwrap().str().unwrap().len() == 64 })
    );
    assert!(
        effects.iter().any(|effect| {
            effect
                .get("entity")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                == "pkg/first.py"
                && effect.get("assessment").unwrap().str().unwrap() == "CANDIDATE_PATH"
        }),
        "expected an importer candidate in the independently constructed Git fixture"
    );
    assert!(effects.iter().all(|effect| {
        effect
            .get("hops")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .all(|hop| {
                hop.get("evidence")
                    .unwrap()
                    .array_ref()
                    .unwrap()
                    .iter()
                    .all(|e| e.get("assessment").unwrap().str().unwrap() == "CANDIDATE")
            })
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn revisions_without_supported_sources_are_explicitly_out_of_scope() {
    let root = std::env::temp_dir().join(format!(
        "excession-impact-no-source-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| -> String {
        let output = Command::new("git")
            .current_dir(&repo).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q"]);
    fs::write(repo.join("README.md"), "first\n").unwrap();
    git(&["add", "README.md"]);
    git(&["-c", "user.name=Fixture", "-c",
          "user.email=fixture@example.invalid", "commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    fs::write(repo.join("README.md"), "second\n").unwrap();
    git(&["add", "README.md"]);
    git(&["-c", "user.name=Fixture", "-c",
          "user.email=fixture@example.invalid", "commit", "-qm", "head"]);
    let bin = env!("CARGO_BIN_EXE_excession");
    // Ordinary analysis remains an actionable error, never fake success.
    let analyze = Command::new(bin).current_dir(&repo)
        .args(["analyze", ".", "--json"]).output().unwrap();
    assert!(!analyze.status.success());
    let report = Command::new(bin).current_dir(&repo)
        .args(["impact", ".", "--base", &base, "--json"]).output().unwrap();
    assert!(report.status.success(), "{}", String::from_utf8_lossy(&report.stderr));
    let result = json::parse(&report.stdout).unwrap();
    assert!(result.get("effects").unwrap().array_ref().unwrap().is_empty());
    let reasons: Vec<_> = result.get("unknown").unwrap().array_ref().unwrap()
        .iter().map(|gap| gap.get("reason").unwrap().str().unwrap()).collect();
    assert!(reasons.contains(&"EXCLUDED_SCOPE"));
    assert!(reasons.contains(&"UNSUPPORTED_LANGUAGE"));
    assert!(!reasons.contains(&"MISSING_ADAPTER"));
    assert!(!result.get("safe_to_skip_unlisted_tests").unwrap().bool().unwrap());
    fs::remove_dir_all(root).unwrap();
}
