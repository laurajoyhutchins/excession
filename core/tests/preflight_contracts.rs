//! CLI and library preflight contracts migrated from Python.
//! These checks deliberately make no claim about Git source authentication.
use excession::json::{self, Value, obj};
use excession::preflight;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn seal(mut body: Value, field: &str) -> Value {
    let digest = body.sha256();
    body.entries_mut()
        .unwrap()
        .insert(field.into(), Value::string(digest));
    body
}
fn syntax_report() -> Value {
    let source = obj(&[
        ("source_path", Value::string("src/entry.rs")),
        ("source_blob", Value::string("a".repeat(40))),
        ("source_mode", Value::string("100644")),
        ("language", Value::string("rust")),
        ("status", Value::string("PARSED")),
        ("structure", Value::array([])),
        (
            "imports",
            Value::array([obj(&[
                ("source", Value::string("use crate::value;")),
                ("line", Value::number(1)),
                ("wildcard", Value::Bool(false)),
            ])]),
        ),
        ("diagnostics", Value::array([])),
    ]);
    seal(
        obj(&[
            ("schema", Value::string("excession.syntax-observations.v1")),
            ("parser", Value::string("tree-sitter-language-pack")),
            ("parser_version", Value::string("1.25.0")),
            ("completeness", Value::string("CONSERVATIVE_SYNTAX_ONLY")),
            ("source_commit", Value::string("b".repeat(40))),
            ("source_tree", Value::string("c".repeat(40))),
            ("roots", Value::array([Value::string("src")])),
            ("source_count", Value::number(1)),
            ("sources", Value::array([source])),
        ]),
        "report_id",
    )
}
fn resolution_report() -> Value {
    let edge = obj(&[
        ("source_path", Value::string("src/entry.rs")),
        ("source_blob", Value::string("a".repeat(40))),
        ("target_path", Value::string("src/value.rs")),
        ("target_blob", Value::string("d".repeat(40))),
        ("language", Value::string("rust")),
        ("specifier", Value::string("crate::value")),
        ("line", Value::number(1)),
        ("resolution", Value::string("RUST_CRATE_PATH_CANDIDATE")),
    ]);
    seal(
        obj(&[
            ("schema", Value::string("excession.syntax-dependencies.v1")),
            ("source_commit", Value::string("b".repeat(40))),
            ("source_tree", Value::string("c".repeat(40))),
            ("syntax_report_id", Value::string("e".repeat(64))),
            (
                "completeness",
                Value::string("CONSERVATIVE_CANDIDATES_ONLY"),
            ),
            ("candidate_count", Value::number(1)),
            ("unresolved_count", Value::number(0)),
            ("candidate_edges", Value::array([edge])),
            ("unresolved_imports", Value::array([])),
            ("skipped_sources", Value::array([])),
            ("metadata", Value::array([])),
        ]),
        "resolution_id",
    )
}
struct ReportFile(PathBuf);
impl ReportFile {
    fn new(value: &Value) -> Self {
        let path = std::env::temp_dir().join(format!(
            "excession-preflight-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, value.canonical()).unwrap();
        Self(path)
    }
}
impl Drop for ReportFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn invoke(command: &str, value: &Value) -> Output {
    let report = ReportFile::new(value);
    Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["syntax", command, "--report", report.0.to_str().unwrap()])
        .output()
        .unwrap()
}
fn accepted(command: &str, value: &Value) -> Value {
    let response = invoke(command, value);
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    json::parse(&response.stdout).unwrap()
}
fn refused(command: &str, value: &Value) {
    let response = invoke(command, value);
    assert!(!response.status.success(), "unvalidated report accepted");
    assert!(response.stdout.is_empty());
    assert!(String::from_utf8_lossy(&response.stderr).contains("REFUSED:"));
}
fn reseal(mut report: Value, id: &str) -> Value {
    report.entries_mut().unwrap().remove(id).expect("id exists");
    seal(report, id)
}
#[test]
fn syntax_preflight_is_shape_and_digest_only() {
    let report = syntax_report();
    let v = accepted("preflight", &report);
    assert_eq!(
        v.get("status").unwrap().str().unwrap(),
        "SHAPE_AND_DIGEST_ONLY"
    );
    assert_eq!(v.get("source_count").unwrap().integer().unwrap(), 1);
    assert_eq!(preflight::syntax(&report).unwrap(), v);
}
#[test]
fn resolution_preflight_is_shape_and_digest_only() {
    let report = resolution_report();
    let v = accepted("resolve-preflight", &report);
    assert_eq!(
        v.get("status").unwrap().str().unwrap(),
        "SHAPE_AND_DIGEST_ONLY"
    );
    assert_eq!(preflight::resolution(&report).unwrap(), v);
}
#[test]
fn mutation_invalidates_both_digests() {
    for (cmd, mut report, key) in [
        ("preflight", syntax_report(), "source_commit"),
        ("resolve-preflight", resolution_report(), "source_tree"),
    ] {
        report
            .entries_mut()
            .unwrap()
            .insert(key.into(), Value::string("f".repeat(40)));
        refused(cmd, &report);
    }
}
#[test]
fn self_consistent_forgery_remains_provisional() {
    let mut report = syntax_report();
    let mut sources = report.get("sources").unwrap().array_ref().unwrap().to_vec();
    sources[0]
        .entries_mut()
        .unwrap()
        .insert("source_blob".into(), Value::string("f".repeat(40)));
    report
        .entries_mut()
        .unwrap()
        .insert("sources".into(), Value::array(sources));
    let report = reseal(report, "report_id");
    assert_eq!(
        accepted("preflight", &report)
            .get("status")
            .unwrap()
            .str()
            .unwrap(),
        "SHAPE_AND_DIGEST_ONLY"
    );
}
#[test]
fn malformed_schema_is_refused_even_with_recomputed_digest() {
    for (cmd, mut report, key, value, id) in [
        (
            "preflight",
            syntax_report(),
            "source_count",
            Value::number(2),
            "report_id",
        ),
        (
            "preflight",
            syntax_report(),
            "unexpected",
            Value::string("evil"),
            "report_id",
        ),
        (
            "preflight",
            syntax_report(),
            "roots",
            Value::array([Value::string("../outside")]),
            "report_id",
        ),
        (
            "resolve-preflight",
            resolution_report(),
            "candidate_count",
            Value::number(5),
            "resolution_id",
        ),
        (
            "resolve-preflight",
            resolution_report(),
            "unknown",
            Value::Bool(true),
            "resolution_id",
        ),
    ] {
        report.entries_mut().unwrap().insert(key.into(), value);
        let sealed = reseal(report, id);
        refused(cmd, &sealed);
        assert!(if cmd == "preflight" {
            preflight::syntax(&sealed).is_err()
        } else {
            preflight::resolution(&sealed).is_err()
        });
    }
}
