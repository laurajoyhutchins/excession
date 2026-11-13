//! Real Git, SQLite build receipt, and REA bundle replay at a contract introduction.
use excession::digest;
use excession::evidence::{build, rea};
use excession::git_reconstruction_evidence;
use excession::json::Value;
use excession::sqlite::Db;
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
    artifact: PathBuf,
    bundle: PathBuf,
    db: Db,
    head: String,
    build: String,
    rea: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-git-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q", "--object-format=sha1"]);
        fs::write(repo.join("src/common.rs"), "pub fn core() {}\n").unwrap();
        git(&repo, &["add", "src/common.rs"]);
        git(&repo, &["commit", "-qm", "SYNTHETIC foundation"]);
        let root_commit = git(&repo, &["rev-parse", "HEAD"]);
        fs::write(repo.join("src/feature.rs"), "pub fn feature() {}\n").unwrap();
        git(&repo, &["add", "src/feature.rs"]);
        git(&repo, &["commit", "-qm", "SYNTHETIC feature"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let plan = root.join("plan.json");
        fs::write(&plan, json!({
            "schema":"excession.git-reconstruction-plan.v1",
            "target_commit":head,
            "source_dependencies":[{"path":"src/feature.rs","requires":["src/common.rs"]}],
            "contracts":[{"id":"feature","source_closure":["src/common.rs","src/feature.rs"],"requires":[]}],
            "histories":[{"name":"feature-first","checkpoints":[
                {"commit":root_commit,"introduces":[]},
                {"commit":head,"introduces":["feature"]}
            ]}],
            "integration_nodes":[]
        }).to_string()).unwrap();
        let artifact = root.join("app.bin");
        fs::write(&artifact, b"bounded artifact bytes\n").unwrap();
        let db = Db::open(&root.join("evidence.sqlite")).unwrap();
        let input_paths = ["src/common.rs", "src/feature.rs"];
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
        Self {
            root,
            repo,
            plan,
            manifest,
            artifact,
            bundle,
            db,
            head,
            build,
            rea,
        }
    }
    fn replay(&self) -> excession::Result<Value> {
        git_reconstruction_evidence::replay(&self.db, &self.repo, &self.plan, &self.manifest)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn real_build_and_rea_are_replayed_without_semantic_promotion() {
    let f = Fixture::new();
    let a = f.replay().unwrap();
    assert_eq!(a, f.replay().unwrap());
    assert_eq!(
        a.get("bindings").unwrap().array_ref().unwrap()[0]
            .get("rea_bundle")
            .unwrap()
            .str()
            .unwrap(),
        f.rea
    );
    assert_eq!(
        a.get("receipt_coverage").unwrap().str().unwrap(),
        "COMPLETE_REPLAYED_BYTES_ONLY"
    );
    assert_eq!(
        a.get("replayed_binding_count").unwrap().integer().unwrap(),
        1
    );
    assert!(!a.get("contract_oracles_verified").unwrap().bool().unwrap());
    assert!(
        !a.get("integration_behavior_verified")
            .unwrap()
            .bool()
            .unwrap()
    );
    assert!(!a.get("build_execution_attested").unwrap().bool().unwrap());
    fs::write(&f.artifact, b"tampered artifact").unwrap();
    assert!(f.replay().is_err());
}
#[test]
fn cannot_bind_an_incomplete_build_source_closure_or_wrong_receipt() {
    let f = Fixture::new();
    let tc = digest::sha256(b"test-toolchain");
    let config = digest::sha256(b"test-configuration");
    let narrow = build::record(
        &f.db,
        &build::BuildSpec {
            repo: &f.repo,
            revision: &f.head,
            artifact: &f.artifact,
            inputs: &["src/feature.rs"],
            toolchain: &tc,
            configuration: &config,
            kind: "OTHER",
        },
    )
    .unwrap();
    let narrow_id = narrow.get("snapshot_id").unwrap().str().unwrap();
    // Bind an independently audited REA bundle to *this* incomplete build,
    // eliminating wrong-link refusal as an explanation for the failure.
    let linked = rea::import(
        &f.db,
        &rea::ImportSpec {
            artifact: &f.artifact,
            bundle: &f.bundle,
            build: Some(rea::BuildLink {
                repo: &f.repo,
                revision: &f.head,
                snapshot: narrow_id,
            }),
        },
    )
    .unwrap();
    let narrow_bundle = linked.get("bundle_id").unwrap().str().unwrap();
    let mut data: J = serde_json::from_slice(&fs::read(&f.manifest).unwrap()).unwrap();
    data["bindings"][0]["build_snapshot"] = J::String(narrow_id.to_owned());
    data["bindings"][0]["rea_bundle"] = J::String(narrow_bundle.to_owned());
    fs::write(&f.manifest, data.to_string()).unwrap();
    let refusal = f.replay().unwrap_err();
    assert!(
        refusal.contains("omits source in contract closure"),
        "{refusal}"
    );
    data["bindings"][0]["build_snapshot"] = J::String(f.build.clone());
    data["bindings"][0]["rea_bundle"] = J::String("e".repeat(64));
    fs::write(&f.manifest, data.to_string()).unwrap();
    assert!(f.replay().is_err());
    let incomplete = json!({"schema":"excession.git-reconstruction-evidence.v1","bindings":[]});
    fs::write(&f.manifest, incomplete.to_string()).unwrap();
    let report = f.replay().unwrap();
    assert_eq!(
        report.get("receipt_coverage").unwrap().str().unwrap(),
        "INCOMPLETE"
    );
    assert_eq!(
        report
            .get("missing_binding_count")
            .unwrap()
            .integer()
            .unwrap(),
        1
    );
}
