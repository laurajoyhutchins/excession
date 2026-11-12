//! Pinned-Git syntax and candidate-resolution integration tests.
//! The optional grammar pack is deliberately not downloaded at test time.
use excession::json::{self, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn parser_installed() -> bool {
    // The real parser runs in Rust, not through a Python interpreter.
    // Only exercise full syntax fixtures when every required grammar is installed.
    [
        "rust",
        "go",
        "java",
        "c",
        "python",
        "typescript",
        "javascript",
    ]
    .iter()
    .all(|language| excession::native_syntax::has_parser(language))
}
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("spawn")
}
fn ok(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn refused(output: Output) {
    assert!(!output.status.success(), "expected a refusal");
    assert!(output.stdout.is_empty(), "refusal leaked success output");
    assert!(String::from_utf8_lossy(&output.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(args: &[&str]) -> Value {
    json::parse(&ok(cli(args))).unwrap()
}
fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).unwrap().str().unwrap()
}
fn items<'a>(record: &'a Value, key: &str) -> &'a [Value] {
    record.get(key).unwrap().array_ref().unwrap()
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
    fn new(files: &[(&str, &str)]) -> Self {
        let temp = std::env::temp_dir().join(format!(
            "excession-syntax-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = temp.join("git");
        fs::create_dir_all(&root).unwrap();
        let repo = root.to_str().unwrap();
        ok(run("git", &["-C", repo, "init", "-q", "-b", "main"]));
        ok(run(
            "git",
            &["-C", repo, "config", "user.name", "Syntax fixtures"],
        ));
        ok(run(
            "git",
            &["-C", repo, "config", "user.email", "syntax@example.invalid"],
        ));
        for (path, source) in files {
            let full = root.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, source).unwrap();
        }
        ok(run("git", &["-C", repo, "add", "."]));
        ok(run(
            "git",
            &["-C", repo, "commit", "-qm", "parser fixtures"],
        ));
        let commit = String::from_utf8(ok(run("git", &["-C", repo, "rev-parse", "HEAD"])))
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
    fn scan(&self) -> Value {
        data(&[
            "syntax",
            "scan",
            "--repo",
            self.path(),
            "--rev",
            &self.commit,
            "--roots",
            "src",
        ])
    }
    fn rev_blob(&self, path: &str) -> String {
        let spec = format!("{}:{path}", self.commit);
        String::from_utf8(ok(run("git", &["-C", self.path(), "rev-parse", &spec])))
            .unwrap()
            .trim()
            .to_owned()
    }
}
fn parse_fixtures() -> Repo {
    Repo::new(&[
        ("src/main.rs", "pub fn answer() -> u32 { 42 }\n"),
        (
            "src/main.go",
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"hi\") }\n",
        ),
        (
            "src/Main.java",
            "class Main { static int answer() { return 42; } }\n",
        ),
        (
            "src/main.c",
            "#include <stdio.h>\nint answer(void) { return 42; }\n",
        ),
        ("src/ignored.bin", "arbitrary"),
    ])
}
fn resolve_fixtures() -> Repo {
    Repo::new(&[
        ("go.mod", "module example.org/test\n\ngo 1.23\n"),
        (
            "src/go/main.go",
            "package goapp\nimport \"example.org/test/src/gohelper\"\nfunc answer() int { return gohelper.Answer }\n",
        ),
        (
            "src/gohelper/helper.go",
            "package gohelper\nconst Answer = 42\n",
        ),
        (
            "src/gohelper/helper_test.go",
            "package gohelper\nfunc TestUnused() {}\n",
        ),
        (
            "src/main.rs",
            "mod widget;\nuse crate::widget::answer;\nfn main() { let _ = answer(); }\n",
        ),
        ("src/widget.rs", "pub fn answer() -> u32 { 42 }\n"),
        (
            "src/ui.ts",
            "import {value} from './util';\nimport foo from 'external-package';\nexport const out=value;\n",
        ),
        ("src/util.ts", "export const value=42;\n"),
        ("src/ambiguous.ts", "export const answer=2;\n"),
        ("src/ambiguous.js", "export const answer=3;\n"),
        ("src/ambiguous_user.ts", "import './ambiguous';\n"),
        ("src/pkg/__init__.py", ""),
        ("src/pkg/entry.py", "import pkg.util\n"),
        ("src/pkg/util.py", "answer = 42\n"),
    ])
}
fn scanned_resolution(repo: &Repo) -> (PathBuf, Value, PathBuf) {
    let syntax = repo.scan();
    let syntax_path = repo.save("syntax.json", &syntax);
    let report = data(&[
        "syntax",
        "resolve",
        "--repo",
        repo.path(),
        "--report",
        syntax_path.to_str().unwrap(),
    ]);
    let resolution_path = repo.save("resolution.json", &report);
    (syntax_path, report, resolution_path)
}
#[test]
fn multilang_scan_is_bound_to_tracked_git_blobs() {
    if !parser_installed() {
        eprintln!("optional grammar pack unavailable");
        return;
    }
    let repo = parse_fixtures();
    let report = repo.scan();
    assert_eq!(text(&report, "schema"), "excession.syntax-observations.v1");
    assert_eq!(text(&report, "source_commit"), repo.commit);
    assert_eq!(report.get("source_count").unwrap().integer().unwrap(), 4);
    assert_eq!(text(&report, "report_id").len(), 64);
    let records = items(&report, "sources");
    assert_eq!(records.len(), 4);
    for expected in ["src/main.rs", "src/main.go", "src/Main.java", "src/main.c"] {
        let item = records
            .iter()
            .find(|v| text(v, "source_path") == expected)
            .unwrap();
        assert_eq!(text(item, "source_blob"), repo.rev_blob(expected));
        assert_eq!(text(item, "status"), "PARSED");
        assert!(!text(item, "language").is_empty());
    }
    assert!(
        !items(
            records
                .iter()
                .find(|v| text(v, "source_path") == "src/main.rs")
                .unwrap(),
            "structure"
        )
        .is_empty()
    );
    assert!(
        !items(
            records
                .iter()
                .find(|v| text(v, "source_path") == "src/main.go")
                .unwrap(),
            "imports"
        )
        .is_empty()
    );
    let report_path = repo.save("syntax.json", &report);
    let result = data(&[
        "syntax",
        "audit",
        "--repo",
        repo.path(),
        "--report",
        report_path.to_str().unwrap(),
    ]);
    assert_eq!(text(&result, "status"), "REPARSED_MATCH");
    assert_eq!(
        result.get("report_id").unwrap(),
        report.get("report_id").unwrap()
    );
    let mut changed = report;
    let mut sources = items(&changed, "sources").to_vec();
    sources[0]
        .entries_mut()
        .unwrap()
        .insert("source_blob".into(), Value::string("0".repeat(40)));
    changed
        .entries_mut()
        .unwrap()
        .insert("sources".into(), Value::array(sources));
    fs::write(&report_path, changed.canonical()).unwrap();
    refused(cli(&[
        "syntax",
        "audit",
        "--repo",
        repo.path(),
        "--report",
        report_path.to_str().unwrap(),
    ]));
}
#[test]
fn scan_refuses_dirty_unpinned_checkout() {
    if !parser_installed() {
        return;
    }
    let repo = parse_fixtures();
    repo.scan();
    fs::write(repo.root.join("src/untracked.rs"), "fn unknown() {}").unwrap();
    refused(cli(&[
        "syntax",
        "scan",
        "--repo",
        repo.path(),
        "--roots",
        "src",
    ]));
}
#[test]
fn dependency_candidates_replay_exact_tracked_sources() {
    if !parser_installed() {
        return;
    }
    let repo = resolve_fixtures();
    let (syntax, mut report, resolution) = scanned_resolution(&repo);
    assert_eq!(text(&report, "schema"), "excession.syntax-dependencies.v1");
    assert_eq!(text(&report, "source_commit"), repo.commit);
    assert_eq!(
        text(&report, "completeness"),
        "CONSERVATIVE_CANDIDATES_ONLY"
    );
    let edges = items(&report, "candidate_edges");
    assert_eq!(
        report.get("candidate_count").unwrap().integer().unwrap() as usize,
        edges.len()
    );
    for (source, target) in [
        ("src/ui.ts", "src/util.ts"),
        ("src/pkg/entry.py", "src/pkg/util.py"),
        ("src/go/main.go", "src/gohelper/helper.go"),
    ] {
        assert!(
            edges
                .iter()
                .any(|e| text(e, "source_path") == source && text(e, "target_path") == target),
            "missing {source} -> {target}"
        );
    }
    assert!(
        !edges
            .iter()
            .any(|e| text(e, "target_path") == "src/gohelper/helper_test.go")
    );
    assert!(
        !edges
            .iter()
            .any(|e| text(e, "source_path") == "src/ambiguous_user.ts")
    );
    assert!(!edges.iter().any(
        |e| text(e, "source_path").ends_with(".rs") || text(e, "target_path").ends_with(".rs")
    ));
    assert!(
        items(&report, "skipped_sources")
            .iter()
            .any(|e| text(e, "source_path") == "src/main.rs"
                && text(e, "status") == "NO_RESOLVER_FOR_LANGUAGE")
    );
    let unresolved = items(&report, "unresolved_imports");
    for (source, why) in [
        ("src/ambiguous_user.ts", "AMBIGUOUS_TRACKED_TARGETS"),
        ("src/ui.ts", "BARE_OR_ALIAS_IMPORT"),
    ] {
        assert!(
            unresolved
                .iter()
                .any(|e| text(e, "source_path") == source && text(e, "reason") == why)
        );
    }
    for edge in edges {
        for side in ["source", "target"] {
            let path = text(edge, &format!("{side}_path"));
            assert_eq!(text(edge, &format!("{side}_blob")), repo.rev_blob(path));
        }
    }
    let audited = data(&[
        "syntax",
        "resolve-audit",
        "--repo",
        repo.path(),
        "--syntax-report",
        syntax.to_str().unwrap(),
        "--report",
        resolution.to_str().unwrap(),
    ]);
    assert_eq!(text(&audited, "status"), "REGENERATED_MATCH");
    let mut changed_edges = items(&report, "candidate_edges").to_vec();
    changed_edges[0]
        .entries_mut()
        .unwrap()
        .insert("target_blob".into(), Value::string("0".repeat(40)));
    report
        .entries_mut()
        .unwrap()
        .insert("candidate_edges".into(), Value::array(changed_edges));
    fs::write(&resolution, report.canonical()).unwrap();
    refused(cli(&[
        "syntax",
        "resolve-audit",
        "--repo",
        repo.path(),
        "--syntax-report",
        syntax.to_str().unwrap(),
        "--report",
        resolution.to_str().unwrap(),
    ]));
}
#[test]
fn forged_syntax_cannot_generate_dependency_candidates() {
    if !parser_installed() {
        return;
    }
    let repo = resolve_fixtures();
    let mut source = repo.scan();
    let mut entries = items(&source, "sources").to_vec();
    entries[0]
        .entries_mut()
        .unwrap()
        .insert("source_blob".into(), Value::string("0".repeat(40)));
    source
        .entries_mut()
        .unwrap()
        .insert("sources".into(), Value::array(entries));
    let syntax = repo.save("forged-syntax.json", &source);
    refused(cli(&[
        "syntax",
        "resolve",
        "--repo",
        repo.path(),
        "--report",
        syntax.to_str().unwrap(),
    ]));
}
#[test]
fn changed_checkout_refuses_candidate_replay() {
    if !parser_installed() {
        return;
    }
    let repo = resolve_fixtures();
    let (syntax, _, _) = scanned_resolution(&repo);
    fs::write(repo.root.join("src/untracked.rs"), "const X: u8=1;").unwrap();
    refused(cli(&[
        "syntax",
        "resolve",
        "--repo",
        repo.path(),
        "--report",
        syntax.to_str().unwrap(),
    ]));
}
