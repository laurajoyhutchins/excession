//! Independent real-Git acceptance of exact source before/after inventory.
use excession::json;
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
fn run(cwd: &Path, exe: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(exe)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {:?}: {}",
        exe,
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn git(cwd: &Path, args: &[&str]) -> String {
    String::from_utf8(run(cwd, "git", args))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn inventory_follows_two_real_commits_not_worktree_changes() {
    let tmp = std::env::temp_dir().join(format!(
        "excession-change-inventory-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _fixture = Fixture(tmp.clone());
    let root = tmp.join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("alpha.py"), "A = 1\n").unwrap();
    fs::write(root.join("gone.py"), "B = 1\n").unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
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
    let base = git(&root, &["rev-parse", "HEAD"]);
    let old_oid = git(&root, &["rev-parse", "HEAD:alpha.py"]);
    fs::write(root.join("alpha.py"), "A = 2\n").unwrap();
    fs::remove_file(root.join("gone.py")).unwrap();
    fs::write(root.join("added.py"), "C = 1\n").unwrap();
    git(&root, &["add", "-A"]);
    git(
        &root,
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
    let head = git(&root, &["rev-parse", "HEAD"]);
    let new_oid = git(&root, &["rev-parse", "HEAD:alpha.py"]);
    // The uncommitted source must not affect an immutable Git diff.
    fs::write(root.join("alpha.py"), "UNCOMMITTED = True\n").unwrap();
    let reply = json::parse(&run(
        &root,
        env!("CARGO_BIN_EXE_excession"),
        &["changes", ".", "--base", &base, "--head", &head],
    ))
    .unwrap();
    assert_eq!(
        reply.get("schema").unwrap().str().unwrap(),
        "excession.change-inventory.v1"
    );
    assert_eq!(reply.get("base_commit").unwrap().str().unwrap(), base);
    assert_eq!(reply.get("head_commit").unwrap().str().unwrap(), head);
    let changes = reply.get("changes").unwrap().array_ref().unwrap();
    assert_eq!(changes.len(), 3);
    let kinds: Vec<_> = changes
        .iter()
        .map(|c| c.get("kind").unwrap().str().unwrap())
        .collect();
    assert_eq!(kinds, ["ADDED", "MODIFIED", "DELETED"]);
    let modified = &changes[1];
    assert_eq!(
        modified
            .get("before")
            .unwrap()
            .get("blob")
            .unwrap()
            .str()
            .unwrap(),
        old_oid
    );
    assert_eq!(
        modified
            .get("after")
            .unwrap()
            .get("blob")
            .unwrap()
            .str()
            .unwrap(),
        new_oid
    );
    assert_ne!(old_oid, new_oid);
    assert_eq!(changes[0].get("before").unwrap(), &json::Value::Null);
    assert_eq!(changes[2].get("after").unwrap(), &json::Value::Null);
}
