//! Source-bound prospective exception-propagation hypotheses.
//!
//! The Java compiler AST is an external, syntax-only producer, not a proof of
//! runtime behavior or a complete interprocedural semantics. Rust validates
//! complete tracked-source coverage, commit/blob identities and frozen inputs.
//! The B-side comparison cannot read the later repair C or a behavioral oracle.
use crate::git;
use crate::json::{self, Value, obj};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const PREFIX: &str = "src/main/java/";
const OBS_SCHEMA: &str = "excession.java.exception-flow.observation.v1";
const FROZEN_SCHEMA: &str = "excession.concepts.exception-flow-frozen.v1";
const REPORT_SCHEMA: &str = "excession.concepts.exception-flow-review.v1";
const MAX_INPUT: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 512;
const MAX_ROWS: usize = 20_000;
const MAX_OBLIGATIONS: usize = 4096;

type Key = (String, String, i64, String);
#[derive(Clone)]
struct Handler {
    source: String,
    method: String,
    ordinal: i64,
    caught: String,
    line: i64,
    direct_throws: i64,
    modeled_exits: i64,
    guaranteed_throw: bool,
}
impl Handler {
    fn key(&self) -> Key {
        (self.source.clone(), self.method.clone(), self.ordinal, self.caught.clone())
    }
    fn value(&self, blob: &str) -> Value {
        obj(&[
            ("source", Value::string(&self.source)),
            ("source_blob", Value::string(blob)),
            ("method", Value::string(&self.method)),
            ("catch_index", Value::number(self.ordinal)),
            ("caught", Value::string(&self.caught)),
            ("line", Value::number(self.line)),
            ("direct_throw_count", Value::number(self.direct_throws)),
            ("modeled_exits", Value::number(self.modeled_exits)),
            ("guaranteed_throw", Value::Bool(self.guaranteed_throw)),
        ])
    }
}
struct Snapshot {
    commit: String,
    tree: String,
    raw_hash: String,
    files: BTreeMap<String, String>,
    handlers: BTreeMap<Key, Handler>,
}
fn oid_commit(repo: &Path, rev: &str) -> Result<String> {
    git::verify_repo(repo)?;
    let commit = git::rev(repo, rev)?;
    if git::text(repo, &["cat-file", "-t", &commit])? != "commit" {
        return refuse("exception-flow revision is not an object of type commit");
    }
    Ok(commit)
}
fn bounded_input(path: &Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path).map_err(|e| format!("source observation stat: {e}"))?.len();
    if len == 0 || len > MAX_INPUT {
        return refuse("source observation must be nonempty and at most four MiB");
    }
    std::fs::read(path).map_err(|e| format!("source observation read: {e}"))
}
fn row_text<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row.get(key)?.str()
}
fn number(row: &Value, key: &str, high: i64) -> Result<i64> {
    let n = row.get(key)?.integer()?;
    if !(0..=high).contains(&n) {
        return refuse(format!("invalid AST observation integer for {key}"));
    }
    Ok(n)
}
fn snapshot(repo: &Path, rev: &str, path: &Path) -> Result<Snapshot> {
    let commit = oid_commit(repo, rev)?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let expected: BTreeMap<String,String> = git::entries(repo, &commit)?
        .into_iter()
        .filter(|entry| entry.kind == "blob" && entry.path.starts_with(PREFIX)
            && entry.path.ends_with(".java")
            && (entry.mode == "100644" || entry.mode == "100755"))
        .map(|entry| (entry.path, entry.oid)).collect();
    if expected.is_empty() || expected.len() > MAX_FILES {
        return refuse("Java exception observation requires 1..512 tracked source files");
    }
    let raw = bounded_input(path)?;
    let text = std::str::from_utf8(&raw).map_err(|_| "non-UTF8 AST observation stream")?;
    let raw_hash = crate::digest::sha256(&raw);
    let mut sources = BTreeSet::new();
    let mut handlers = BTreeMap::new();
    for (index,line) in text.lines().enumerate() {
        if index >= MAX_ROWS { return refuse("too many AST observations"); }
        if line.is_empty() { return refuse("empty AST observation record"); }
        let item = json::parse(line.as_bytes())?;
        if row_text(&item, "schema")? != OBS_SCHEMA {
            return refuse("unsupported Java AST producer receipt");
        }
        let path = row_text(&item, "source")?.to_owned();
        git::validate_path(&path)?;
        if !expected.contains_key(&path) {
            return refuse("AST observation references source outside pinned Git tree");
        }
        match row_text(&item, "kind")? {
            "SOURCE" => {
                if !sources.insert(path) {
                    return refuse("duplicate Java source observation");
                }
            }
            "CATCH" => {
                let method = row_text(&item, "method")?.to_owned();
                let caught = row_text(&item, "caught")?.to_owned();
                if method.is_empty() || method.len()>300 || caught.is_empty() || caught.len()>200 {
                    return refuse("invalid method or caught exception name");
                }
                let ordinal = number(&item, "catch_index", 2048)?;
                let line = number(&item, "line", 2_000_000)?;
                let direct_throws = number(&item, "direct_throw_count", 2000)?;
                let exits = number(&item, "modeled_exits", 7)?;
                let guaranteed = item.get("guaranteed_throw")?.bool()?;
                if line == 0 || guaranteed != (exits == 2)
                    || (direct_throws > 0 && exits == 0) {
                    return refuse("internally inconsistent AST exception-flow observation");
                }
                let observation = Handler {
                    source: path, method, ordinal, caught, line,
                    direct_throws, modeled_exits: exits, guaranteed_throw: guaranteed,
                };
                if handlers.insert(observation.key(), observation).is_some() {
                    return refuse("duplicate source/method/catch identity");
                }
            }
            _ => return refuse("unknown Java source observation record type"),
        }
    }
    if sources != expected.keys().cloned().collect::<BTreeSet<_>>() {
        return refuse("incomplete Java source coverage at pinned Git revision");
    }
    Ok(Snapshot { commit, tree, raw_hash, files: expected, handlers })
}
fn seal(mut report: Value) -> Result<Value> {
    let hash = report.sha256();
    report.entries_mut()?.insert("report_id".to_owned(), Value::string(hash));
    Ok(report)
}
fn verify_seal(report: &Value) -> Result<()> {
    let expected = row_text(report, "report_id")?;
    let mut unsigned = report.clone();
    unsigned.entries_mut()?.remove("report_id");
    if unsigned.sha256() != expected {
        return refuse("frozen exception observation report digest mismatch");
    }
    Ok(())
}
/// The working revision A is the *only* Git revision supplied here.
/// The caller must not include B/C observations, regressions or oracle tests.
pub fn freeze(repo: &Path, rev: &str, observations: &Path) -> Result<Value> {
    let source = snapshot(repo, rev, observations)?;
    let obligations: Vec<Value> = source.handlers.values()
        .filter(|h| h.guaranteed_throw)
        .map(|h| h.value(&source.files[&h.source]))
        .collect();
    if obligations.len() > MAX_OBLIGATIONS {
        return refuse("too many exception-propagation obligations");
    }
    seal(obj(&[
        ("schema", Value::string(FROZEN_SCHEMA)),
        ("source_commit", Value::string(&source.commit)),
        ("source_tree", Value::string(&source.tree)),
        ("producer", Value::string("JDK_COMPILER_AST_PARSE_ONLY")),
        ("inference", Value::string("CONSERVATIVE_INTERPROCEDURAL_EXIT_SUMMARY")),
        ("raw_observation_sha256", Value::string(&source.raw_hash)),
        ("scanned_source_files", Value::number(source.files.len())),
        ("observed_catch_handlers", Value::number(source.handlers.len())),
        ("candidate_count", Value::number(obligations.len())),
        ("obligations", Value::array(obligations)),
        ("semantic_status", Value::string("PROPOSED_NOT_VERIFIED")),
        ("behavioral_oracle_seen", Value::Bool(false)),
        ("repair_commit_seen", Value::Bool(false)),
        ("observation_trust", Value::string("UNATTESTED_EXTERNAL_AST_PRODUCER")),
    ]))
}
fn modified(repo: &Path, a: &str, b: &str) -> Result<BTreeSet<String>> {
    let bytes = git::command(repo, &[
        "diff", "--name-only", "-z", "--diff-filter=M", a, b, "--", PREFIX,
    ])?;
    bytes.split(|c| *c == 0).filter(|p| !p.is_empty())
        .map(|p| String::from_utf8(p.to_vec())
            .map_err(|_| "non-UTF8 changed source path".to_owned()))
        .collect()
}
/// Assess B using only a previously sealed A model and an independently
/// complete AST scan of B. The result is a *candidate* obligation loss.
pub fn assess(repo: &Path, candidate: &str, frozen: &Path, observations: &Path) -> Result<Value> {
    let data = bounded_input(frozen)?;
    let base = json::parse(&data)?;
    verify_seal(&base)?;
    if row_text(&base, "schema")? != FROZEN_SCHEMA
        || row_text(&base, "semantic_status")? != "PROPOSED_NOT_VERIFIED"
        || row_text(&base, "producer")? != "JDK_COMPILER_AST_PARSE_ONLY"
        || base.get("behavioral_oracle_seen")?.bool()?
        || base.get("repair_commit_seen")?.bool()? {
        return refuse("expected a clean, prospective A-only exception-flow freeze");
    }
    let base_commit = row_text(&base, "source_commit")?;
    let base_tree = git::rev(repo, &format!("{base_commit}^{{tree}}"))?;
    if row_text(&base, "source_tree")? != base_tree {
        return refuse("frozen source tree no longer matches pinned commit");
    }
    let observed = snapshot(repo, candidate, observations)?;
    if base_commit == observed.commit {
        return refuse("candidate must be a later distinct source revision");
    }
    git::text(repo, &["merge-base", "--is-ancestor", base_commit, &observed.commit])?;
    let changed = modified(repo, base_commit, &observed.commit)?;
    let candidates = base.get("obligations")?.array_ref()?;
    if candidates.len() > MAX_OBLIGATIONS
        || number(&base, "candidate_count", MAX_OBLIGATIONS as i64)? != candidates.len() as i64 {
        return refuse("frozen obligation count mismatch");
    }
    let mut reports = Vec::new();
    let mut lost = 0_usize;
    let mut baseline = 0_usize;
    let mut incremental = 0_usize;
    let mut eligible = 0_usize;
    for item in candidates {
        if !item.get("guaranteed_throw")?.bool()? {
            return refuse("frozen obligation lacks established syntax precondition");
        }
        let path = row_text(item, "source")?.to_owned();
        let method = row_text(item, "method")?.to_owned();
        let caught = row_text(item, "caught")?.to_owned();
        let ordinal = number(item, "catch_index", 2048)?;
        git::validate_path(&path)?;
        let actual_base_blob = git::blob(repo, base_commit, &path)?;
        if row_text(item, "source_blob")? != actual_base_blob {
            return refuse("frozen source obligation blob differs from Git");
        }
        if !changed.contains(&path) {
            continue;
        }
        eligible += 1;
        let key = (path.clone(), method.clone(), ordinal, caught.clone());
        let new_handler = observed.handlers.get(&key);
        let status = match new_handler {
            Some(h) if h.guaranteed_throw => "PRESERVED_MODELED_THROW_GUARANTEE",
            Some(_) => "LOST_MODELED_THROW_GUARANTEE",
            None => "HANDLER_REMOVED_OR_REKEYED_UNSUPPORTED",
        };
        let is_lost = status == "LOST_MODELED_THROW_GUARANTEE";
        let simple = new_handler.is_some_and(|h| {
            item.get("direct_throw_count").ok().and_then(|v| v.integer().ok())
                .is_some_and(|old| old > 0 && h.direct_throws == 0)
        });
        if is_lost { lost += 1; }
        if simple { baseline += 1; }
        if is_lost && !simple { incremental += 1; }
        let new_line = new_handler.map_or(Value::Null, |h| Value::number(h.line));
        let new_blob = observed.files.get(&path)
            .map_or(Value::Null, |s| Value::string(s));
        reports.push(obj(&[
            ("source", Value::string(&path)),
            ("method", Value::string(&method)),
            ("caught", Value::string(&caught)),
            ("catch_index", Value::number(ordinal)),
            ("base_line", item.get("line")?.clone()),
            ("base_blob", Value::string(&actual_base_blob)),
            ("candidate_blob", new_blob),
            ("candidate_line", new_line),
            ("state", Value::string(status)),
            ("simple_local_throw_removal", Value::Bool(simple)),
            ("behavioral_test_generated", Value::Bool(false)),
            ("validity", Value::string("UNVERIFIED_STATIC_HYPOTHESIS")),
        ]));
    }
    let report = obj(&[
        ("schema", Value::string(REPORT_SCHEMA)),
        ("frozen_report_id", base.get("report_id")?.clone()),
        ("source_commit", Value::string(base_commit)),
        ("source_tree", Value::string(&base_tree)),
        ("candidate_commit", Value::string(&observed.commit)),
        ("candidate_tree", Value::string(&observed.tree)),
        ("candidate_observation_sha256", Value::string(&observed.raw_hash)),
        ("candidate_scanned_files", Value::number(observed.files.len())),
        ("frozen_obligations", Value::number(candidates.len())),
        ("modified_obligations", Value::number(eligible)),
        ("lost_modeled_throw_guarantees", Value::number(lost)),
        ("simple_local_throw_removals", Value::number(baseline)),
        ("model_only_handler_warnings", Value::number(incremental)),
        ("hypotheses", Value::array(reports)),
        ("status", Value::string("UNVERIFIED_SOURCE_ONLY_CONTROL_FLOW_CHANGE")),
        ("behavioral_oracle_seen", Value::Bool(false)),
        ("repair_commit_seen", Value::Bool(false)),
        ("semantic_invariant_proven", Value::Bool(false)),
        ("superior_bug_prevention_proven", Value::Bool(false)),
    ]);
    seal(report)
}
