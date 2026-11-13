//! Independent C compiler fixtures for bounded binary-first reconstruction.
use excession::json::Value;
use excession::sqlite::Db;
use std::fs;
use std::path::{Path,PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64,Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(tool: &str, args: &[&str]) -> Output {
    Command::new(tool).args(args).output().expect("spawn fixture tool")
}
fn success(result: Output) -> Vec<u8> {
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    result.stdout
}
fn refused(result: Output) {
    assert!(!result.status.success(), "unexpected acceptance");
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"),args)
}
fn parsed(result: Output) -> Value {
    excession::json::parse(&success(result)).unwrap()
}
fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).unwrap().str().unwrap()
}
struct Fixture {
    home: PathBuf,
    target: PathBuf,
    source: PathBuf,
    output: PathBuf,
    db: PathBuf,
}
impl Fixture {
    fn new(original: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "excession-auto-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1,Ordering::Relaxed)
        ));
        fs::create_dir(&home).unwrap();
        let source = home.join("private-original.c");
        let target = home.join("target.o");
        let output = home.join("reconstructed.c");
        let db = home.join("store.sqlite");
        fs::write(&source,original).unwrap();
        compile(&source,&target);
        Self {home,target,source,output,db}
    }
    fn db(&self) -> &str { self.db.to_str().unwrap() }
    fn target(&self) -> &str { self.target.to_str().unwrap() }
    fn output(&self) -> &str { self.output.to_str().unwrap() }
    fn search(&self,budget:&str,consent:&str) -> Output {
        cli(&[
            "reconstruct","auto",
            "--db",self.db(),
            "--target",self.target(),
            "--out",self.output(),
            "--trusted-local",consent,
            "--max-candidates",budget,
        ])
    }
    fn searches(&self) -> Output {
        cli(&["reconstruct","searches","--db",self.db(),"--target",self.target()])
    }
}
fn compile(source:&Path, output:&Path) {
    success(run("cc",&[
        "-O2","-fno-stack-protector",
        "-fno-asynchronous-unwind-tables","-fno-ident",
        "-c","-o",output.to_str().unwrap(),source.to_str().unwrap(),
    ]));
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.home); }
}
#[test]
fn discovers_multiplication_without_reading_original_source() {
    let f=Fixture::new("int evaluate(int x) { return x * 3; }\n");
    let source_before=fs::read_to_string(&f.source).unwrap();
    let result=parsed(f.search("16","true"));
    assert_eq!(field(&result,"search_status"),"EXACT_TEXT_MATCH");
    assert_eq!(field(&result,"best_expression"),"x * 3");
    assert!(result.get("attempt_count").unwrap().integer().unwrap() <= 16);
    assert_eq!(result.get("best_mismatch_count").unwrap().integer().unwrap(),0);
    assert_eq!(field(&result,"behavioral_equivalence"),"NOT_ESTABLISHED");
    assert_eq!(field(&result,"source_recovery"),"NOT_ESTABLISHED");
    assert!(!result.get("build_execution_attested").unwrap().bool().unwrap());
    assert!(fs::read_to_string(f.output()).unwrap().contains("return x * 3;"));
    assert_eq!(fs::read_to_string(&f.source).unwrap(),source_before);
    let generated=f.home.join("recompiled.o");
    compile(&f.output,&generated);
    let target_text=run("objcopy",&["--dump-section", &format!(".text={}",f.home.join("target.text").display()),f.target()]);
    success(target_text);
    success(run("objcopy",&["--dump-section", &format!(".text={}",f.home.join("result.text").display()),generated.to_str().unwrap()]));
    assert_eq!(fs::read(f.home.join("target.text")).unwrap(),fs::read(f.home.join("result.text")).unwrap());

    let history=parsed(f.searches());
    assert_eq!(history.get("stored_search_count").unwrap().integer().unwrap(),1);
    assert_eq!(history.get("searches").unwrap().array_ref().unwrap()[0],result);
    let schema:serde_json::Value=serde_json::from_str(include_str!("../../contracts/evidence-core/reconstruction-search.v1.schema.json")).unwrap();
    let validator=jsonschema::draft202012::new(&schema).unwrap();
    let output:serde_json::Value=serde_json::from_str(&result.canonical()).unwrap();
    assert!(validator.is_valid(&output));
    let db=Db::open(&f.db).unwrap();
    assert!(db.exec("UPDATE evidence_reconstruction_search_v1 SET report='{}'").is_err());
    assert!(db.exec("DELETE FROM evidence_reconstruction_search_v1").is_err());
}
#[test]
fn finds_constant_offset_and_bitwise_from_binary_observations() {
    for (program,expected) in [
        ("int evaluate(int x) { return x + 7; }\n","x + 7"),
        ("int evaluate(int x) { return x ^ 85; }\n","x ^ 85"),
        ("int evaluate(int x) { return x * 3 + 2; }\n","x * 3 + 2"),
    ] {
        let f=Fixture::new(program);
        let result=parsed(f.search("64","true"));
        assert_eq!(field(&result,"search_status"),"EXACT_TEXT_MATCH", "program: {program}");
        assert_eq!(field(&result,"best_expression"),expected);
        assert!(result.get("attempt_count").unwrap().integer().unwrap() <= 64);
    }
}
#[test]
fn limited_budget_returns_partial_candidate_with_explicit_uncertainty() {
    let f=Fixture::new("int evaluate(int x) { return x * 11; }\n");
    let report=parsed(f.search("1","true"));
    assert_eq!(report.get("attempt_count").unwrap().integer().unwrap(),1);
    assert_eq!(field(&report,"search_status"),"CANDIDATE_BUDGET_EXHAUSTED");
    assert!(!report.get("search_exhaustive").unwrap().bool().unwrap());
    assert!(report.get("best_mismatch_count").unwrap().integer().unwrap()>0);
    assert!(!fs::read_to_string(f.output()).unwrap().is_empty());
    refused(f.search("1","true"));
}
#[test]
fn refuses_unsafe_invocation_and_ambiguous_functions() {
    let f=Fixture::new("int evaluate(int x) { return x * 3; }\n");
    refused(f.search("16","false"));
    refused(f.search("0","true"));
    refused(f.search("129","true"));
    assert!(!f.output.exists());
    let bad=Fixture::new("int foo(int x) { return x; }\nint bar(int x) { return x*3; }\n");
    refused(bad.search("8","true"));
    assert!(!bad.output.exists());
    let mut invalid=Fixture::new("int evaluate(int x) { return x+7; }\n");
    fs::write(&invalid.target,b"not ELF").unwrap();
    refused(invalid.search("8","true"));
    invalid.source.clear();
}
