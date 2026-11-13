//! Artifact-first Ghidra FID/BSim ingestion and corpus-wide source candidate joins.
//! Provider fixtures are synthetic: they test fidelity and authority boundaries,
//! not Ghidra's upstream search behavior.
use excession::digest;
use excession::json::{Value, obj};
use excession::sqlite::Db;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn tool")
}
fn good(out: Output) -> Vec<u8> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}
fn bad(out: Output) {
    assert!(!out.status.success(), "unexpected acceptance");
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn read(out: Output) -> Value {
    excession::json::parse(&good(out)).unwrap()
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).unwrap().str().unwrap()
}
fn fid() -> Value {
    obj(&[
        ("engine", Value::string("FID")),
        ("database", obj(&[
            ("basis", Value::string("DECLARED_NAME")),
            ("identity", Value::string("fiddb:sample-x86-64-v1")),
        ])),
        ("library_family",Value::string("sample-c-library")),
        ("library_version",Value::string("1.0")),
        ("library_variant",Value::string("clang-O2")),
        ("function",Value::string("triple")),
    ])
}
fn bsim() -> Value {
    obj(&[
        ("engine",Value::string("BSIM")),
        ("database",obj(&[
            ("basis",Value::string("DECLARED_NAME")),
            ("identity",Value::string("bsim:h2:sample-corpus-v1")),
        ])),
        ("executable_md5",Value::string("b".repeat(32))),
        ("entry",Value::string("ram:00001000")),
        ("function",Value::string("triple")),
    ])
}
fn evidence(candidate: Value, entry: &str) -> Value {
    let engine=candidate.get("engine").unwrap().str().unwrap();
    let score=if engine=="FID" {
        obj(&[
            ("engine",Value::string("FID")),
            ("overall_score",Value::string("23.6")),
            ("match_mode",Value::string("FULL_HASH")),
        ])
    } else {
        obj(&[
            ("engine",Value::string("BSIM")),
            ("similarity",Value::string("0.96")),
            ("confidence",Value::string("8.7")),
        ])
    };
    obj(&[
        ("entry",Value::string(entry)),
        ("candidate",candidate),
        ("evidence",score),
    ])
}
struct Fixture {
    root:PathBuf,
    repo:PathBuf,
    db:PathBuf,
    native:PathBuf,
    report:PathBuf,
    candidate:PathBuf,
    first:PathBuf,
    second:PathBuf,
    commit:String,
}
impl Fixture {
    fn new() -> Self {
        let root=std::env::temp_dir().join(format!(
            "excession-ghidra-corpus-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)
        ));
        let repo=root.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        good(run("git",&["-C",repo.to_str().unwrap(),"init","-q"]));
        fs::write(repo.join("src/lib.c"),"int triple(int x) { return x * 3; }\n").unwrap();
        good(run("git",&["-C",repo.to_str().unwrap(),"add","."]));
        good(run("git",&[
            "-C",repo.to_str().unwrap(),"-c","user.name=Fixture",
            "-c","user.email=fixture@example.invalid","commit","-qm","source",
        ]));
        let commit=String::from_utf8(good(run("git",&[
            "-C",repo.to_str().unwrap(),"rev-parse","HEAD",
        ]))).unwrap().trim().to_owned();
        let first=root.join("a.o");
        let second=root.join("b.o");
        fs::write(repo.join("second.c"),"int another(int x) { return x * 3; }\n").unwrap();
        for (src,obj) in [(repo.join("src/lib.c"),&first),(repo.join("second.c"),&second)] {
            good(run("cc",&[
                "-c","-O2","-o",obj.to_str().unwrap(),src.to_str().unwrap()
            ]));
        }
        let native=root.join("native.txt");
        fs::write(&native,"Synthetic test-only FID and BSim observations (not a Ghidra run)\n").unwrap();
        let report=root.join("normalized.json");
        let candidate=root.join("candidate.json");
        let db=root.join("evidence.sqlite");
        Self{root,repo,db,native,report,candidate,first,second,commit}
    }
    fn path(path:&Path)->&str {path.to_str().unwrap()}
    fn report(&self, artifact:&Path, observations:Vec<Value>) {
        let binary=fs::read(artifact).unwrap();
        let raw=fs::read(&self.native).unwrap();
        let payload=obj(&[
            ("schema",Value::string("excession.ghidra-matches.v1")),
            ("artifact",obj(&[
                ("sha256",Value::string(digest::sha256(&binary))),
                ("size_bytes",Value::number(binary.len())),
            ])),
            ("producer",obj(&[
                ("tool",Value::string("Ghidra")),
                ("version",Value::string("12.2")),
                ("analysis_profile_sha256",Value::string("9".repeat(64))),
                ("native_receipt_sha256",Value::string(digest::sha256(&raw))),
            ])),
            ("matches",Value::array(observations)),
        ]);
        fs::write(&self.report,payload.canonical()).unwrap();
    }
    fn import(&self, artifact:&Path) -> Output {
        cli(&[
            "evidence","ghidra-import","--db",Self::path(&self.db),
            "--artifact",Self::path(artifact),"--report",Self::path(&self.report),
            "--native",Self::path(&self.native),
        ])
    }
    fn binding(&self, provider:Value)->Output {
        fs::write(&self.candidate,provider.canonical()).unwrap();
        cli(&[
            "evidence","ghidra-source-bind","--db",Self::path(&self.db),
            "--repo",Self::path(&self.repo),"--rev",&self.commit,
            "--source","src/lib.c","--candidate",Self::path(&self.candidate),
            "--symbol","triple",
        ])
    }
    fn candidates(&self, artifact:&Path, entry:&str, limit:&str)->Output {
        cli(&[
            "inspect","ghidra-candidates","--db",Self::path(&self.db),
            "--artifact",Self::path(artifact),"--entry",entry,"--limit",limit,
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {let _=fs::remove_dir_all(&self.root);}
}
#[test]
fn two_upstream_match_families_share_pinned_git_source_in_one_corpus() {
    let f=Fixture::new();
    f.report(&f.first,vec![evidence(fid(),"ram:00401000"),evidence(bsim(),"ram:00401000")]);
    let imported=read(f.import(&f.first));
    assert_eq!(imported.get("match_count").unwrap().integer().unwrap(),2);
    assert!(imported.get("artifact_bytes_verified").unwrap().bool().unwrap());
    assert!(!imported.get("producer_execution_attested").unwrap().bool().unwrap());
    assert_eq!(field(&imported,"semantic_source_correspondence"),"NOT_ESTABLISHED");
    assert_eq!(read(f.import(&f.first)).get("report_id").unwrap(),imported.get("report_id").unwrap());

    let before=read(f.candidates(&f.first,"ram:00401000","16"));
    assert_eq!(before.get("total_matches").unwrap().integer().unwrap(),2);
    for m in before.get("matches").unwrap().array_ref().unwrap() {
        assert!(m.get("source_candidates").unwrap().array_ref().unwrap().is_empty());
    }
    let fid_source=read(f.binding(fid()));
    let bsim_source=read(f.binding(bsim()));
    assert!(fid_source.get("binding_id").is_ok());
    assert_ne!(field(&fid_source,"binding_id"),field(&bsim_source,"binding_id"));
    let after=read(f.candidates(&f.first,"ram:00401000","16"));
    for m in after.get("matches").unwrap().array_ref().unwrap() {
        let sources=m.get("source_candidates").unwrap().array_ref().unwrap();
        assert_eq!(sources.len(),1);
        let record=&sources[0];
        assert_eq!(record.get("source_git_replayed").unwrap().bool().unwrap(),true);
        assert_eq!(field(record,"binary_source_equivalence"),"NOT_ESTABLISHED");
        assert_eq!(field(record.get("source").unwrap(),"path"),"src/lib.c");
        assert_eq!(field(record.get("source").unwrap(),"commit"),f.commit);
        assert_eq!(field(m,"binary_source_correspondence"),"NOT_ESTABLISHED");
    }
    let schema:serde_json::Value=serde_json::from_str(include_str!(
        "../../contracts/evidence-core/ghidra-matches.v1.schema.json"
    )).unwrap();
    let validator=jsonschema::draft202012::new(&schema).unwrap();
    let payload:serde_json::Value=serde_json::from_slice(&fs::read(&f.report).unwrap()).unwrap();
    assert!(validator.is_valid(&payload));
    let db=Db::open(&f.db).unwrap();
    assert_eq!(db.scalar("SELECT count(*) FROM evidence_ghidra_report_v1",&[]).unwrap(),"1");
    assert_eq!(db.scalar("SELECT count(*) FROM evidence_ghidra_match_v1",&[]).unwrap(),"2");
    assert_eq!(db.scalar("SELECT count(*) FROM evidence_ghidra_source_v1",&[]).unwrap(),"2");
    assert!(db.exec("UPDATE evidence_ghidra_report_v1 SET raw_report='{}'").is_err());
    assert!(db.exec("DELETE FROM evidence_ghidra_match_v1").is_err());
    assert!(db.exec("DELETE FROM evidence_ghidra_source_v1").is_err());
}
#[test]
fn matches_from_distinct_artifacts_join_same_source_without_reanalysis() {
    let f=Fixture::new();
    read(f.binding(fid()));
    f.report(&f.first,vec![evidence(fid(),"ram:1000")]);
    read(f.import(&f.first));
    f.report(&f.second,vec![evidence(fid(),"ram:2000")]);
    read(f.import(&f.second));
    let first=read(f.candidates(&f.first,"ram:1000","1"));
    let second=read(f.candidates(&f.second,"ram:2000","1"));
    let a=&first.get("matches").unwrap().array_ref().unwrap()[0];
    let b=&second.get("matches").unwrap().array_ref().unwrap()[0];
    assert_ne!(field(&first,"artifact_subject_id"),field(&second,"artifact_subject_id"));
    assert_eq!(field(a,"candidate_id"),field(b,"candidate_id"));
    assert_eq!(
        field(&a.get("source_candidates").unwrap().array_ref().unwrap()[0],"binding_id"),
        field(&b.get("source_candidates").unwrap().array_ref().unwrap()[0],"binding_id"),
    );
    assert_eq!(f.candidates(&f.second,"ram:1000","1").status.success(),true);
}
#[test]
fn corrupted_provider_receipts_and_artifact_identities_refuse_closed() {
    let f=Fixture::new();
    f.report(&f.first,vec![evidence(fid(),"ram:1000")]);
    fs::write(&f.native,"changed native receipts").unwrap();
    bad(f.import(&f.first));
    assert_eq!(f.candidates(&f.first,"ram:1000","1").status.success(),true);
    f.report(&f.first,vec![evidence(fid(),"ram:1000")]);
    let report_json=fs::read_to_string(&f.report).unwrap();
    let broken=report_json.replace(
        &digest::sha256(&fs::read(&f.first).unwrap()),&"0".repeat(64)
    );
    fs::write(&f.report,broken).unwrap();
    bad(f.import(&f.first));
    f.report(&f.first,vec![evidence(fid(),"ram:1000"),evidence(fid(),"ram:1000")]);
    bad(f.import(&f.first));
    f.report(&f.first,vec![evidence(fid(),"ram:1000")]);
    read(f.import(&f.first));
    fs::OpenOptions::new().append(true).open(&f.first).unwrap()
        .write_all(b"tampered").unwrap();
    bad(f.candidates(&f.first,"ram:1000","1"));
}
#[test]
fn source_associations_reject_forged_git_sources_and_constrain_output() {
    let f=Fixture::new();
    f.report(&f.first,vec![
        evidence(fid(),"ram:1000"),evidence(bsim(),"ram:1000")
    ]);
    read(f.import(&f.first));
    fs::write(&f.candidate,fid().canonical()).unwrap();
    bad(cli(&[
        "evidence","ghidra-source-bind","--db",Fixture::path(&f.db),
        "--repo",Fixture::path(&f.repo),"--rev",&f.commit,
        "--source","src/does-not-exist.c",
        "--candidate",Fixture::path(&f.candidate),"--symbol","triple",
    ]));
    read(f.binding(fid()));
    let truncated=read(f.candidates(&f.first,"ram:1000","1"));
    assert_eq!(truncated.get("total_matches").unwrap().integer().unwrap(),2);
    assert!(truncated.get("results_truncated").unwrap().bool().unwrap());
    bad(f.candidates(&f.first,"ram:1000","0"));
    let db=Db::open(&f.db).unwrap();
    assert!(db.exec("UPDATE evidence_ghidra_match_v1 SET candidate_id='forged'").is_err());
}
