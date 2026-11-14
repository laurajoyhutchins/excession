//! Real Git and SQLite-bound executable oracle at a reconstructed capability introduction.
use excession::digest;
use excession::evidence::{build, rea};
use excession::json::Value;
use excession::sqlite::Db;
use excession::{evidence_store, git_reconstruction_oracles};
use serde_json::{Value as J, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    plan: PathBuf,
    manifest: PathBuf,
    checks: PathBuf,
    oracle: PathBuf,
    db: Db,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-git-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "--object-format=sha1"]);
        fs::write(repo.join("common.py"), "def common(): return 3\n").unwrap();
        git(&repo, &["add", "common.py"]);
        git(&repo, &["commit", "-qm", "SYNTHETIC foundation"]);
        let root_commit = git(&repo, &["rev-parse", "HEAD"]);
        fs::write(
            repo.join("feature.py"),
            "from common import common\ndef feature(): return common()+1\n",
        )
        .unwrap();
        git(&repo, &["add", "feature.py"]);
        git(&repo, &["commit", "-qm", "SYNTHETIC feature"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let plan = root.join("plan.json");
        fs::write(&plan, json!({
            "schema":"excession.git-reconstruction-plan.v1",
            "target_commit":head,
            "source_dependencies":[{"path":"feature.py","requires":["common.py"]}],
            "contracts":[{"id":"feature","source_closure":["common.py","feature.py"],"requires":[]}],
            "histories":[{"name":"feature-first","checkpoints":[
                {"commit":root_commit,"introduces":[]},
                {"commit":head,"introduces":["feature"]}
            ]}],
            "integration_nodes":[]
        }).to_string()).unwrap();
        let artifact = root.join("app.bin");
        fs::write(&artifact, b"bounded artifact bytes\n").unwrap();
        let db = Db::open(&root.join("evidence.sqlite")).unwrap();
        let input_paths = ["common.py", "feature.py"];
        let tc = digest::sha256(b"test-toolchain");
        let config = digest::sha256(b"test-configuration");
        let build_result = build::record(
            &db,
            &build::BuildSpec {
                repo: &repo,
                revision: &head,
                artifact: &artifact,
                inputs: &input_paths,
                toolchain: &tc,
                configuration: &config,
                kind: "OTHER",
            },
        )
        .unwrap();
        let build = build_result
            .get("snapshot_id")
            .unwrap()
            .str()
            .unwrap()
            .to_owned();
        let sha = digest::sha256(&fs::read(&artifact).unwrap());
        let provider = json!({"id":"fixture","name":"Independent evidence fixture","version":"1"});
        let subject = json!({
            "name":"app.bin", "digest":{"sha256":sha}, "local_path":artifact.to_str().unwrap(),
            "format":"raw", "architecture":null
        });
        let mut record = json!({
            "subject":subject, "provider":provider, "analysis_profile":null,
            "predicate_type":"rea.analysis", "operation":"inspect_artifact",
            "parameters":{}, "raw_result":null, "normalized_result":{"bytes":23},
            "confidence":"observed", "authority":"shipped-artifact", "environment":null,
            "limitations":["bytes only"], "relationship_coverage":null,
            "locations":[], "evidence_links":[]
        });
        // This independent fixture occupies the supported ASCII/integer RFC 8785 subset.
        // serde_json's lexically ordered keys agree with the implementation's JCS
        // ordering for these ASCII keys; display-only subject keys are excluded.
        let mut semantic = record.clone();
        semantic["subject"] = json!({"digest":{"sha256":sha},"format":"raw","architecture":null});
        let id = format!("ev_{}", digest::sha256(semantic.to_string().as_bytes()));
        record["evidence_id"] = J::String(id);
        let bundle = root.join("bundle.json");
        fs::write(
            &bundle,
            json!({
                "artifacts":[{"digest":{"sha256":sha},"format":"raw","architecture":null}],
                "providers":[provider],"environments":[],"scenarios":[],"captures":[],
                "unknowns":[], "records":[record]
            })
            .to_string(),
        )
        .unwrap();
        let imported = rea::import(
            &db,
            &rea::ImportSpec {
                artifact: &artifact,
                bundle: &bundle,
                build: Some(rea::BuildLink {
                    repo: &repo,
                    revision: &head,
                    snapshot: &build,
                }),
            },
        )
        .unwrap();
        let rea = imported.get("bundle_id").unwrap().str().unwrap().to_owned();
        let manifest = root.join("bindings.json");
        fs::write(
            &manifest,
            json!({
                "schema":"excession.git-reconstruction-evidence.v1",
                "bindings":[{"phase":"INTRODUCTION","checkpoint":head,"contract":"feature",
                    "artifact":artifact.to_str().unwrap(),"build_snapshot":build,"rea_bundle":rea}]
            })
            .to_string(),
        )
        .unwrap();
        let oracle = root.join("independent_oracle.py");
        fs::write(&oracle, "import sys\nfrom pathlib import Path\nsys.path.insert(0,str(Path.cwd()))\nfrom feature import feature\nassert feature()==4\nprint('PASS')\n").unwrap();
        let program = fs::canonicalize("/usr/bin/python3").unwrap();
        let checks = root.join("checks.json");
        fs::write(
            &checks,
            json!({
                "schema":"excession.git-reconstruction-oracles.v1",
                "checks":[{
                    "phase":"INTRODUCTION", "checkpoint":head, "contracts":["feature"],
                    "checkout":repo.to_str().unwrap(), "program":program.to_str().unwrap(),
                    "program_sha256":digest::sha256(&fs::read(&program).unwrap()),
                    "oracle_source":oracle.to_str().unwrap(),
                    "oracle_sha256":digest::sha256(&fs::read(&oracle).unwrap()),
                    "args":[oracle.to_str().unwrap()], "timeout_ms":5000,
                    "expected_stdout_sha256":digest::sha256(b"PASS\n")
                }]
            })
            .to_string(),
        )
        .unwrap();
        Self {
            root,
            repo,
            plan,
            manifest,
            checks,
            oracle,
            db,
        }
    }
    fn replay(&self, trusted: bool) -> excession::Result<Value> {
        git_reconstruction_oracles::replay(
            &self.db,
            &self.repo,
            &self.plan,
            &self.manifest,
            &self.checks,
            trusted,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn pinned_oracle_replays_real_build_rea_and_source_bound_behavior() {
    let f = Fixture::new();
    assert!(f.replay(false).is_err(), "untrusted invocation must refuse");
    let observed = f.replay(true).unwrap();
    assert_eq!(observed.get("checked_count").unwrap().integer().unwrap(), 1);
    assert_eq!(
        observed.get("check_status").unwrap().str().unwrap(),
        "ALL_DECLARED_BOUNDED_CHECKS_PASSED"
    );
    assert_eq!(
        observed
            .get("behavioral_equivalence")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_ESTABLISHED"
    );
    assert!(!observed.get("execution_attested").unwrap().bool().unwrap());
    let stored = observed.get("checks").unwrap().array_ref().unwrap()[0]
        .get("snapshot_id")
        .unwrap()
        .str()
        .unwrap();
    assert_eq!(
        evidence_store::audit(&f.db, stored)
            .unwrap()
            .get("source_authentication")
            .unwrap()
            .str()
            .unwrap(),
        "NOT_CHECKED"
    );
    assert_eq!(
        observed,
        f.replay(true).unwrap(),
        "identical observations are idempotent"
    );
    let schema: J = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/git-reconstruction-oracles.v1.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let input: J = serde_json::from_slice(&fs::read(&f.checks).unwrap()).unwrap();
    assert!(validator.is_valid(&input));
}

#[test]
fn oracle_refuses_falsified_expectation_and_mutated_source() {
    let f = Fixture::new();
    let mut input: J = serde_json::from_slice(&fs::read(&f.checks).unwrap()).unwrap();
    input["checks"][0]["expected_stdout_sha256"] = J::String("a".repeat(64));
    fs::write(&f.checks, input.to_string()).unwrap();
    assert!(f.replay(true).unwrap_err().contains("oracle output"));
    // A clean checkpoint is a required condition, not an optional preference.
    fs::write(f.repo.join("feature.py"), "def feature(): return 400\n").unwrap();
    assert!(f.replay(true).is_err());
    git(&f.repo, &["checkout", "--", "feature.py"]);
    let mut input: J = serde_json::from_slice(&fs::read(&f.checks).unwrap()).unwrap();
    input["checks"][0]["expected_stdout_sha256"] = J::String(digest::sha256(b"PASS\n"));
    fs::write(&f.checks, input.to_string()).unwrap();
    fs::write(&f.oracle, "print('FAIL')\n").unwrap();
    assert!(f.replay(true).unwrap_err().contains("digest mismatch"));
    input["checks"][0]["oracle_sha256"] = J::String(digest::sha256(&fs::read(&f.oracle).unwrap()));
    fs::write(&f.checks, input.to_string()).unwrap();
    assert!(f.replay(true).unwrap_err().contains("oracle output"));
}
