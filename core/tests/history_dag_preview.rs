//! End-to-end DAG condensation against real multi-branch Git fixtures.
//! No tests rewrite refs in the original source checkout.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn fixture(side_dependency: bool) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "excession-dag-preview-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = root.join("source");
    let output = root.join("review");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Git DAG fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    let commit = |path: &str, value: &str, message: &str| {
        fs::write(repo.join(path), value).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", message]);
    };
    commit("initial.txt", "root", "initial source");
    commit("one.txt", "one", "feat(core): first independent change");
    if side_dependency {
        git(&repo, &["branch", "side", "HEAD"]);
    }
    commit("two.txt", "two", "feat(core): second independent change");
    let terminal = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["tag", "review-anchor", &terminal]);
    commit("three.txt", "three", "fix(other): following change");
    git(&repo, &["checkout", "-q", "-b", "feature", &terminal]);
    commit("feature.txt", "feature", "feat(other): branch change");
    git(&repo, &["checkout", "-q", "main"]);
    git(
        &repo,
        &["merge", "--no-ff", "-qm", "merge feature", "feature"],
    );
    if side_dependency {
        git(&repo, &["checkout", "-q", "side"]);
        commit(
            "side.txt",
            "side",
            "fix(other): another child of first change",
        );
        git(&repo, &["checkout", "-q", "main"]);
    }
    (repo, output)
}

fn cli(repo: &Path, output: &Path, extras: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_excession"));
    command
        .args(["history", "preview-dag", "--repo"])
        .arg(repo)
        .args(["--output"])
        .arg(output)
        .args(extras)
        .output()
        .unwrap()
}

#[test]
fn branch_merge_and_tag_are_tree_identical_and_original_refs_stay_untouched() {
    let (repo, output) = fixture(false);
    let before = git(
        &repo,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname) %(objectname)",
        ],
    );
    let head = git(&repo, &["rev-parse", "HEAD"]);
    // A dirty checkout is allowed because the algorithm reads Git objects only.
    fs::write(
        repo.join("untracked-local-file"),
        "keep uncommitted changes",
    )
    .unwrap();
    let result = cli(&repo, &output, &["--group", "0", "--max-commits", "100"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(summary["status"], "ISOLATED_DAG_REWRITE_TREE_VERIFIED");
    assert_eq!(summary["folded_commits"], 2);
    assert_eq!(summary["exact_final_source_tree"], true);
    assert_eq!(summary["all_local_ref_terminal_trees_preserved"], true);
    assert_eq!(summary["original_refs_unchanged"], true);
    assert_eq!(summary["rewrite_authorized"], false);
    assert!(summary["synthetic_commit_count"].as_u64().unwrap() >= 3);

    let mapping: Value =
        serde_json::from_slice(&fs::read(output.join("mapping.json")).unwrap()).unwrap();
    let rows = mapping["mapping"].as_array().unwrap();
    assert_eq!(
        rows.len() as u64,
        summary["mapped_commit_count"].as_u64().unwrap()
    );
    let collapsed = rows
        .iter()
        .filter(|row| row["intermediate_state_dropped"] == true)
        .count();
    assert_eq!(collapsed, 1);
    let candidate_head = summary["candidate_head"].as_str().unwrap();
    let source_tree = git(&repo, &["rev-parse", "HEAD^{tree}"]);
    let clone = output.join("candidate.git");
    assert_eq!(
        git(
            &clone,
            &["rev-parse", &format!("{candidate_head}^{{tree}}")]
        ),
        source_tree
    );
    assert_eq!(
        git(&clone, &["rev-parse", "refs/heads/excession-dag-preview"]),
        candidate_head
    );
    let proposals: Value =
        serde_json::from_slice(&fs::read(output.join("refs.json")).unwrap()).unwrap();
    assert_eq!(proposals["original_refs_preserved"], true);
    assert_eq!(proposals["external_refs_complete"], false);
    let obligations: Value =
        serde_json::from_slice(&fs::read(output.join("obligations.json")).unwrap()).unwrap();
    assert_eq!(obligations["rewrite_authorized"], false);
    assert_eq!(
        obligations["rea_evidence"],
        "REQUIRES_INDEPENDENT_REBINDING"
    );

    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(
            &repo,
            &[
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname) %(objectname)"
            ]
        ),
        before
    );
    assert_eq!(
        fs::read(repo.join("untracked-local-file")).unwrap(),
        b"keep uncommitted changes"
    );
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}

#[test]
fn cannot_collapse_a_commit_referenced_by_another_branch() {
    let (repo, output) = fixture(true);
    let before = git(
        &repo,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname) %(objectname)",
        ],
    );
    let result = cli(&repo, &output, &["--group", "0"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("dependency"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!output.exists());
    assert_eq!(
        git(
            &repo,
            &[
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname) %(objectname)"
            ]
        ),
        before
    );
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}

#[test]
fn refuses_truncated_and_unexpected_output_locations() {
    let (repo, output) = fixture(false);
    let limited = cli(&repo, &output, &["--max-commits", "2"]);
    assert!(!limited.status.success());
    assert!(!output.exists());
    let inside = repo.join("candidate");
    let unsafe_destination = cli(&repo, &inside, &["--group", "0"]);
    assert!(!unsafe_destination.status.success());
    assert!(!inside.exists());
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}
