//! Real compiler and objcopy feedback over isolated fixture objects.
use excession::json::Value;
use excession::sqlite::Db;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn run(binary: &str, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .output()
        .expect("tool exists")
}
fn success(out: Output) -> Vec<u8> {
    assert!(
        out.status.success(),
        "tool refused: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn cli(args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_excession"), args)
}
fn json(out: Output) -> Value {
    excession::json::parse(&success(out)).unwrap()
}
fn fail(out: Output) {
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED:"));
}
fn field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).unwrap().str().unwrap()
}
struct Fixture {
    root: PathBuf,
    target: PathBuf,
    candidate: PathBuf,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "excession-reconstruct-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let original = root.join("original.c");
        let target = root.join("original.o");
        let candidate = root.join("candidate.c");
        let db = root.join("evidence.sqlite");
        fs::write(&original, "int evaluate(int x) { return x * 3; }\n").unwrap();
        fs::write(&candidate, "int evaluate(int x) { return x + x + x; }\n").unwrap();
        success(run(
            "cc",
            &[
                "-O2",
                "-fno-stack-protector",
                "-fno-asynchronous-unwind-tables",
                "-fno-ident",
                "-c",
                "-o",
                target.to_str().unwrap(),
                original.to_str().unwrap(),
            ],
        ));
        Self {
            root,
            target,
            candidate,
            db,
        }
    }
    fn target(&self) -> &str {
        self.target.to_str().unwrap()
    }
    fn candidate(&self) -> &str {
        self.candidate.to_str().unwrap()
    }
    fn db(&self) -> &str {
        self.db.to_str().unwrap()
    }
    fn evaluate(&self, consent: &str) -> Output {
        cli(&[
            "reconstruct",
            "evaluate",
            "--db",
            self.db(),
            "--target",
            self.target(),
            "--candidate",
            self.candidate(),
            "--trusted-local",
            consent,
        ])
    }
    fn history(&self) -> Output {
        cli(&[
            "reconstruct",
            "history",
            "--db",
            self.db(),
            "--target",
            self.target(),
        ])
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn real_compiler_feedback_distinguishes_match_and_mismatch_and_records_history() {
    let f = Fixture::new();
    let same = json(f.evaluate("true"));
    assert_eq!(field(&same, "text_relation"), "IDENTICAL_TEXT_BYTES");
    assert_eq!(same.get("mismatch_count").unwrap().integer().unwrap(), 0);
    assert_eq!(field(&same, "behavioral_equivalence"), "NOT_ESTABLISHED");
    assert_eq!(field(&same, "source_recovery"), "NOT_ESTABLISHED");
    assert_eq!(field(&same, "relocation_equivalence"), "NOT_ESTABLISHED");
    assert!(
        !same
            .get("build_execution_attested")
            .unwrap()
            .bool()
            .unwrap()
    );
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../contracts/evidence-core/reconstruction-attempt.v1.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let value: serde_json::Value = serde_json::from_str(&same.canonical()).unwrap();
    assert!(validator.is_valid(&value));

    let repeated = json(f.evaluate("true"));
    assert_eq!(field(&repeated, "attempt_id"), field(&same, "attempt_id"));

    fs::write(&f.candidate, "int evaluate(int x) { return x * 4; }\n").unwrap();
    let different = json(f.evaluate("true"));
    assert_eq!(field(&different, "text_relation"), "DIFFERENT_TEXT_BYTES");
    assert!(different.get("mismatch_count").unwrap().integer().unwrap() > 0);
    assert!(
        !different
            .get("mismatch_examples")
            .unwrap()
            .array_ref()
            .unwrap()
            .is_empty()
    );
    let history = json(f.history());
    assert_eq!(
        history
            .get("stored_attempt_count")
            .unwrap()
            .integer()
            .unwrap(),
        2
    );
    let attempts = history.get("attempts").unwrap().array_ref().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        field(&attempts[0], "attempt_id"),
        field(&different, "attempt_id")
    );
    assert_eq!(
        field(&attempts[1], "attempt_id"),
        field(&same, "attempt_id")
    );
    assert_eq!(field(&history, "execution_attestation"), "NOT_ESTABLISHED");

    let db = Db::open(&f.db).unwrap();
    assert!(
        db.exec("DELETE FROM evidence_reconstruction_attempt_v1")
            .is_err()
    );
    assert!(
        db.exec("UPDATE evidence_reconstruction_attempt_v1 SET report='{}'")
            .is_err()
    );
}
#[test]
fn no_consent_and_invalid_inputs_fail_closed_without_committing() {
    let f = Fixture::new();
    fail(f.evaluate("false"));
    fail(f.evaluate("other"));
    let db = Db::open(&f.db).unwrap();
    assert_eq!(
        db.scalar(
            "SELECT count(*) FROM sqlite_master WHERE name='evidence_reconstruction_attempt_v1'",
            &[],
        )
        .unwrap(),
        "0"
    );
    fs::write(&f.candidate, vec![b'!'; 1024 * 1024 + 1]).unwrap();
    fail(f.evaluate("true"));
    fs::write(&f.candidate, "int evaluate(int x) { BROKEN C }\n").unwrap();
    fail(f.evaluate("true"));
    fs::write(&f.candidate, "int evaluate(int x) { return x * 3; }\n").unwrap();
    fs::write(&f.target, "not an ELF object").unwrap();
    fail(f.evaluate("true"));
}
#[test]
fn changed_target_identity_does_not_reuse_prior_attempts() {
    let f = Fixture::new();
    json(f.evaluate("true"));
    let target2 = f.root.join("another.o");
    let source2 = f.root.join("another.c");
    fs::write(&source2, "int evaluate(int x) { return x * 7; }\n").unwrap();
    success(run(
        "cc",
        &[
            "-O2",
            "-fno-stack-protector",
            "-fno-asynchronous-unwind-tables",
            "-fno-ident",
            "-c",
            "-o",
            target2.to_str().unwrap(),
            source2.to_str().unwrap(),
        ],
    ));
    let other = json(cli(&[
        "reconstruct",
        "history",
        "--db",
        f.db(),
        "--target",
        target2.to_str().unwrap(),
    ]));
    assert_eq!(
        other
            .get("stored_attempt_count")
            .unwrap()
            .integer()
            .unwrap(),
        0
    );
}
