//! Bounded compiler-in-the-loop synthesis for a single C integer function.
//! Reuse system binutils; never run the target. No semantic or source-recovery claim.
use super::{FLAGS, MAX_TEXT, Scratch, bounded_file, code_region, run, sample, subject};
use crate::evidence::artifact;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, refuse};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_SEARCH: usize = 128;
const GENERATOR: &str = "c-one-arg-integer-v1";
const SCHEMA: &str = "excession.reconstruction-search.v1";

pub struct SearchOptions<'a> {
    pub target: &'a Path,
    pub output: &'a Path,
    pub compiler: &'a str,
    pub objcopy: &'a str,
    pub objdump: &'a str,
    pub nm: &'a str,
    pub budget: usize,
    pub seconds: u64,
    pub trusted_local: bool,
}

struct Hypothesis {
    expression: String,
    source: String,
}
fn tool_output(program: &str, args: &[&str], path: &Path) -> Result<String> {
    let out = fs::File::create(path).map_err(|e| format!("tool stdout: {e}"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("start trusted-local {program:?}: {e}"))?;
    let begin = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => break,
            Some(status) => return refuse(format!("trusted-local {program:?} failed: {status}")),
            None if begin.elapsed() >= Duration::from_secs(10) => {
                let _ = child.kill();
                let _ = child.wait();
                return refuse(format!("trusted-local {program:?} timed out"));
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let bytes = bounded_file(path, 256 * 1024)?;
    String::from_utf8(bytes).map_err(|_| "tool output was not UTF-8".into())
}

fn one_function(nm: &str) -> Result<String> {
    let names = nm
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let name = columns.next()?;
            let kind = columns.next()?;
            matches!(kind, "T" | "t").then_some(name)
        })
        .collect::<Vec<_>>();
    if names.len() != 1 {
        return refuse("automatic C reconstruction requires exactly one defined text function");
    }
    let name = names[0];
    if name.len() > 128
        || !name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return refuse("C reconstruction function symbol is not a supported identifier");
    }
    Ok(name.to_owned())
}

fn observed_constants(disassembly: &str) -> Vec<i32> {
    let mut values = Vec::new();
    for line in disassembly.lines() {
        let Some((_, instruction)) = line.split_once(":\t") else {
            continue;
        };
        let bytes = instruction.as_bytes();
        let mut at = 0;
        while at + 2 < bytes.len() {
            if bytes[at] == b'0' && bytes[at + 1] == b'x' {
                let begin = at + 2;
                let mut end = begin;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() && end - begin < 8 {
                    end += 1;
                }
                if end > begin
                    && (end == bytes.len() || !bytes[end].is_ascii_hexdigit())
                    && let Ok(value) = u32::from_str_radix(&instruction[begin..end], 16)
                {
                    let signed = if at > 0 && bytes[at - 1] == b'-' {
                        -(value as i64)
                    } else {
                        value as i32 as i64
                    };
                    if (i32::MIN as i64..=i32::MAX as i64).contains(&signed) {
                        values.push(signed as i32);
                    }
                }
                at = end;
            } else {
                at += 1;
            }
        }
    }
    // Scaled LEA can encode x * (1 + scale) without an immediate constant.
    for (pattern, factor) in [(",%rdi,2)", 3), (",%rdi,4)", 5), (",%rdi,8)", 9)] {
        if disassembly.contains(pattern) {
            values.insert(0, factor);
        }
    }
    values.extend([3, 5, 7, 2, 4, 8, 9, 1, -1, 0, 16, 31, 42]);
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|v| seen.insert(*v))
        .take(20)
        .collect()
}

fn hypotheses(name: &str, disassembly: &str) -> Vec<Hypothesis> {
    let values = observed_constants(disassembly);
    let scaled = disassembly.contains(",%rdi,");
    let offset = disassembly.contains("0x");
    let bitwise =
        disassembly.contains("xor") || disassembly.contains("and") || disassembly.contains("or ");
    let mut expressions = Vec::new();
    if scaled && offset {
        // Compile the likely combined LEA form before exploring its factors.
        for &a in values.iter().take(5) {
            for &c in values.iter().take(5) {
                if a != c {
                    expressions.push(format!("x * {a} + {c}"));
                }
            }
        }
    }
    let priorities = if bitwise {
        ["xor", "and", "or", "add", "mul", "sub", "const"]
    } else if scaled {
        ["mul", "add", "sub", "xor", "and", "or", "const"]
    } else if offset {
        ["add", "sub", "mul", "xor", "and", "or", "const"]
    } else {
        ["mul", "add", "sub", "xor", "and", "or", "const"]
    };
    expressions.extend(["x", "-x", "~x", "x * x"].iter().map(|s| (*s).to_owned()));
    for op in priorities {
        for &constant in &values {
            let expr = match op {
                "mul" => format!("x * {constant}"),
                "add" => format!("x + {constant}"),
                "sub" => format!("x - {constant}"),
                "xor" => format!("x ^ {constant}"),
                "and" => format!("x & {constant}"),
                "or" => format!("x | {constant}"),
                _ => format!("{constant}"),
            };
            expressions.push(expr);
        }
    }
    // Include direct shifts after the observed low-cost operation families.
    for shift in 1..=8 {
        expressions.push(format!("x << {shift}"));
        expressions.push(format!("x >> {shift}"));
    }
    let mut seen = BTreeSet::new();
    expressions
        .into_iter()
        .filter(|expr| seen.insert(expr.clone()))
        .map(|expression| Hypothesis {
            source: format!("int {name}(int x) {{ return {expression}; }}\n"),
            expression,
        })
        .collect()
}

