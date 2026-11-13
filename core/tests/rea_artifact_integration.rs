//! REA foreign evidence test: independent JS canonical oracle and real Rust ELF.
use excession::digest;
use excession::sqlite::Db;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("launch")
}
fn okay(out: Output) -> Vec<u8> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}
fn reject(out: Output) {
    assert!(!out.status.success(), "unexpected acceptance");
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output { run(env!("CARGO_BIN_EXE_excession"), args) }
fn result(out: Output) -> Value { serde_json::from_slice(&okay(out)).unwrap() }

/// Independently hash the REA-documented semantic projection via Node's
/// JSON.stringify and UTF-16-sorted object keys, not Excession's Rust path.
fn node_native_id(record: &Value) -> String {
    let script = r#"
const crypto = require('node:crypto');
let body = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', x => body += x);
process.stdin.on('end', () => {
  const record = JSON.parse(body);
  const subject = record.subject === null ? null : {
    digest: record.subject.digest,
    format: record.subject.format,
    architecture: record.subject.architecture
  };
  const keys = ['provider', 'analysis_profile', 'predicate_type', 'operation',
    'parameters', 'raw_result', 'normalized_result', 'confidence', 'authority',
    'environment', 'limitations', 'relationship_coverage', 'locations',
    'evidence_links'];
  const semantic = { subject };
  for (const key of keys) semantic[key] = record[key];
  const sort = x => Array.isArray(x) ? x.map(sort) : x !== null && typeof x === 'object'
    ? Object.fromEntries(Object.keys(x).sort().map(k => [k, sort(x[k])])) : x;
  const canonical = JSON.stringify(sort(semantic));
  process.stdout.write('ev_' + crypto.createHash('sha256').update(canonical).digest('hex'));
});
"#;
    let mut child = Command::new("node").arg("-e").arg(script)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(record.to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    String::from_utf8(okay(output)).unwrap()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    artifact: PathBuf,
    bundle: PathBuf,
    db: PathBuf,
    rev: String,
    foreign_id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-rea-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        okay(run("git", &["-C", repo.to_str().unwrap(), "init", "-q"]));
        fs::write(repo.join("src/main.rs"), "fn main() { println!(\"REA\"); }\n").unwrap();
        okay(run("git", &["-C", repo.to_str().unwrap(), "add", "."]));
        okay(run("git", &[
            "-C", repo.to_str().unwrap(), "-c", "user.name=Fixture",
            "-c", "user.email=fixture@example.invalid", "commit", "-qm", "pinned",
        ]));
        let rev = String::from_utf8(okay(run(
            "git", &["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"]
        ))).unwrap().trim().to_owned();
        let artifact = root.join("application");
        okay(run("rustc", &[
            "--edition=2024", "-C", "debuginfo=0",
            repo.join("src/main.rs").to_str().unwrap(),
            "-o", artifact.to_str().unwrap(),
        ]));
        let sha = digest::sha256(&fs::read(&artifact).unwrap());
        let mut observation = json!({
            "subject": {
                "name": "application",
                "digest": {"sha256": sha},
                "local_path": artifact.to_str().unwrap(),
                "format": "elf",
                "architecture": null
            },
            "provider": {"id":"fixture", "name":"REA fixture producer", "version":"1"},
            "analysis_profile": null,
            "predicate_type": "rea.analysis",
            "operation": "inspect_artifact",
            "parameters": {},
            "raw_result": null,
            "normalized_result": {"functions": 1, "kind":"native-code"},
            "confidence": "observed",
            "authority": "shipped-artifact",
            "environment": null,
            "limitations": ["no independent execution evidence"],
            "relationship_coverage": null,
            "locations": [],
            "evidence_links": []
        });
        let foreign_id = node_native_id(&observation);
        observation["evidence_id"] = json!(foreign_id);
        let bundle = root.join("evidence.json");
        let payload = json!({
            "artifacts":[{"digest":{"sha256":sha}, "format":"elf", "architecture":null}],
            "providers":[{"id":"fixture", "name":"REA fixture producer", "version":"1"}],
            "environments":[], "scenarios":[], "captures":[], "unknowns":[],
            "records":[observation]
        });
        fs::write(&bundle, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();
        let db = root.join("records.sqlite");
        Self { root, repo, artifact, bundle, db, rev, foreign_id }
    }
    fn artifact(&self) -> &str { self.artifact.to_str().unwrap() }
    fn repo(&self) -> &str { self.repo.to_str().unwrap() }
    fn db(&self) -> &str { self.db.to_str().unwrap() }
    fn bundle(&self) -> &str { self.bundle.to_str().unwrap() }
    fn import(&self) -> Output {
        cli(&["evidence","rea-import","--db",self.db(),
            "--artifact",self.artifact(),"--bundle",self.bundle()])
    }
    fn audit(&self, id: &str) -> Output {
        cli(&["inspect","rea-evidence","--db",self.db(),
            "--artifact",self.artifact(),"--id",id])
    }
    fn build(&self) -> String {
        let toolchain = digest::sha256(b"toolchain-under-test");
        let config = digest::sha256(b"test-edition=2024");
        let report = result(cli(&[
            "evidence","build-record","--db",self.db(),
            "--repo",self.repo(),"--rev",&self.rev,
            "--artifact",self.artifact(),"--sources","src/main.rs",
            "--toolchain",&toolchain,"--configuration",&config,
            "--representation","EXECUTABLE",
        ]));
        report["snapshot_id"].as_str().unwrap().to_owned()
    }
    fn linked_import(&self, build_id: &str) -> Output {
        cli(&[
            "evidence","rea-import","--db",self.db(),
            "--artifact",self.artifact(),"--bundle",self.bundle(),
            "--repo",self.repo(),"--rev",&self.rev,"--build-snapshot",build_id
        ])
    }
    fn linked_audit(&self, id: &str, build_id: &str) -> Output {
        cli(&[
            "inspect","rea-evidence","--db",self.db(),
            "--artifact",self.artifact(),"--id",id,
            "--repo",self.repo(),"--rev",&self.rev,"--build-snapshot",build_id
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}
#[test]
fn rea_artifact_import_roundtrips_native_id_and_source_free_evidence() {
    let f = Fixture::new();
    let first = result(f.import());
    assert_eq!(first["producer"], "REA");
    assert_eq!(first["format"], "EVIDENCE_V2_EXPORT");
    assert_eq!(first["native_record_ids"][0], f.foreign_id);
    assert_eq!(first["native_identity"], "REHASHED_SUPPORTED_RFC8785_INTEGER_DOMAIN");
    assert_eq!(first["build_snapshot_id"], Value::Null);
    assert_eq!(first["semantic_correctness"], "NOT_ESTABLISHED");
    assert_eq!(first["source_artifact_relation"], "NOT_ESTABLISHED");
    assert_eq!(first["imported_record_count"], 1);
    assert_eq!(first["unknown_graph_verification"], "NOT_ESTABLISHED");
    let id = first["bundle_id"].as_str().unwrap();
    let audited = result(f.audit(id));
    assert_eq!(audited, first);
    assert_eq!(result(f.import()), first, "repeat import must preserve exact identity");
    let native = result(cli(&[
        "inspect","rea-record","--db",f.db(),
        "--artifact",f.artifact(),"--id",id,"--record",&f.foreign_id
    ]));
    assert_eq!(native["native_evidence_id"], f.foreign_id);
    assert_eq!(native["original_record"]["predicate_type"], "rea.analysis");
    assert_eq!(native["original_record"]["normalized_result"]["functions"], 1);
    assert_eq!(native["semantic_correctness"], "NOT_ESTABLISHED");

    let schema: Value = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/rea-import.v1.schema.json"
    )).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    assert!(validator.is_valid(&first), "actual REA CLI report violates JSON Schema");
}
#[test]
fn declared_build_link_joins_exact_artifact_without_semantic_promotion() {
    let f = Fixture::new();
    let build = f.build();
    let original = result(f.import());
    let linked = result(f.linked_import(&build));
    assert_eq!(linked["build_snapshot_id"], build);
    assert_eq!(original["artifact_subject_id"], linked["artifact_subject_id"]);
    assert_ne!(original["bundle_id"], linked["bundle_id"]);
    assert_eq!(linked["source_artifact_relation"], "NOT_ESTABLISHED");
    let id = linked["bundle_id"].as_str().unwrap();
    reject(f.audit(id)); // Must replay the originally recorded build link.
    assert_eq!(result(f.linked_audit(id, &build)), linked);
    reject(cli(&[
        "inspect","rea-evidence","--db",f.db(),
        "--artifact",f.artifact(),"--id",id,
        "--repo",f.repo(),"--rev",&"a".repeat(40),"--build-snapshot",&build
    ]));
    reject(cli(&[
        "evidence","rea-import","--db",f.db(),
        "--artifact",f.artifact(),"--bundle",f.bundle(),
        "--repo",f.repo()
    ]));
}
#[test]
fn forged_native_id_and_changed_binary_and_tampered_sql_fail_closed() {
    let f = Fixture::new();
    let first = result(f.import());
    let id = first["bundle_id"].as_str().unwrap();
    let original_bytes = fs::read(&f.bundle).unwrap();
    let mut modified: Value = serde_json::from_slice(&original_bytes).unwrap();
    modified["records"][0]["evidence_id"] = json!(format!("ev_{}", "f".repeat(64)));
    fs::write(&f.bundle, modified.to_string()).unwrap();
    reject(f.import());
    fs::write(&f.bundle, original_bytes).unwrap();
    fs::OpenOptions::new().append(true).open(&f.artifact).unwrap()
        .write_all(b"repacked").unwrap();
    reject(f.audit(id));
}
#[test]
fn sqlite_immutability_and_metadata_corruption_are_auditable() {
    let f = Fixture::new();
    let first = result(f.import());
    let id = first["bundle_id"].as_str().unwrap();
    let db = Db::open(&f.db).unwrap();
    assert!(db.exec("UPDATE evidence_rea_bundle_v1 SET raw_json='{}'").is_err());
    assert!(db.exec("DELETE FROM evidence_rea_bundle_v1").is_err());
    db.exec("DROP TRIGGER evidence_rea_bundle_no_update_v1").unwrap();
    db.exec("UPDATE evidence_rea_bundle_v1 SET artifact_sha256='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'").unwrap();
    reject(f.audit(id));
    reject(f.import());
}
