//! Real Git/SQLite integration for native JS resolution. Nothing is read from
//! uncommitted source files; only the pinned revision is authoritative.
use excession::index;
use excession::sqlite::Db;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pinned_typescript_config_and_package_main_resolve_without_worktree_guessing() {
    let work = std::env::temp_dir().join(format!(
        "excession-native-ts-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _cleanup = Cleanup(work.clone());
    let repo = work.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join("pkg/lib")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(
        repo.join("tsconfig.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    )
    .unwrap();
    fs::write(
        repo.join("pkg/package.json"),
        r#"{"main":"./lib/index.js"}"#,
    )
    .unwrap();
    fs::write(
        repo.join("src/start.ts"),
        "import { util } from '@/util'; import p from '../pkg'; import missing from './missing';\n",
    )
    .unwrap();
    fs::write(repo.join("src/util.ts"), "export const util = 1;\n").unwrap();
    fs::write(repo.join("pkg/lib/index.js"), "module.exports = 2;\n").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "pinned",
        ],
    );
    let db = Db::open(&work.join("index.sqlite")).unwrap();
    let index_once = || {
        index::index_revision_with_roots(&db, &repo, "native-js", "HEAD", "test", &["."]).unwrap();
    };
    index_once();
    let edges = db
        .query(
            "SELECT source_name,target_name FROM dependency ORDER BY source_name,target_name",
            &[],
        )
        .unwrap();
    assert!(
        edges
            .iter()
            .any(|r| r[0].as_deref() == Some("src.start.ts")
                && r[1].as_deref() == Some("src.util.ts")),
        "missing native tsconfig alias"
    );
    assert!(
        edges
            .iter()
            .any(|r| r[0].as_deref() == Some("src.start.ts") && r[1].as_deref() == Some("pkg.lib")),
        "missing native package main edge"
    );
    let before = db.scalar("SELECT COUNT(*) FROM dependency", &[]).unwrap();
    assert_ne!(
        db.scalar(
            "SELECT COUNT(*) FROM hazard WHERE reason LIKE 'unresolved source import:%'",
            &[]
        )
        .unwrap(),
        "0",
        "unknown local import must remain visible"
    );

    // Change only the working-tree configuration. Reindexing HEAD must
    // produce the same source-bound observations from committed Git blobs.
    fs::write(
        repo.join("tsconfig.json"),
        r#"{"compilerOptions":{"paths":{"@/*":["uncommitted/*"]}}}"#,
    )
    .unwrap();
    index_once();
    assert_eq!(
        db.scalar("SELECT COUNT(*) FROM dependency", &[]).unwrap(),
        before
    );
}
