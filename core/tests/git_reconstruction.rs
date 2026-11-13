//! Source-bound reconstruction against real rootless Git object histories.
use excession::git_reconstruction;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Reconstruction fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Reconstruction fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    root: PathBuf,
    plan: PathBuf,
    json: serde_json::Value,
    first: Vec<String>,
    second: Vec<String>,
    merged: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-git-rebuild-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "--object-format=sha1"]);
        for (name, contents) in [
            ("common.rs", "pub fn core() {}\n"),
            ("reader.rs", "pub fn read() {}\n"),
            ("writer.rs", "pub fn write() {}\n"),
            ("interface.rs", "pub fn integrated() {}\n"),
        ] {
            fs::write(root.join(name), contents).unwrap();
        }
        fn history(root: &Path, name: &str, middle: [&str; 2]) -> Vec<String> {
            git(root, &["read-tree", "--empty"]);
            let mut ids = Vec::new();
            let mut previous: Option<String> = None;
            for path in ["common.rs", middle[0], middle[1], "interface.rs"] {
                git(root, &["add", "--", path]);
                let tree = git(root, &["write-tree"]);
                let msg = format!("SYNTHETIC {name}: {path}");
                let mut args = vec!["commit-tree", tree.as_str(), "-m", msg.as_str()];
                if let Some(parent) = &previous {
                    args.extend(["-p", parent]);
                }
                let commit = git(root, &args);
                previous = Some(commit.clone());
                ids.push(commit);
            }
            ids
        }
        let first = history(&root, "reader-first", ["reader.rs", "writer.rs"]);
        let second = history(&root, "writer-first", ["writer.rs", "reader.rs"]);
        assert_eq!(
            git(&root, &["rev-parse", &format!("{}^{{tree}}", first[3])]),
            git(&root, &["rev-parse", &format!("{}^{{tree}}", second[3])])
        );
        assert_ne!(first[0], second[0]);
        assert_ne!(first[1], second[1]);
        // A real two-parent merge: each parent independently carries one
        // complete source-bound codec contract, and the join carries both.
        fn branch(root: &Path, base: &str, path: &str) -> String {
            git(root, &["read-tree", base]);
            git(root, &["add", "--", path]);
            let tree = git(root, &["write-tree"]);
            git(
                root,
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    base,
                    "-m",
                    "SYNTHETIC codec branch",
                ],
            )
        }
        let left = branch(&root, &first[0], "reader.rs");
        let right = branch(&root, &first[0], "writer.rs");
        git(&root, &["read-tree", &left]);
        git(&root, &["add", "--", "writer.rs"]);
        let union_tree = git(&root, &["write-tree"]);
        let merged = git(
            &root,
            &[
                "commit-tree",
                &union_tree,
                "-p",
                &left,
                "-p",
                &right,
                "-m",
                "SYNTHETIC two-parent codec merge",
            ],
        );
        git(&root, &["read-tree", &merged]);
        git(&root, &["add", "--", "interface.rs"]);
        let integrated_tree = git(&root, &["write-tree"]);
        let integrated = git(
            &root,
            &[
                "commit-tree",
                &integrated_tree,
                "-p",
                &merged,
                "-m",
                "SYNTHETIC integration",
            ],
        );
        assert_eq!(
            git(&root, &["rev-parse", &format!("{}^{{tree}}", first[3])]),
            integrated_tree
        );
        let json = serde_json::json!({
            "schema":"excession.git-reconstruction-plan.v1",
            "target_commit": first[3],
            "source_dependencies":[
                {"path":"reader.rs","requires":["common.rs"]},
                {"path":"writer.rs","requires":["common.rs"]},
                {"path":"interface.rs","requires":["reader.rs","writer.rs"]}
            ],
            "contracts":[
                {"id":"read","source_closure":["common.rs","reader.rs"],"requires":[]},
                {"id":"write","source_closure":["common.rs","writer.rs"],"requires":[]},
                {"id":"integrate","source_closure":["common.rs","reader.rs","writer.rs","interface.rs"],"requires":["read","write"]}
            ],
            "histories":[
                {"name":"reader-first","checkpoints":[
                    {"commit":first[0],"introduces":[]},
                    {"commit":first[1],"introduces":["read"]},
                    {"commit":first[2],"introduces":["write"]},
                    {"commit":first[3],"introduces":["integrate"]}
                ]},
                {"name":"writer-first","checkpoints":[
                    {"commit":second[0],"introduces":[]},
                    {"commit":second[1],"introduces":["write"]},
                    {"commit":second[2],"introduces":["read"]},
                    {"commit":second[3],"introduces":["integrate"]}
                ]},
                {"name":"true-merge","checkpoints":[
                    {"commit":first[0],"introduces":[]},
                    {"commit":merged,"introduces":["read","write"]},
                    {"commit":integrated,"introduces":["integrate"]}
                ]}
            ],
            "integration_nodes":[
                {"commit":merged,"parents":[left,right],"preserves":["read","write"]}
            ]
        });
        let plan = root.join("plan.json");
        Self {
            root,
            plan,
            json,
            first,
            second,
            merged,
        }
    }
    fn verify(&self) -> excession::Result<excession::json::Value> {
        fs::write(&self.plan, self.json.to_string()).unwrap();
        git_reconstruction::verify(&self.root, &self.plan)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn two_rootless_histories_replay_identical_target_and_retain_source_contracts() {
    let fixture = Fixture::new();
    let report = fixture.verify().unwrap();
    assert_eq!(
        report.get("observed").unwrap().str().unwrap(),
        "EXACT_GIT_SOURCE_IDENTITY"
    );
    assert_eq!(
        report
            .get("independent_contract_oracles")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert_eq!(
        report.get("build_provenance").unwrap().str().unwrap(),
        "NOT_ESTABLISHED"
    );
    assert_eq!(
        report.get("rea_evidence").unwrap().str().unwrap(),
        "NOT_ESTABLISHED"
    );
    let histories = report.get("histories").unwrap().array_ref().unwrap();
    assert_eq!(histories.len(), 3);
    assert_eq!(
        report
            .get("integration_nodes")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        histories[0].get("final_tree").unwrap(),
        histories[1].get("final_tree").unwrap()
    );
    assert_eq!(
        histories[1].get("final_tree").unwrap(),
        histories[2].get("final_tree").unwrap()
    );
    assert_eq!(
        histories[0]
            .get("checkpoints")
            .unwrap()
            .array_ref()
            .unwrap()
            .len(),
        4
    );
    let report2 = fixture.verify().unwrap();
    assert_eq!(report.canonical(), report2.canonical());
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/git-reconstruction-plan.v1.schema.json"
    ))
    .unwrap();
    assert!(
        jsonschema::draft202012::new(&schema)
            .unwrap()
            .is_valid(&fixture.json)
    );
}

