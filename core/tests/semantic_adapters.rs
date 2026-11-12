//! Shared semantic observations emitted by language-specific Rust resolvers.
//! Real pinned-Git fixtures; assertions establish candidates, not graph completeness.
use excession::json::{self, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run(binary: &str, args: &[&str]) -> Output {
    Command::new(binary).args(args).output().expect("spawn")
}

fn checked(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn refused(output: Output) {
    assert!(!output.status.success(), "expected refusal");
    assert!(output.stdout.is_empty(), "refusal emitted success output");
    assert!(String::from_utf8_lossy(&output.stderr).contains("REFUSED:"));
}

fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}

fn data(args: &[&str]) -> Value {
    json::parse(&checked(cli(args))).expect("valid JSON")
}

fn parser_installed() -> bool {
    Command::new("python3")
        .args(["-c", "import tree_sitter_language_pack"])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn str_field<'a>(item: &'a Value, key: &str) -> &'a str {
    item.get(key).unwrap().str().unwrap()
}

fn arr<'a>(item: &'a Value, key: &str) -> &'a [Value] {
    item.get(key).unwrap().array_ref().unwrap()
}

struct Repo {
    temp: PathBuf,
    root: PathBuf,
    commit: String,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp);
    }
}

impl Repo {
    fn new() -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-semantic-adapters-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("source");
        fs::create_dir_all(&root).unwrap();
        let repo = root.to_str().unwrap();
        checked(run("git", &["-C", repo, "init", "-q", "-b", "main"]));
        checked(run(
            "git",
            &["-C", repo, "config", "user.name", "Excession fixtures"],
        ));
        checked(run(
            "git",
            &[
                "-C",
                repo,
                "config",
                "user.email",
                "fixture@example.invalid",
            ],
        ));
        let files = [
            ("go.mod", "module example.org/local\n\ngo 1.23\n"),
            ("src/pkg/__init__.py", "# package\n"),
            (
                "src/pkg/main.py",
                "from .util import value\nimport missing_outside_pkg\nanswer = value\n",
            ),
            ("src/pkg/util.py", "value = 42\n"),
            (
                "src/main.rs",
                "mod math;\nuse crate::math::value;\nfn main() { let _ = value(); }\n",
            ),
            ("src/math.rs", "pub fn value() -> u32 { 42 }\n"),
            (
                "src/go/main.go",
                "package main\nimport \"example.org/local/src/lib\"\nfunc main() { _ = lib.Answer }\n",
            ),
            ("src/lib/lib.go", "package lib\nconst Answer = 42\n"),
            (
                "src/ui.ts",
                "import { value } from './util';\nimport { external } from '@pkg/alias';\nexport const answer = value;\n",
            ),
            ("src/util.ts", "export const value = 42;\n"),
            (
                "src/app.js",
                "import { value } from './helper.js';\nconsole.log(value);\n",
            ),
            ("src/helper.js", "export const value = 42;\n"),
            ("src/unsupported.c", "int answer(void) { return 42; }\n"),
        ];
        for (path, content) in files {
            let target = root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
        checked(run("git", &["-C", repo, "add", "."]));
        checked(run("git", &["-C", repo, "commit", "-qm", "source fixture"]));
        let commit = String::from_utf8(checked(run("git", &["-C", repo, "rev-parse", "HEAD"])))
            .unwrap()
            .trim()
            .to_owned();
        Self { temp, root, commit }
    }

    fn path(&self) -> &str {
        self.root.to_str().unwrap()
    }

    fn save(&self, name: &str, content: &Value) -> PathBuf {
        let path = self.temp.join(name);
        fs::write(&path, content.canonical()).unwrap();
        path
    }

    fn blob(&self, path: &str) -> String {
        let spec = format!("{}:{path}", self.commit);
        String::from_utf8(checked(run(
            "git",
            &["-C", self.path(), "rev-parse", &spec],
        )))
        .unwrap()
        .trim()
        .to_owned()
    }

    fn semantic(&self) -> Value {
        data(&[
            "semantic",
            "scan",
            "--repo",
            self.path(),
            "--rev",
            &self.commit,
            "--roots",
            "src",
        ])
    }
}

