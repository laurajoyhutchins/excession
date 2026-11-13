//! Authentic Git objects and a two-parent join, not a mocked history graph.
use excession::git_reconstruction_synthesis;
use excession::json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    seed: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-synth-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("source");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q", "--object-format=sha1"]);
        fs::write(repo.join("src/common.rs"), "pub const VALUE:i32=3;\n").unwrap();
        fs::write(repo.join("src/reader.rs"), "use crate::common::VALUE;\n").unwrap();
        fs::write(repo.join("src/writer.rs"), "use crate::common::VALUE;\n").unwrap();
        fs::write(repo.join("README.md"), "original source snapshot\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "original source"]);
        let commit = git(&repo, &["rev-parse", "HEAD"]);
        let seed = root.join("seed.json");
        fs::write(&seed,serde_json::json!({
            "schema":"excession.git-reconstruction-seed.v1",
            "target_commit":commit,
            "source_dependencies":[
                {"path":"src/reader.rs","requires":["src/common.rs"]},
                {"path":"src/writer.rs","requires":["src/common.rs"]}
            ],
            "contracts":[
                {"id":"reader","requires":[],"source_closure":["src/common.rs","src/reader.rs"]},
                {"id":"writer","requires":[],"source_closure":["src/common.rs","src/writer.rs"]}
            ]
        }).to_string()).unwrap();
        Self { root, repo, seed }
    }
    fn run(&self, name: &str, count: usize, feedback: Option<&Path>) -> Result<Value, String> {
        git_reconstruction_synthesis::synthesize(
            &self.repo,
            &self.seed,
            &self.root.join(name),
            count,
            feedback,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn two_independent_contracts_yield_real_merge_and_exact_final_tree() {
    let f = Fixture::new();
    let result = f.run("result", 2, None).unwrap();
    assert_eq!(result.get("candidate_count").unwrap().integer().unwrap(), 2);
    assert_eq!(
        result.get("source_identity").unwrap().str().unwrap(),
        "EXACT_TARGET_BLOBS_AND_MODES"
    );
    assert_eq!(
        result.get("behavioral_correctness").unwrap().str().unwrap(),
        "NOT_ESTABLISHED"
    );
    let out = f.root.join("result");
    let target = git(&f.repo, &["rev-parse", "HEAD^{tree}"]);
    let plan: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("plan.json")).unwrap()).unwrap();
    assert_eq!(plan["histories"].as_array().unwrap().len(), 2);
    let integration = &plan["integration_nodes"][0];
    let merged = integration["commit"].as_str().unwrap();
    let parents = git(
        &out.join("repository"),
        &["rev-list", "--parents", "-n", "1", merged],
    );
    assert_eq!(
        parents,
        format!(
            "{merged} {} {}",
            integration["parents"][0].as_str().unwrap(),
            integration["parents"][1].as_str().unwrap()
        )
    );
    for history in plan["histories"].as_array().unwrap() {
        let steps = history["checkpoints"].as_array().unwrap();
        assert_eq!(steps.len(), 4);
        let root = steps[0]["commit"].as_str().unwrap();
        assert_eq!(
            git(
                &out.join("repository"),
                &["rev-list", "--parents", "-n", "1", root]
            ),
            root
        );
        let final_commit = steps[3]["commit"].as_str().unwrap();
        assert_eq!(
            git(
                &out.join("repository"),
                &["rev-parse", &format!("{final_commit}^{{tree}}")]
            ),
            target
        );
    }
    assert_eq!(
        result
            .get("oracle_selections")
            .unwrap()
            .array_ref()
            .unwrap()
            .iter()
            .filter(|r| r.get("phase").unwrap().str().unwrap() == "INTEGRATION")
            .count(),
        1
    );
    // Source stays unchanged: all generated history is outside the original checkout.
    assert_eq!(git(&f.repo, &["status", "--porcelain"]), "");
    assert!(f.run("result", 2, None).is_err());
}
#[test]
fn counterexamples_never_promote_reordering_to_a_behavioral_fix() {
    let f = Fixture::new();
    let report = f.run("first", 2, None).unwrap();
    let first = report
        .get("oracle_selections")
        .unwrap()
        .array_ref()
        .unwrap()
        .iter()
        .find(|r| r.get("phase").unwrap().str().unwrap() == "INTRODUCTION")
        .unwrap();
    let feedback = f.root.join("feedback.json");
    fs::write(&feedback,serde_json::json!({"schema":"excession.git-reconstruction-counterexamples.v1",
        "failures":[{"contract":first.get("contract").unwrap().str().unwrap(),"tree":first.get("tree").unwrap().str().unwrap(),"oracle_sha256":"a".repeat(64)}]
    }).to_string()).unwrap();
    let second = f.run("second", 2, Some(&feedback)).unwrap();
    assert!(
        second
            .get("counterexample_matches")
            .unwrap()
            .integer()
            .unwrap()
            > 0
    );
    assert_eq!(
        second.get("feedback_authority").unwrap().str().unwrap(),
        "CALLER_SUPPLIED_UNATTESTED"
    );
    assert_eq!(
        second.get("behavioral_correctness").unwrap().str().unwrap(),
        "NOT_ESTABLISHED"
    );
    let target = git(&f.repo, &["rev-parse", "HEAD^{tree}"]);
    fs::write(
        &feedback,
        serde_json::json!({"schema":"excession.git-reconstruction-counterexamples.v1",
            "failures":[{"contract":"reader","tree":target,"oracle_sha256":"a".repeat(64)}]
        })
        .to_string(),
    )
    .unwrap();
    let final_feedback = f.run("third", 2, Some(&feedback)).unwrap();
    assert_eq!(
        final_feedback
            .get("target_behavior")
            .unwrap()
            .str()
            .unwrap(),
        "COUNTEREXAMPLE_BLOCKS_ALL_SOURCE_IDENTICAL_HISTORIES"
    );
}
#[test]
fn cyclic_contracts_missing_dependencies_and_unsafe_output_are_refused() {
    let f = Fixture::new();
    let mut seed: serde_json::Value = serde_json::from_slice(&fs::read(&f.seed).unwrap()).unwrap();
    seed["contracts"][0]["requires"] = serde_json::json!(["writer"]);
    seed["contracts"][1]["requires"] = serde_json::json!(["reader"]);
    fs::write(&f.seed, seed.to_string()).unwrap();
    assert!(f.run("cyclic", 2, None).is_err());
    assert!(!f.root.join("cyclic").exists());
    seed["contracts"][0]["requires"] = serde_json::json!([]);
    seed["contracts"][1]["requires"] = serde_json::json!([]);
    seed["contracts"][0]["source_closure"] = serde_json::json!(["src/reader.rs"]);
    fs::write(&f.seed, seed.to_string()).unwrap();
    assert!(f.run("missing", 2, None).is_err());
    let path = f.repo.join("forbidden");
    assert!(git_reconstruction_synthesis::synthesize(&f.repo, &f.seed, &path, 2, None).is_err());
}
