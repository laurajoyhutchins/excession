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
    assert_eq!(
        result
            .get("safe_to_skip_unlisted_tests")
            .unwrap()
            .bool()
            .unwrap(),
        false
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