fn schema(db: &Db) -> Result<()> {
    db.exec(
        "CREATE TABLE IF NOT EXISTS evidence_reconstruction_search_v1 (
           sequence INTEGER PRIMARY KEY AUTOINCREMENT,
           search_id TEXT NOT NULL UNIQUE,
           target_subject_id TEXT NOT NULL,
           report TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS evidence_reconstruction_search_target_idx
            ON evidence_reconstruction_search_v1(target_subject_id,sequence);
         CREATE INDEX IF NOT EXISTS evidence_reconstruction_text_lookup_v1
            ON evidence_reconstruction_search_v1(json_extract(report,'$.target_text_sha256'))
            WHERE json_extract(report,'$.search_status')='EXACT_TEXT_MATCH';
         CREATE TRIGGER IF NOT EXISTS evidence_reconstruction_search_no_update
           BEFORE UPDATE ON evidence_reconstruction_search_v1
           BEGIN SELECT RAISE(ABORT,'immutable reconstruction search'); END;
         CREATE TRIGGER IF NOT EXISTS evidence_reconstruction_search_no_delete
           BEFORE DELETE ON evidence_reconstruction_search_v1
           BEGIN SELECT RAISE(ABORT,'immutable reconstruction search'); END;",
    )
}

fn audit_report(raw: &str, search_id: &str, target_id: &str) -> Result<Value> {
    let mut parsed = json::parse(raw.as_bytes())?;
    if parsed.canonical() != raw
        || parsed.get("search_id")?.str()? != search_id
        || parsed.get("schema")?.str()? != SCHEMA
        || parsed.get("target_artifact_subject_id")?.str()? != target_id
    {
        return refuse("reconstruction search report is not canonical or subject-bound");
    }
    parsed.entries_mut()?.remove("search_id");
    if parsed.sha256() != search_id {
        return refuse("reconstruction search report digest mismatch");
    }
    parsed
        .entries_mut()?
        .insert("search_id".into(), Value::string(search_id));
    Ok(parsed)
}

pub fn searches(db: &Db, target: &Path, limit: usize) -> Result<Value> {
    if !(1..=32).contains(&limit) {
        return refuse("reconstruction search history limit must be 1..32");
    }
    let bytes = bounded_file(target, 16 * 1024 * 1024)?;
    let id = subject(&bytes)?;
    schema(db)?;
    db.read_transaction(|db| {
        let total = db
            .scalar(
                "SELECT count(*) FROM evidence_reconstruction_search_v1 WHERE target_subject_id=?",
                &[Param::Text(&id)],
            )?
            .parse::<usize>()
            .map_err(|_| "bad search count")?;
        let rows = db.query(
            "SELECT search_id,report FROM evidence_reconstruction_search_v1
             WHERE target_subject_id=? ORDER BY sequence DESC LIMIT ?",
            &[Param::Text(&id), Param::Integer(limit as i64)],
        )?;
        let mut found = Vec::new();
        for row in rows {
            found.push(audit_report(
                row[1].as_deref().ok_or("missing search report")?,
                row[0].as_deref().ok_or("missing search id")?,
                &id,
            )?);
        }
        Ok(obj(&[
            (
                "schema",
                Value::string("excession.reconstruction-search-history.v1"),
            ),
            ("target_artifact_subject_id", Value::string(id)),
            ("stored_search_count", Value::number(total)),
            ("truncated", Value::Bool(total > limit)),
            ("searches", Value::array(found)),
            ("execution_replay", Value::string("NOT_ESTABLISHED")),
        ]))
    })
}

