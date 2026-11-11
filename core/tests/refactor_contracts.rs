//! Controlled Git and SCIP inputs for native refactor suggestions.
//! These are hypotheses, never evidence of independently observed behavior.
use excession::json::{self, Value};
use excession::sqlite::Db;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}
fn ok(out: Output) -> Vec<u8> {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}
fn denied(out: Output) {
    assert!(!out.status.success(), "unexpectedly accepted");
    assert!(out.stdout.is_empty());
}
fn cli(args: &[&str]) -> Output { run(env!("CARGO_BIN_EXE_excession"), args) }
fn data(args: &[&str]) -> Value { json::parse(&ok(cli(args))).unwrap() }
fn str_at<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).unwrap().str().unwrap() }
fn items<'a>(v: &'a Value, k: &str) -> &'a [Value] { v.get(k).unwrap().array_ref().unwrap() }
fn edit<'a>(v: &'a mut Value, k: &str) -> &'a mut Value {
    v.entries_mut().unwrap().get_mut(k).unwrap()
}
fn item(v: &mut Value, n: usize) -> &mut Value {
    match v { Value::Array(xs) => &mut xs[n], _ => panic!("expected array") }
}
struct Fixture {
    tmp: PathBuf,
    repo: PathBuf,
    commit: String,
    db: PathBuf,
    index: PathBuf,
    report: PathBuf,
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.tmp); } }
impl Fixture {
    fn new(mode: &str) -> Self {
        let tmp=std::env::temp_dir().join(format!("excession-refactor-{}-{}",
            std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        let repo=tmp.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let path=repo.to_str().unwrap();
        ok(run("git",&["-C",path,"init","-q"]));
        if mode=="python" {
            fs::create_dir_all(repo.join("pkg")).unwrap();
            fs::write(repo.join("pkg/__init__.py"),"").unwrap();
            fs::write(repo.join("pkg/a.py"),"from pkg import b\n\ndef shared(value):\n    return value + 1\n").unwrap();
            fs::write(repo.join("pkg/b.py"),"from pkg import a\n\ndef shared(value):\n    return value + 1\n").unwrap();
        }else {
            fs::write(repo.join("lib.rs"),"pub fn emit() {}\n").unwrap();
            fs::write(repo.join("app.py"),"def parse():\n    return 1\n").unwrap();
        }
        ok(run("git",&["-C",path,"add","."]));
        ok(run("git",&["-C",path,"-c","user.name=Fixture","-c","user.email=fixture@example.invalid",
            "commit","-qm","controlled source"]));
        let commit=String::from_utf8(ok(run("git",&["-C",path,"rev-parse","HEAD"])))
            .unwrap().trim().to_owned();
        let db=tmp.join("index.sqlite");
        if mode=="python" {
            ok(cli(&["index","--repo",path,"--db",db.to_str().unwrap(),
                "--name","fixture","--rev",&commit,"--roots","pkg"]));
        }
        let index=tmp.join("scip.json");let report=tmp.join("report.json");
        Self { tmp,repo,commit,db,index,report }
    }
    fn repo(&self)->&str { self.repo.to_str().unwrap() }
    fn suggest(&self)->Value {
        data(&["refactor","suggest","--db",self.db.to_str().unwrap(),
            "--name","fixture","--rev",&self.commit])
    }
    fn audit(&self)->Output {
        cli(&["refactor","audit","--db",self.db.to_str().unwrap(),
            "--name","fixture","--rev",&self.commit,"--report",self.report.to_str().unwrap()])
    }
    fn scip_suggest(&self,index:&Value)->Output {
        fs::write(&self.index,index.canonical()).unwrap();
        cli(&["refactor","scip-suggest","--repo",self.repo(),"--rev",&self.commit,
            "--index",self.index.to_str().unwrap()])
    }
    fn scip_audit(&self)->Output {
        cli(&["refactor","scip-audit","--repo",self.repo(),"--rev",&self.commit,
            "--index",self.index.to_str().unwrap(),"--report",self.report.to_str().unwrap()])
    }
}
fn scip_index()->Value {
    json::parse(br#"{
        "metadata":{"tool_info":{"name":"controlled-scip-fixture","version":"0.1"}},
        "documents":[
            {"relative_path":"lib.rs","language":"rust","position_encoding":1,
              "occurrences":[
                {"range":[0,7,11],"symbol":"scip rust cargo example 0.1.0 api/emit().","symbol_roles":1},
                {"range":[0,7,11],"symbol":"scip python pypi example 0.1.0 app/parse()."}]},
            {"relative_path":"app.py","language":"python","position_encoding":1,
              "occurrences":[
                {"range":[0,4,9],"symbol":"scip python pypi example 0.1.0 app/parse().","symbol_roles":1},
                {"range":[0,4,9],"symbol":"scip rust cargo example 0.1.0 api/emit()."}]}
        ]
    }"#).unwrap()
}
#[test]
fn source_bound_refactor_cycle_and_duplicate_body_replay() {
    let f=Fixture::new("python");
    let report=f.suggest();
    assert_eq!(str_at(&report,"schema"),"excession.refactor-candidates.v1");
    assert_eq!(str_at(&report,"source_commit"),f.commit);
    assert_eq!(str_at(&report,"status"),"UNVERIFIED_REFACTOR_HYPOTHESES");
    let candidates=items(&report,"candidates");
    assert_eq!(candidates.len(),2);
    let cycle=candidates.iter().find(|c|str_at(c,"kind")=="dependency_cycle").unwrap();
    let names=items(cycle,"modules").iter().map(|v|str_at(v,"name")).collect::<Vec<_>>();
    assert!(names.contains(&"pkg.a")&&names.contains(&"pkg.b"));
    assert_eq!(items(cycle,"edges").len(),2);
    let duplicate=candidates.iter().find(|c|str_at(c,"kind")=="duplicate_body_fingerprint").unwrap();
    assert_eq!(str_at(duplicate,"language"),"python");
    let symbols=items(duplicate,"symbols");
    assert_eq!(symbols.len(),2);
    for entry in symbols {
        let path=str_at(entry,"path");
        assert!(["pkg/a.py","pkg/b.py"].contains(&path));
        let spec=format!("{}:{path}",f.commit);
        let observed=String::from_utf8(ok(run("git",&["-C",f.repo(),"rev-parse",&spec]))).unwrap();
        assert_eq!(str_at(entry,"blob"),observed.trim());
    }
    assert_eq!(f.suggest(),report);
    fs::write(&f.report,report.canonical()).unwrap();
    let receipt=json::parse(&ok(f.audit())).unwrap();
    assert_eq!(receipt.get("report_id").unwrap(),report.get("report_id").unwrap());
    assert!(!receipt.get("semantic_reparse_performed").unwrap().bool().unwrap());
}
#[test]
fn tampered_refactor_advice_and_index_relations_are_refused() {
    let f=Fixture::new("python");
    let mut report=f.suggest();
    let mut candidates=items(&report,"candidates").to_vec();
    candidates[0].entries_mut().unwrap().insert("action".into(),Value::string("forged advice"));
    report.entries_mut().unwrap().insert("candidates".into(),Value::array(candidates));
    fs::write(&f.report,report.canonical()).unwrap();
    denied(f.audit());
    fs::write(&f.report,f.suggest().canonical()).unwrap();
    Db::open(&f.db).unwrap().execute(
        "DELETE FROM dependency WHERE source_name=? AND target_name=?",
        &[excession::sqlite::Param::Text("pkg.a"),excession::sqlite::Param::Text("pkg.b")]).unwrap();
    denied(f.audit());
}
#[test]
fn refactor_requires_indexed_revision_and_unique_flags() {
    let f=Fixture::new("python");
    fs::write(f.repo.join("extra.txt"),"new commit\n").unwrap();
    ok(run("git",&["-C",f.repo(),"add","."]));
    ok(run("git",&["-C",f.repo(),"-c","user.name=Fixture","-c","user.email=fixture@example.invalid",
        "commit","-qm","next"]));
    assert_eq!(str_at(&f.suggest(),"source_commit"),f.commit);
    denied(cli(&["refactor","suggest","--db",f.db.to_str().unwrap(),
        "--name","fixture","--rev","HEAD"]));
    denied(cli(&["refactor","suggest","--db",f.db.to_str().unwrap(),
        "--name","fixture","--rev",&f.commit,"--rev",&f.commit]));
}
#[test]
fn scip_cross_language_references_are_unverified_and_source_bound() {
    let f=Fixture::new("scip");
    let index=scip_index();
    let report=json::parse(&ok(f.scip_suggest(&index))).unwrap();
    assert_eq!(str_at(&report,"schema"),"excession.scip-refactor.v1");
    assert_eq!(str_at(&report,"source_commit"),f.commit);
    assert_eq!(items(report.get("coverage").unwrap(),"languages"),
        &[Value::string("python"),Value::string("rust")]);
    let coverage=report.get("coverage").unwrap();
    assert_eq!(coverage.get("matched_cross_file_references").unwrap().integer().unwrap(),2);
    assert_eq!(coverage.get("unresolved_references").unwrap().integer().unwrap(),0);
    let candidates=items(&report,"candidates");
    assert_eq!(candidates.iter().filter(|c|str_at(c,"kind")=="reciprocal_symbol_references").count(),1);
    assert_eq!(candidates.iter().filter(|c|str_at(c,"kind")=="cross_language_symbol_boundary").count(),2);
    assert!(candidates.iter().all(|c|str_at(c,"status")=="UNVERIFIED_REFACTOR_HYPOTHESIS"));
    fs::write(&f.report,report.canonical()).unwrap();
    let receipt=json::parse(&ok(f.scip_audit())).unwrap();
    assert_eq!(receipt.get("report_id").unwrap(),report.get("report_id").unwrap());
    assert!(!receipt.get("indexer_independently_executed").unwrap().bool().unwrap());
    assert_eq!(json::parse(&ok(f.scip_suggest(&index))).unwrap(),report);
}
#[test]
fn scip_candidate_edges_require_exact_symbol_identity() {
    let f=Fixture::new("scip");
    let mut index=scip_index();
    let documents=edit(&mut index,"documents");
    let occurrences=edit(item(documents,1),"occurrences");
    edit(item(occurrences,1),"symbol").clone_from(&Value::string(
        "scip rust cargo different 0.1.0 api/emit()."));
    let report=json::parse(&ok(f.scip_suggest(&index))).unwrap();
    assert_eq!(report.get("coverage").unwrap().get("unresolved_references").unwrap().integer().unwrap(),1);
    let candidates=items(&report,"candidates");
    assert_eq!(candidates.iter().filter(|c|str_at(c,"kind")=="cross_language_symbol_boundary").count(),1);
    assert!(!candidates.iter().any(|c|str_at(c,"kind")=="reciprocal_symbol_references"));
}
#[test]
fn scip_forged_reports_paths_ranges_and_sources_are_refused() {
    let f=Fixture::new("scip");
    let index=scip_index();
    let mut report=json::parse(&ok(f.scip_suggest(&index))).unwrap();
    edit(edit(&mut report,"coverage"),"matched_cross_file_references")
        .clone_from(&Value::number(200));
    fs::write(&f.report,report.canonical()).unwrap();
    denied(f.scip_audit());
    let mut escaped=scip_index();
    edit(item(edit(&mut escaped,"documents"),0),"relative_path")
        .clone_from(&Value::string("../escape.rs"));
    denied(f.scip_suggest(&escaped));
    edit(item(edit(&mut escaped,"documents"),0),"relative_path")
        .clone_from(&Value::string("lib.rs"));
    edit(item(edit(item(edit(&mut escaped,"documents"),0),"occurrences"),0),"range")
        .clone_from(&Value::array([Value::number(90),Value::number(0),Value::number(1)]));
    denied(f.scip_suggest(&escaped));
    let mut textual=scip_index();
    item(edit(&mut textual,"documents"),0).entries_mut().unwrap()
        .insert("text".into(),Value::string("forged source"));
    denied(f.scip_suggest(&textual));
}
#[test]
fn changed_scip_index_invalidates_audit() {
    let f=Fixture::new("scip");
    let mut index=scip_index();
    let report=json::parse(&ok(f.scip_suggest(&index))).unwrap();
    fs::write(&f.report,report.canonical()).unwrap();
    edit(item(edit(item(edit(&mut index,"documents"),1),"occurrences"),1),"symbol")
        .clone_from(&Value::string("scip python pypi example 0.1.0 app/parse()."));
    fs::write(&f.index,index.canonical()).unwrap();
    denied(f.scip_audit());
}
