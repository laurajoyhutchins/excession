//! Independent, authored expected relationship set on real Git objects.
//! The expected edges do not come from Excession's SQL view or impact engine.
use excession::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(cwd: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("launch test command");
    assert!(
        output.status.success(),
        "{} {:?}: {}",
        program,
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn commit(repo: &Path, message: &str) -> String {
    run(repo, "git", &["add", "-A"]);
    run(
        repo,
        "git",
        &[
            "-c",
            "user.name=Acceptance",
            "-c",
            "user.email=acceptance@example.invalid",
            "commit",
            "-qm",
            message,
        ],
    );
    run(repo, "git", &["rev-parse", "HEAD"])
}

#[test]
fn independent_git_oracle_checks_exact_import_edges_and_changed_dependents() {
    let root = std::env::temp_dir().join(format!(
        "excession-product-acceptance-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _fixture = Fixture(root.clone());
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("pkg")).unwrap();
    run(&repo, "git", &["init", "-q"]);
    fs::write(repo.join("pkg/__init__.py"), "").unwrap();
    fs::write(repo.join("pkg/a.py"), "import pkg.b\n").unwrap();
    fs::write(repo.join("pkg/b.py"), "VALUE = 1\n").unwrap();
    fs::write(repo.join("pkg/c.py"), "import pkg.b\n").unwrap();
    fs::write(repo.join("pkg/d.py"), "VALUE = 'unrelated'\n").unwrap();
    let base = commit(&repo, "base");
    let tree = run(&repo, "git", &["rev-parse", "HEAD^{tree}"]);
    let expected: BTreeSet<_> = [
        ("pkg/a.py".to_owned(), "pkg/b.py".to_owned()),
        ("pkg/c.py".to_owned(), "pkg/b.py".to_owned()),
    ]
    .into_iter()
    .collect();
    let binary = env!("CARGO_BIN_EXE_excession");
    let cache = root.join("cache.sqlite");
    let analysis = run(
        &repo,
        binary,
        &["analyze", ".", "--db", cache.to_str().unwrap(), "--json"],
    );
    let observed = json::parse(analysis.as_bytes()).unwrap();
    assert_eq!(observed.get("source_commit").unwrap().str().unwrap(), base);
    assert_eq!(observed.get("source_tree").unwrap().str().unwrap(), tree);
    let mut actual = BTreeSet::new();
    for relationship in observed.get("relationships").unwrap().array_ref().unwrap() {
        let a = relationship.get("source").unwrap();
        let b = relationship.get("target").unwrap();
        assert_eq!(a.get("source_commit").unwrap().str().unwrap(), base);
        assert_eq!(b.get("source_commit").unwrap().str().unwrap(), base);
        let path = a.get("path").unwrap().str().unwrap();
        let target = b.get("path").unwrap().str().unwrap();
        assert_eq!(
            a.get("blob").unwrap().str().unwrap(),
            run(&repo, "git", &["rev-parse", &format!("{base}:{path}")])
        );
        assert_eq!(
            b.get("blob").unwrap().str().unwrap(),
            run(&repo, "git", &["rev-parse", &format!("{base}:{target}")])
        );
        assert!(
            relationship
                .get("evidence")
                .unwrap()
                .array_ref()
                .unwrap()
                .iter()
                .all(
                    |observation| observation.get("assessment").unwrap().str().unwrap()
                        == "CANDIDATE"
                )
        );
        actual.insert((path.to_owned(), target.to_owned()));
    }
    assert_eq!(
        actual, expected,
        "the independently authored relationship set must match exactly"
    );
    fs::write(repo.join("pkg/b.py"), "VALUE = 2\n").unwrap();
    let head = commit(&repo, "modify imported module");
    let impact = run(
        &repo,
        binary,
        &[
            "impact",
            ".",
            "--base",
            &base,
            "--head",
            &head,
            "--db",
            cache.to_str().unwrap(),
            "--json",
        ],
    );
    let reported = json::parse(impact.as_bytes()).unwrap();
    let affected: BTreeSet<_> = reported
        .get("effects")
        .unwrap()
        .array_ref()
        .unwrap()
        .iter()
        .map(|e| {
            assert_eq!(
                e.get("assessment").unwrap().str().unwrap(),
                "CANDIDATE_PATH"
            );
            e.get("entity")
                .unwrap()
                .get("path")
                .unwrap()
                .str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        affected,
        ["pkg/a.py".to_owned(), "pkg/c.py".to_owned()]
            .into_iter()
            .collect()
    );
    assert!(
        !reported
            .get("safe_to_skip_unlisted_tests")
            .unwrap()
            .bool()
            .unwrap()
    );
}