/// Retrieve earlier successful source hypotheses for precisely the same .text
/// bytes, but never promote their prior compiler result to evidence for this
/// artifact. Only the current grammar's generated expressions may be reused.
/// Each borrowed hypothesis must compile and match against the new target.
fn reusable_hypothesis(
    db: &Db,
    text_sha256: &str,
    generated: &[Hypothesis],
) -> Result<Option<(usize, String)>> {
    schema(db)?;
    let rows = db.query(
        "SELECT search_id,target_subject_id,report
         FROM evidence_reconstruction_search_v1
         WHERE json_extract(report,'$.target_text_sha256')=?
           AND json_extract(report,'$.search_status')='EXACT_TEXT_MATCH'
         ORDER BY sequence DESC LIMIT 16",
        &[Param::Text(text_sha256)],
    )?;
    for row in rows {
        let id = row[0].as_deref().ok_or("missing reusable search id")?;
        let subject = row[1]
            .as_deref()
            .ok_or("missing reusable target identity")?;
        let raw = row[2].as_deref().ok_or("missing reusable search receipt")?;
        let receipt = audit_report(raw, id, subject)?;
        if receipt.get("generator")?.str()? != GENERATOR
            || receipt.get("comparison_scope")?.str()? != "ELF_TEXT_SECTION_BYTES_ONLY"
            || receipt.get("target_text_sha256")?.str()? != text_sha256
            || receipt.get("best_text_sha256")?.str()? != text_sha256
            || receipt.get("best_mismatch_count")?.integer()? != 0
        {
            return refuse("inconsistent reusable function evidence");
        }
        let expr = receipt.get("best_expression")?.str()?;
        let previous_name = receipt.get("target_function")?.str()?;
        let saved_source = receipt.get("best_source")?.str()?;
        let canonical = format!("int {previous_name}(int x) {{ return {expr}; }}\n");
        if saved_source != canonical
            || digest::sha256(canonical.as_bytes()) != receipt.get("best_source_sha256")?.str()?
        {
            return refuse("reusable function source no longer has canonical grammar identity");
        }
        if let Some(index) = generated.iter().position(|h| h.expression == expr) {
            return Ok(Some((index, id.to_owned())));
        }
    }
    Ok(None)
}