#[test]
fn direct_multi_language_adapter_records_bind_git_sources() {
    if !parser_installed() {
        eprintln!("optional grammar pack unavailable");
        return;
    }
    let f = Repo::new();
    let report = f.semantic();
    assert_eq!(
        str_field(&report, "schema"),
        "excession.semantic-observations.v1"
    );
    assert_eq!(
        str_field(&report, "completeness"),
        "PARTIAL_CANDIDATES_ONLY"
    );
    assert_eq!(str_field(&report, "source_commit"), f.commit);
    for (source, target, language) in [
        ("src/pkg/main.py", "src/pkg/util.py", "python"),
        ("src/go/main.go", "src/lib/lib.go", "go"),
        ("src/ui.ts", "src/util.ts", "typescript"),
        ("src/app.js", "src/helper.js", "javascript"),
    ] {
        let item = arr(&report, "records").iter().find(|r| {
            str_field(r.get("subject").unwrap(), "path") == source
                && r.get("target")
                    .is_ok_and(|entry| str_field(entry, "path") == target)
        });
        let item = item.unwrap_or_else(|| panic!("missing {language}: {source} -> {target}"));
        assert_eq!(
            str_field(item.get("subject").unwrap(), "language"),
            language
        );
        assert_eq!(str_field(item, "kind"), "IMPORTS");
        assert_eq!(str_field(item, "condition_evaluation"), "NOT_EVALUATED");
        assert_eq!(
            str_field(item.get("assessment").unwrap(), "status"),
            "CANDIDATE"
        );
    }
    assert!(
        arr(&report, "records")
            .iter()
            .all(|record| { str_field(record.get("subject").unwrap(), "language") != "rust" })
    );
    assert!(arr(&report, "gaps").iter().any(|gap| {
        str_field(gap, "source_path") == "src/main.rs"
            && str_field(gap, "status") == "NO_RESOLVER_FOR_LANGUAGE"
    }));
    for record in arr(&report, "records") {
        let subject = record.get("subject").unwrap();
        assert_eq!(
            str_field(subject, "blob"),
            f.blob(str_field(subject, "path"))
        );
        if let Ok(target) = record.get("target") {
            assert_eq!(str_field(target, "blob"), f.blob(str_field(target, "path")));
        } else {
            assert_eq!(
                str_field(record.get("assessment").unwrap(), "status"),
                "UNRESOLVED"
            );
        }
    }
    assert!(arr(&report, "gaps").iter().any(|gap| {
        str_field(gap, "source_path") == "src/unsupported.c"
            && str_field(gap, "status") == "NO_RESOLVER_FOR_LANGUAGE"
    }));
    assert!(arr(&report, "records").iter().any(|record| {
        str_field(record, "specifier") == "@pkg/alias"
            && str_field(record.get("assessment").unwrap(), "status") == "UNRESOLVED"
    }));
}

#[test]
fn direct_and_replayed_emission_are_exactly_identical() {
    if !parser_installed() {
        return;
    }
    let f = Repo::new();
    let direct = f.semantic();
    let syntax = data(&[
        "syntax",
        "scan",
        "--repo",
        f.path(),
        "--roots",
        "src",
        "--rev",
        &f.commit,
    ]);
    let syntax_path = f.save("syntax.json", &syntax);
    let resolution = data(&[
        "syntax",
        "resolve",
        "--repo",
        f.path(),
        "--report",
        syntax_path.to_str().unwrap(),
    ]);
    let resolution_path = f.save("resolution.json", &resolution);
    let from_syntax = data(&[
        "semantic",
        "resolve",
        "--repo",
        f.path(),
        "--syntax-report",
        syntax_path.to_str().unwrap(),
    ]);
    let from_saved = data(&[
        "semantic",
        "export",
        "--repo",
        f.path(),
        "--syntax-report",
        syntax_path.to_str().unwrap(),
        "--resolution-report",
        resolution_path.to_str().unwrap(),
    ]);
    assert_eq!(direct, from_syntax);
    assert_eq!(direct, from_saved);
    let report_path = f.save("semantic.json", &direct);
    let audited = data(&[
        "semantic",
        "audit",
        "--repo",
        f.path(),
        "--roots",
        "src",
        "--report",
        report_path.to_str().unwrap(),
    ]);
    assert_eq!(str_field(&audited, "status"), "REGENERATED_MATCH");
    assert_eq!(
        str_field(&audited, "report_id"),
        str_field(&direct, "report_id")
    );
}

#[test]
fn semantic_audit_rejects_self_consistent_forgery_and_wrong_scope() {
    if !parser_installed() {
        return;
    }
    let f = Repo::new();
    let mut forged = f.semantic();
    let mut records = arr(&forged, "records").to_vec();
    records[0]
        .entries_mut()
        .unwrap()
        .insert("specifier".into(), Value::string("forged"));
    let mut record_body = records[0].entries().unwrap().clone();
    record_body.remove("observation_id");
    let id = Value::Object(record_body).sha256();
    records[0]
        .entries_mut()
        .unwrap()
        .insert("observation_id".into(), Value::string(id));
    forged
        .entries_mut()
        .unwrap()
        .insert("records".into(), Value::array(records));
    let mut body = forged.entries().unwrap().clone();
    body.remove("report_id");
    let hash = Value::Object(body).sha256();
    forged
        .entries_mut()
        .unwrap()
        .insert("report_id".into(), Value::string(hash));
    let saved = f.save("forged.json", &forged);
    refused(cli(&[
        "semantic",
        "audit",
        "--repo",
        f.path(),
        "--roots",
        "src",
        "--report",
        saved.to_str().unwrap(),
    ]));

    let valid = f.save("valid.json", &f.semantic());
    refused(cli(&[
        "semantic",
        "audit",
        "--repo",
        f.path(),
        "--roots",
        "src/pkg",
        "--report",
        valid.to_str().unwrap(),
    ]));
}

#[test]
fn semantic_scan_refuses_dirty_checkout() {
    if !parser_installed() {
        return;
    }
    let f = Repo::new();
    fs::write(f.root.join("src/untracked.rs"), "fn extra() {}\n").unwrap();
    refused(cli(&[
        "semantic",
        "scan",
        "--repo",
        f.path(),
        "--roots",
        "src",
        "--rev",
        &f.commit,
    ]));
}

#[test]
fn historical_index_and_semantic_name_exports_agree() {
    use excession::{index, semantic};
    for (path, roots, base) in [
        ("pkg/a.py", &["pkg"][..], Some("pkg")),
        ("pkg/sub/__init__.py", &["pkg"][..], Some("pkg")),
        ("node_modules/vendor.js", &["."][..], None),
        ("other/tool.ts", &["pkg"][..], Some("pkg")),
    ] {
        let expected = semantic::module_name_with_import_base(path, roots, base);
        assert_eq!(
            index::module_name_with_import_base(path, roots, base),
            expected,
            "compatibility export for {path}",
        );
    }
}