#[test]
fn fail_closed_on_missing_dependency_early_contract_and_missing_final_tree() {
    let mut fixture = Fixture::new();
    fixture.json["source_dependencies"][0]["requires"] = serde_json::json!(["writer.rs"]);
    assert!(fixture.verify().is_err());
    fixture.json["source_dependencies"][0]["requires"] = serde_json::json!(["common.rs"]);
    fixture.json["histories"][0]["checkpoints"][0]["introduces"] = serde_json::json!(["read"]);
    assert!(fixture.verify().is_err());
    fixture.json["histories"][0]["checkpoints"][0]["introduces"] = serde_json::json!([]);
    fixture.json["histories"][0]["checkpoints"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(fixture.verify().is_err());
}

#[test]
fn fail_closed_on_nonancestor_stages_and_source_changes() {
    let mut fixture = Fixture::new();
    fixture.json["histories"][0]["checkpoints"][2]["commit"] = serde_json::json!(fixture.second[2]);
    assert!(fixture.verify().is_err());
    fixture.json["histories"][0]["checkpoints"][2]["commit"] = serde_json::json!(fixture.first[2]);
    // A synthetic stage using different bytes for the reader cannot claim
    // preservation, even if the following stage restores the correct bytes.
    fs::write(
        fixture.root.join("reader.rs"),
        "pub fn read() { panic!() }\n",
    )
    .unwrap();
    git(&fixture.root, &["read-tree", &fixture.first[1]]);
    git(&fixture.root, &["add", "--", "reader.rs"]);
    let wrong_tree = git(&fixture.root, &["write-tree"]);
    let wrong_commit = git(
        &fixture.root,
        &[
            "commit-tree",
            &wrong_tree,
            "-p",
            &fixture.first[0],
            "-m",
            "SYNTHETIC wrong reader",
        ],
    );
    fixture.json["histories"][0]["checkpoints"][1]["commit"] = serde_json::json!(wrong_commit);
    assert!(fixture.verify().is_err());
}

#[test]
fn real_merge_rejects_wrong_parentage_and_missing_independent_contract_closure() {
    let mut fixture = Fixture::new();
    let original = fixture.json["integration_nodes"][0]["parents"][1].clone();
    fixture.json["integration_nodes"][0]["parents"][1] = serde_json::json!(fixture.first[2]);
    assert!(fixture.verify().is_err());
    fixture.json["integration_nodes"][0]["parents"][1] = original;
    fixture.json["integration_nodes"][0]["preserves"] = serde_json::json!(["integrate"]);
    assert!(fixture.verify().is_err());
    fixture.json["integration_nodes"][0]["preserves"] = serde_json::json!(["read", "write"]);
    assert_eq!(
        fixture.json["integration_nodes"][0]["commit"],
        fixture.merged
    );
    assert!(fixture.verify().is_ok());
}