pub fn search(db: &Db, opt: &SearchOptions<'_>) -> Result<Value> {
    if !opt.trusted_local {
        return refuse(
            "automatic reconstruction requires --trusted-local true; compiler is not sandboxed",
        );
    }
    if !(1..=MAX_SEARCH).contains(&opt.budget) || !(1..=120).contains(&opt.seconds) {
        return refuse("automatic reconstruction requires 1..128 candidates and 1..120 seconds");
    }
    if [opt.compiler, opt.objcopy, opt.objdump, opt.nm]
        .iter()
        .any(|value| value.is_empty())
    {
        return refuse("reconstruction tool name cannot be empty");
    }
    if opt.output.exists() {
        return refuse("reconstruction output already exists; refusing overwrite");
    }
    let original = bounded_file(opt.target, 16 * 1024 * 1024)?;
    if !original.starts_with(b"\x7fELF") {
        return refuse("automatic reconstruction currently supports ELF object files only");
    }
    let original_id = subject(&original)?;
    let scratch = Scratch::new()?;
    let target = scratch.path("target.o");
    fs::write(&target, &original).map_err(|e| e.to_string())?;
    let text = code_region(opt.objcopy, &target, &scratch.path("original.text"))?;
    if text.len() > MAX_TEXT {
        return refuse("target function text exceeds bound");
    }
    let nm = tool_output(
        opt.nm,
        &[
            "-a",
            "--defined-only",
            "--format=posix",
            target.to_str().ok_or("invalid target path")?,
        ],
        &scratch.path("nm.txt"),
    )?;
    let function = one_function(&nm)?;
    let disasm = tool_output(
        opt.objdump,
        &[
            "-d",
            "-j",
            ".text",
            "--no-show-raw-insn",
            target.to_str().ok_or("invalid target path")?,
        ],
        &scratch.path("disasm.txt"),
    )?;
    let mut pool = hypotheses(&function, &disasm);
    let reuse = reusable_hypothesis(db, &digest::sha256(&text), &pool)?;
    if let Some((index, _)) = &reuse {
        let item = pool.remove(*index);
        pool.insert(0, item);
    }
    let begin = Instant::now();
    let mut examined = Vec::new();
    let mut best: Option<(usize, String, String, String)> = None;
    let mut completed = false;
    let source_file = scratch.path("candidate.c");
    let object_file = scratch.path("candidate.o");
    let section_file = scratch.path("candidate.text");
    for h in pool.iter().take(opt.budget) {
        if begin.elapsed() >= Duration::from_secs(opt.seconds) {
            break;
        }
        fs::write(&source_file, &h.source).map_err(|e| e.to_string())?;
        run(
            opt.compiler,
            &[
                FLAGS[0],
                FLAGS[1],
                FLAGS[2],
                FLAGS[3],
                FLAGS[4],
                "-o",
                object_file.to_str().ok_or("bad candidate path")?,
                source_file.to_str().ok_or("bad source path")?,
            ],
        )?;
        fs::remove_file(&section_file).ok();
        let candidate_text = code_region(opt.objcopy, &object_file, &section_file)?;
        let (difference, examples) = sample(&text, &candidate_text);
        let digest = digest::sha256(&candidate_text);
        examined.push(obj(&[
            ("expression", Value::string(&h.expression)),
            (
                "source_sha256",
                Value::string(digest::sha256(h.source.as_bytes())),
            ),
            ("text_sha256", Value::string(&digest)),
            ("mismatch_count", Value::number(difference)),
            ("mismatch_examples", Value::array(examples)),
        ]));
        if best.as_ref().is_none_or(|old| difference < old.0) {
            best = Some((difference, h.source.clone(), h.expression.clone(), digest));
        }
        if difference == 0 {
            completed = true;
            break;
        }
    }
    let Some((distance, source, expression, candidate_text_digest)) = best else {
        return refuse("automatic reconstruction budget expired before any candidate was compiled");
    };
    let state = if completed {
        "EXACT_TEXT_MATCH"
    } else if begin.elapsed() >= Duration::from_secs(opt.seconds) {
        "TIME_BUDGET_EXHAUSTED"
    } else {
        "CANDIDATE_BUDGET_EXHAUSTED"
    };
    let target_rea = artifact::dossier(db, &target, None, 1)?;
    let mut report = obj(&[
        ("schema", Value::string(SCHEMA)),
        ("generator", Value::string(GENERATOR)),
        (
            "candidate_selection",
            Value::string(if reuse.is_some() {
                "REUSED_EXACT_TEXT"
            } else {
                "GENERATED_GRAMMAR"
            }),
        ),
        (
            "reused_from_search_id",
            reuse
                .as_ref()
                .map_or(Value::Null, |(_, id)| Value::string(id)),
        ),
        ("target_artifact_subject_id", Value::string(&original_id)),
        (
            "target_artifact_sha256",
            Value::string(digest::sha256(&original)),
        ),
        ("target_text_sha256", Value::string(digest::sha256(&text))),
        ("target_function", Value::string(&function)),
        ("language", Value::string("C")),
        ("compiler", Value::string(opt.compiler)),
        ("objcopy", Value::string(opt.objcopy)),
        ("objdump", Value::string(opt.objdump)),
        ("nm", Value::string(opt.nm)),
        ("max_candidates", Value::number(opt.budget)),
        ("time_budget_seconds", Value::number(opt.seconds)),
        ("attempt_count", Value::number(examined.len())),
        ("search_status", Value::string(state)),
        ("best_expression", Value::string(expression)),
        ("best_source", Value::string(&source)),
        (
            "best_source_sha256",
            Value::string(digest::sha256(source.as_bytes())),
        ),
        ("best_text_sha256", Value::string(candidate_text_digest)),
        ("best_mismatch_count", Value::number(distance)),
        ("attempts", Value::array(examined)),
        (
            "target_rea_bundles",
            target_rea.get("rea_total_stored")?.clone(),
        ),
        (
            "execution_boundary",
            Value::string("TRUSTED_LOCAL_UNSANDBOXED"),
        ),
        (
            "comparison_scope",
            Value::string("ELF_TEXT_SECTION_BYTES_ONLY"),
        ),
        ("source_recovery", Value::string("NOT_ESTABLISHED")),
        ("behavioral_equivalence", Value::string("NOT_ESTABLISHED")),
        ("build_execution_attested", Value::Bool(false)),
        ("search_exhaustive", Value::Bool(false)),
    ]);
    let search_id = report.sha256();
    report
        .entries_mut()?
        .insert("search_id".into(), Value::string(&search_id));
    schema(db)?;
    db.transaction(|db| {
        db.execute(
            "INSERT OR IGNORE INTO evidence_reconstruction_search_v1(search_id,target_subject_id,report)
             VALUES (?,?,?)",
            &[Param::Text(&search_id), Param::Text(&original_id), Param::Text(&report.canonical())],
        )
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(opt.output)
        .map_err(|e| format!("output create: {e}"))?;
    file.write_all(source.as_bytes())
        .map_err(|e| format!("output write: {e}"))?;
    file.sync_all().map_err(|e| format!("output sync: {e}"))?;
    Ok(report)
}
