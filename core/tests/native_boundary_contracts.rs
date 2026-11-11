//! Rust-owned CLI contracts for conservative native input declarations.
//! The grammar adapter is Python-specific; the comparison/replay contract is not.
use excession::json::{self, Value, obj};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(program: &str, args: &[&str]) -> Output {
    Command::new(program).args(args).output().unwrap()
}
fn ok(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn denied(output: Output) {
    assert!(!output.status.success(), "unexpected acceptance");
    assert!(String::from_utf8_lossy(&output.stderr).contains("REFUSED:"));
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn data(args: &[&str]) -> Value {
    json::parse(&ok(cli(args))).unwrap()
}
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).unwrap().str().unwrap()
}
fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).unwrap().array_ref().unwrap()
}
fn grammar_available() -> bool {
    run("python3", &["-c", "import tree_sitter_language_pack"])
        .status
        .success()
}
fn declaration(
    language: &str,
    subject: &str,
    source_paths: &[&str],
    config_paths: &[&str],
) -> Value {
    let mut paths = source_paths
        .iter()
        .map(|p| (*p, "source"))
        .chain(config_paths.iter().map(|p| (*p, "configuration")))
        .collect::<Vec<_>>();
    paths.sort();
    obj(&[
        ("schema", Value::string("excession.input-declaration.v1")),
        ("coverage", Value::string("DECLARED_INPUTS_ONLY")),
        ("language", Value::string(language)),
        ("subject", Value::string(subject)),
        (
            "inputs",
            Value::array(paths.iter().map(|(path, role)| {
                obj(&[
                    ("path", Value::string(*path)),
                    ("role", Value::string(*role)),
                ])
            })),
        ),
        (
            "assumptions",
            obj(&[
                (
                    "dependency-discovery",
                    Value::string("excession-toolchain-native-v1"),
                ),
                ("toolchain-version", Value::string("fixture-resolver-1")),
                ("toolchain-sha256", Value::string("f".repeat(64))),
                ("platform", Value::string("fixture-linux")),
            ]),
        ),
    ])
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    rev: String,
    syntax: PathBuf,
    resolution: PathBuf,
    declaration: PathBuf,
    boundary: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-native-compare-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let path = repo.to_str().unwrap();
        ok(run("git", &["-C", path, "init", "-q", "-b", "main"]));
        ok(run(
            "git",
            &["-C", path, "config", "user.name", "Excession fixture"],
        ));
        ok(run(
            "git",
            &[
                "-C",
                path,
                "config",
                "user.email",
                "fixture@example.invalid",
            ],
        ));
        let files = [
            ("go.mod", "module example.org/local\n\ngo 1.23\n"),
            (
                "Cargo.toml",
                "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            (
                "tsconfig.json",
                "{\"compilerOptions\": {\"module\": \"esnext\"}}\n",
            ),
            (
                "src/go/main.go",
                "package goapp\nimport \"example.org/local/src/lib\"\nvar X=lib.Answer\n",
            ),
            ("src/lib/lib.go", "package lib\nconst Answer=42\n"),
            (
                "src/main.rs",
                "mod math;\nuse crate::math::value;\nfn main() { let _ = value(); }\n",
            ),
            ("src/math.rs", "pub fn value() -> u32 { 42 }\n"),
            (
                "src/ui.ts",
                "import { value } from './util';\nexport const answer = value;\n",
            ),
            ("src/util.ts", "export const value = 42;\n"),
            (
                "src/app.js",
                "import { value } from './helper.js';\nconsole.log(value);\n",
            ),
            ("src/helper.js", "export const value = 42;\n"),
            ("src/outside.py", "answer = 42\n"),
        ];
        for (filename, content) in files {
            let full = repo.join(filename);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, content).unwrap();
        }
        ok(run("git", &["-C", path, "add", "."]));
        ok(run("git", &["-C", path, "commit", "-qm", "replay inputs"]));
        let rev = String::from_utf8(ok(run("git", &["-C", path, "rev-parse", "HEAD"])))
            .unwrap()
            .trim()
            .to_owned();
        let syntax = root.join("syntax.json");
        let resolution = root.join("resolution.json");
        let declaration = root.join("declaration.json");
        let boundary = root.join("boundary.json");
        fs::write(
            &syntax,
            ok(cli(&[
                "syntax", "scan", "--repo", path, "--rev", &rev, "--roots", "src",
            ])),
        )
        .unwrap();
        fs::write(
            &resolution,
            ok(cli(&[
                "syntax",
                "resolve",
                "--repo",
                path,
                "--report",
                syntax.to_str().unwrap(),
            ])),
        )
        .unwrap();
        Self {
            root,
            repo,
            rev,
            syntax,
            resolution,
            declaration,
            boundary,
        }
    }
    fn compare_args(&self) -> Vec<&str> {
        vec![
            "syntax",
            "native-compare",
            "--repo",
            self.repo.to_str().unwrap(),
            "--syntax-report",
            self.syntax.to_str().unwrap(),
            "--resolution-report",
            self.resolution.to_str().unwrap(),
            "--declaration",
            self.declaration.to_str().unwrap(),
        ]
    }
    fn audit_args(&self) -> Vec<&str> {
        vec![
            "syntax",
            "native-audit",
            "--repo",
            self.repo.to_str().unwrap(),
            "--syntax-report",
            self.syntax.to_str().unwrap(),
            "--resolution-report",
            self.resolution.to_str().unwrap(),
            "--declaration",
            self.declaration.to_str().unwrap(),
            "--report",
            self.boundary.to_str().unwrap(),
        ]
    }
    fn evaluate(&self, declaration: &Value) -> Value {
        fs::write(&self.declaration, declaration.canonical()).unwrap();
        let report = ok(cli(&self.compare_args()));
        fs::write(&self.boundary, &report).unwrap();
        json::parse(&report).unwrap()
    }
}
#[test]
fn static_native_scope_matches_four_languages_without_claiming_attestation() {
    if !grammar_available() {
        return;
    }
    let f = Fixture::new();
    let scenarios = [
        (
            "go",
            "src/go/main.go",
            &["src/go/main.go", "src/lib/lib.go"][..],
            &["go.mod"][..],
            "src/lib/lib.go",
        ),
        (
            "rust",
            "src/main.rs",
            &["src/main.rs", "src/math.rs"][..],
            &["Cargo.toml"][..],
            "src/math.rs",
        ),
        (
            "typescript",
            "src/ui.ts",
            &["src/ui.ts", "src/util.ts"][..],
            &["tsconfig.json"][..],
            "src/util.ts",
        ),
        (
            "javascript",
            "src/app.js",
            &["src/app.js", "src/helper.js"][..],
            &[][..],
            "src/helper.js",
        ),
    ];
    for (language, subject, inputs, config, target) in scenarios {
        let report = f.evaluate(&declaration(language, subject, inputs, config));
        assert_eq!(
            s(&report, "status"),
            "NO_STATIC_NATIVE_DISAGREEMENT_OBSERVED"
        );
        assert_eq!(s(&report, "provenance"), "UNATTESTED_NATIVE_DECLARATION");
        assert_eq!(s(&report, "source_commit"), f.rev);
        if language == "rust" {
            assert!(arr(&report, "matching_edges").is_empty());
            assert!(arr(&report, "unresolved_imports").is_empty());
            assert!(arr(&report, "native_only_sources").contains(&Value::string("src/math.rs")));
        } else {
            assert!(
                arr(&report, "matching_edges")
                    .iter()
                    .any(|e| s(e, "source_path") == subject && s(e, "target_path") == target)
            );
        }
        assert!(arr(&report, "static_only_edges").is_empty());
        for entry in arr(&report, "pinned_inputs") {
            let spec = format!("{}:{}", f.rev, s(entry, "path"));
            let blob = String::from_utf8(ok(run(
                "git",
                &["-C", f.repo.to_str().unwrap(), "rev-parse", &spec],
            )))
            .unwrap();
            assert_eq!(s(entry, "blob"), blob.trim());
        }
        let audited = data(&f.audit_args());
        assert_eq!(s(&audited, "status"), "DECLARATION_AND_GIT_REPLAY_MATCH");
    }
}
#[test]
fn incomplete_native_input_scope_is_reviewable_not_a_proven_bug() {
    if !grammar_available() {
        return;
    }
    let f = Fixture::new();
    let report = f.evaluate(&declaration(
        "go",
        "src/go/main.go",
        &["src/go/main.go"],
        &["go.mod"],
    ));
    assert_eq!(
        s(&report, "status"),
        "REVIEW_STATIC_NATIVE_SCOPE_DIFFERENCE"
    );
    assert_eq!(s(&report, "coverage"), "SCOPED_COMPARISON_ONLY");
    assert_eq!(arr(&report, "static_only_edges").len(), 1);
    assert_eq!(
        s(&arr(&report, "static_only_edges")[0], "target_path"),
        "src/lib/lib.go"
    );
}
#[test]
fn untracked_declared_sources_are_refused_before_graph_output() {
    if !grammar_available() {
        return;
    }
    let f = Fixture::new();
    fs::write(
        &f.declaration,
        declaration(
            "go",
            "src/go/main.go",
            &["src/go/main.go", "src/ghost.go"],
            &["go.mod"],
        )
        .canonical(),
    )
    .unwrap();
    denied(cli(&f.compare_args()));
}
#[test]
fn native_declaration_shape_preflight_needs_no_git_or_grammars() {
    let tmp = std::env::temp_dir().join(format!(
        "excession-native-preflight-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut report = declaration("rust", "src/main.rs", &["src/main.rs"], &[]);
    fs::write(&tmp, report.canonical()).unwrap();
    assert_eq!(
        s(
            &data(&[
                "syntax",
                "native-preflight",
                "--declaration",
                tmp.to_str().unwrap()
            ]),
            "status"
        ),
        "DECLARATION_SHAPE_ONLY"
    );
    report
        .entries_mut()
        .unwrap()
        .get_mut("assumptions")
        .unwrap()
        .entries_mut()
        .unwrap()
        .insert("toolchain-sha256".into(), Value::string("malformed"));
    fs::write(&tmp, report.canonical()).unwrap();
    denied(cli(&[
        "syntax",
        "native-preflight",
        "--declaration",
        tmp.to_str().unwrap(),
    ]));
    let _ = fs::remove_file(tmp);
}
#[test]
fn native_scope_audit_detects_changed_report_bytes() {
    if !grammar_available() {
        return;
    }
    let f = Fixture::new();
    let mut report = f.evaluate(&declaration(
        "rust",
        "src/main.rs",
        &["src/main.rs", "src/math.rs"],
        &["Cargo.toml"],
    ));
    let mut edges = arr(&report, "matching_edges").to_vec();
    edges[0]
        .entries_mut()
        .unwrap()
        .insert("target_blob".into(), Value::string("0".repeat(40)));
    report
        .entries_mut()
        .unwrap()
        .insert("matching_edges".into(), Value::array(edges));
    fs::write(&f.boundary, report.canonical()).unwrap();
    denied(cli(&f.audit_args()));
}
