//! A bounded compiler-in-the-loop comparison for a single C object-code region.
//! Code-byte equality is not source recovery, behavioral equivalence or build attestation.
mod automatic;
use crate::evidence::{artifact, identity::Subject};
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, refuse};
pub use automatic::{SearchOptions, search, searches};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_SOURCE: usize = 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
const MAX_HISTORY: usize = 32;
const FLAGS: &[&str] = &[
    "-O2",
    "-fno-stack-protector",
    "-fno-asynchronous-unwind-tables",
    "-fno-ident",
    "-c",
];
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "excession-reconstruct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).map_err(|e| format!("reconstruction scratch: {e}"))?;
        Ok(Self(dir))
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn trusted-local tool {program:?}: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                return refuse(format!("trusted-local tool {program:?} failed: {status}"));
            }
            None if started.elapsed() >= Duration::from_secs(15) => {
                let _ = child.kill();
                let _ = child.wait();
                return refuse(format!("trusted-local tool {program:?} timed out"));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn bounded_file(path: &Path, max: usize) -> Result<Vec<u8>> {
    let meta = fs::metadata(path).map_err(|e| format!("input metadata: {e}"))?;
    if !meta.is_file() || meta.len() > max as u64 {
        return refuse("reconstruction input is not a bounded regular file");
    }
    let bytes = fs::read(path).map_err(|e| format!("input read: {e}"))?;
    if bytes.is_empty() || bytes.len() > max {
        return refuse("reconstruction input must be nonempty and bounded");
    }
    Ok(bytes)
}

fn code_region(objcopy: &str, source: &Path, destination: &Path) -> Result<Vec<u8>> {
    let arg = format!(".text={}", destination.display());
    run(
        objcopy,
        &[
            "--dump-section",
            &arg,
            source.to_str().ok_or("non-UTF-8 object path")?,
        ],
    )?;
    bounded_file(destination, MAX_TEXT)
}

fn schema(db: &Db) -> Result<()> {
    db.exec(
        "CREATE TABLE IF NOT EXISTS evidence_reconstruction_attempt_v1 (
           sequence INTEGER PRIMARY KEY AUTOINCREMENT,
           attempt_id TEXT NOT NULL UNIQUE,
           target_subject_id TEXT NOT NULL,
           candidate_source_sha256 TEXT NOT NULL,
           report TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS evidence_reconstruction_target_idx
           ON evidence_reconstruction_attempt_v1(target_subject_id,sequence);
         CREATE TRIGGER IF NOT EXISTS evidence_reconstruction_no_update
           BEFORE UPDATE ON evidence_reconstruction_attempt_v1
           BEGIN SELECT RAISE(ABORT,'immutable reconstruction evidence'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_reconstruction_no_delete
           BEFORE DELETE ON evidence_reconstruction_attempt_v1
           BEGIN SELECT RAISE(ABORT,'immutable reconstruction evidence'); END;",
    )
}

fn subject(bytes: &[u8]) -> Result<String> {
    Subject::Artifact {
        sha256: digest::sha256(bytes),
        size_bytes: bytes.len() as u64,
    }
    .id()
}

fn sample(left: &[u8], right: &[u8]) -> (usize, Vec<Value>) {
    let mut mismatches = 0usize;
    let mut examples = Vec::new();
    for offset in 0..left.len().max(right.len()) {
        let l = left.get(offset);
        let r = right.get(offset);
        if l != r {
            mismatches += 1;
            if examples.len() < 16 {
                examples.push(obj(&[
                    ("offset", Value::number(offset)),
                    ("target", l.map_or(Value::Null, |b| Value::number(*b))),
                    ("candidate", r.map_or(Value::Null, |b| Value::number(*b))),
                ]));
            }
        }
    }
    (mismatches, examples)
}

/// Evaluate a candidate by compiling a copied source with a fixed C recipe,
/// then extracting only .text from the target object and candidate object.
/// This executes local processes and is explicitly forbidden without consent.
pub fn evaluate(
    db: &Db,
    target: &Path,
    candidate: &Path,
    compiler: &str,
    objcopy: &str,
    trusted_local: bool,
) -> Result<Value> {
    if !trusted_local {
        return refuse(
            "reconstruction compilation requires --trusted-local true; no sandbox is provided",
        );
    }
    if compiler.is_empty() || objcopy.is_empty() {
        return refuse("compiler and objcopy programs must be specified");
    }
    let original = bounded_file(target, 16 * 1024 * 1024)?;
    let source = bounded_file(candidate, MAX_SOURCE)?;
    let original_id = subject(&original)?;
    let scratch = Scratch::new()?;
    let target_copy = scratch.path("target.o");
    let source_copy = scratch.path("candidate.c");
    let object = scratch.path("candidate.o");
    fs::write(&target_copy, &original).map_err(|e| e.to_string())?;
    fs::write(&source_copy, &source).map_err(|e| e.to_string())?;
    let source_path = source_copy
        .to_str()
        .ok_or("non-UTF-8 candidate source path")?;
    let object_path = object.to_str().ok_or("non-UTF-8 candidate object path")?;
    let args = [
        FLAGS[0],
        FLAGS[1],
        FLAGS[2],
        FLAGS[3],
        FLAGS[4],
        "-o",
        object_path,
        source_path,
    ];
    run(compiler, &args)?;
    let candidate_object = bounded_file(&object, 16 * 1024 * 1024)?;
    let target_text = code_region(objcopy, &target_copy, &scratch.path("target.text"))?;
    let candidate_text = code_region(objcopy, &object, &scratch.path("candidate.text"))?;
    let (mismatch_count, mismatch_examples) = sample(&target_text, &candidate_text);
    let match_text = mismatch_count == 0;
    let candidate_id = subject(&candidate_object)?;
    let target_rea = artifact::dossier(db, &target_copy, None, 1)?;
    let candidate_rea = artifact::dossier(db, &object, None, 1)?;
    let mut report = obj(&[
        (
            "schema",
            Value::string("excession.reconstruction-attempt.v1"),
        ),
        ("target_artifact_subject_id", Value::string(&original_id)),
        (
            "target_artifact_sha256",
            Value::string(digest::sha256(&original)),
        ),
        (
            "candidate_artifact_subject_id",
            Value::string(&candidate_id),
        ),
        (
            "candidate_artifact_sha256",
            Value::string(digest::sha256(&candidate_object)),
        ),
        (
            "candidate_source_sha256",
            Value::string(digest::sha256(&source)),
        ),
        ("compiler", Value::string(compiler)),
        ("objcopy", Value::string(objcopy)),
        (
            "compiler_arguments",
            Value::array(
                FLAGS
                    .iter()
                    .copied()
                    .chain(["-o", "<candidate-object>", "<candidate-source>"])
                    .map(Value::string),
            ),
        ),
        (
            "comparison_scope",
            Value::string("ELF_TEXT_SECTION_BYTES_ONLY"),
        ),
        (
            "target_text_sha256",
            Value::string(digest::sha256(&target_text)),
        ),
        (
            "candidate_text_sha256",
            Value::string(digest::sha256(&candidate_text)),
        ),
        ("target_text_bytes", Value::number(target_text.len())),
        ("candidate_text_bytes", Value::number(candidate_text.len())),
        ("mismatch_count", Value::number(mismatch_count)),
        ("mismatch_examples", Value::array(mismatch_examples)),
        (
            "text_relation",
            Value::string(if match_text {
                "IDENTICAL_TEXT_BYTES"
            } else {
                "DIFFERENT_TEXT_BYTES"
            }),
        ),
        (
            "target_rea_bundles",
            target_rea.get("rea_total_stored")?.clone(),
        ),
        (
            "candidate_rea_bundles",
            candidate_rea.get("rea_total_stored")?.clone(),
        ),
        ("source_recovery", Value::string("NOT_ESTABLISHED")),
        ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ("relocation_equivalence", Value::string("NOT_ESTABLISHED")),
        ("build_execution_attested", Value::Bool(false)),
        (
            "execution_boundary",
            Value::string("TRUSTED_LOCAL_UNSANDBOXED"),
        ),
    ]);
    // Scratch path names are never part of the content-addressed attempt.
    let id = report.sha256();
    report
        .entries_mut()?
        .insert("attempt_id".into(), Value::string(&id));
    schema(db)?;
    db.transaction(|db| {
        db.execute(
            "INSERT OR IGNORE INTO evidence_reconstruction_attempt_v1
             (attempt_id,target_subject_id,candidate_source_sha256,report)
             VALUES (?,?,?,?)",
            &[
                Param::Text(&id),
                Param::Text(&original_id),
                Param::Text(&digest::sha256(&source)),
                Param::Text(&report.canonical()),
            ],
        )?;
        Ok(())
    })?;
    Ok(report)
}

/// Read back a bounded sequence of immutable attempts, checking the exact
/// target artifact identity and each report's content-addressed identity.
pub fn history(db: &Db, target: &Path, limit: usize) -> Result<Value> {
    if !(1..=MAX_HISTORY).contains(&limit) {
        return refuse("reconstruction history limit must be 1..32");
    }
    let target_bytes = bounded_file(target, 16 * 1024 * 1024)?;
    let id = subject(&target_bytes)?;
    schema(db)?;
    db.read_transaction(|db| {
        let total = db.scalar(
            "SELECT count(*) FROM evidence_reconstruction_attempt_v1 WHERE target_subject_id=?",
            &[Param::Text(&id)],
        )?.parse::<usize>().map_err(|_| "invalid reconstruction history count")?;
        let rows = db.query(
            "SELECT attempt_id,candidate_source_sha256,report FROM evidence_reconstruction_attempt_v1
             WHERE target_subject_id=? ORDER BY sequence DESC LIMIT ?",
            &[Param::Text(&id), Param::Integer(i64::try_from(limit).map_err(|_| "invalid history limit")?)],
        )?;
        let mut attempts = Vec::new();
        for row in rows {
            let stored_id = row[0].as_deref().ok_or("missing attempt id")?;
            let stored_source = row[1].as_deref().ok_or("missing attempt source")?;
            let raw = row[2].as_deref().ok_or("missing attempt report")?;
            let mut v = json::parse(raw.as_bytes())?;
            if v.canonical() != raw
                || v.get("schema")?.str()? != "excession.reconstruction-attempt.v1"
                || v.get("attempt_id")?.str()? != stored_id
                || v.get("target_artifact_subject_id")?.str()? != id
                || v.get("candidate_source_sha256")?.str()? != stored_source
            {
                return refuse("stored reconstruction report is inconsistent");
            }
            v.entries_mut()?.remove("attempt_id");
            if v.sha256() != stored_id {
                return refuse("stored reconstruction report identity mismatch");
            }
            v.entries_mut()?.insert("attempt_id".into(), Value::string(stored_id));
            attempts.push(v);
        }
        Ok(obj(&[
            ("schema", Value::string("excession.reconstruction-history.v1")),
            ("target_artifact_subject_id", Value::string(id)),
            ("stored_attempt_count", Value::number(total)),
            ("truncated", Value::Bool(total > limit)),
            ("attempts", Value::array(attempts)),
            ("audit_scope", Value::string("TARGET_BYTES_AND_STORED_REPORT_INTEGRITY")),
            ("execution_attestation", Value::string("NOT_ESTABLISHED")),
        ]))
    })
}
