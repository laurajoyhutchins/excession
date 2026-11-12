//! Native, source-specified relational workload for the benchmark program.
//!
//! The fixture's import graph is defined before any source is written or indexed.
//! This module times complete CLI processes; it is not a Criterion microbenchmark.
use crate::{Result, digest, refuse};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const KINDS: [(&str, &str); 4] = [
    ("focus", "focus"),
    ("dependencies_direct", "direct-dependencies"),
    ("dependents_direct", "direct-dependents"),
    ("tests", "indexed-test"),
];
const TIMEOUT: Duration = Duration::from_secs(90);
type Graph = BTreeMap<String, BTreeSet<String>>;

#[derive(Debug)]
pub struct Config {
    pub out: PathBuf,
    pub sizes: Vec<usize>,
    pub fanouts: Vec<usize>,
    pub revisions: usize,
    pub repetitions: usize,
}

impl Config {
    fn validate(&self) -> Result<()> {
        if self.out.exists() {
            return refuse("benchmark output directory already exists");
        }
        if self.sizes.is_empty()
            || self.fanouts.is_empty()
            || self.sizes.len() > 8
            || self.fanouts.len() > 8
            || self.sizes.iter().any(|n| !(4..=512).contains(n))
            || self.fanouts.contains(&0)
            || self
                .fanouts
                .iter()
                .any(|f| self.sizes.iter().any(|n| f >= n))
            || !(1..=20).contains(&self.revisions)
            || !(1..=50).contains(&self.repetitions)
        {
            return refuse(
                "invalid benchmark dimensions (size 4..512, fanout < size, revisions 1..20, repetitions 1..50)",
            );
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Revision {
    commit: String,
    tree: String,
    graph: Graph,
    ordinal: usize,
}

fn module_name(i: usize) -> String {
    format!("bench.m{i:04}")
}

fn graph_spec(n: usize, fanout: usize, revision: usize) -> Result<Graph> {
    if n < 4 || fanout == 0 || fanout >= n || revision >= 20 {
        return refuse("invalid fixture graph dimensions");
    }
    let mut graph = Graph::new();
    graph.insert("bench".into(), BTreeSet::new());
    graph.insert("tests".into(), BTreeSet::new());
    for i in 0..n {
        let mut edges = BTreeSet::new();
        for j in 1..=fanout {
            if i + j < n {
                edges.insert(module_name(i + j));
            }
        }
        graph.insert(module_name(i), edges);
    }
    if revision > 0 {
        let pivot = 1 + (revision - 1) % (n - 1);
        graph
            .get_mut(&module_name(pivot))
            .ok_or("missing pivot")?
            .insert(module_name(0));
    }
    graph.insert(
        "tests.test_links".into(),
        [module_name(0), module_name(n / 2)].into_iter().collect(),
    );
    Ok(graph)
}

fn module_path(name: &str) -> String {
    if matches!(name, "bench" | "tests") {
        format!("{name}/__init__.py")
    } else {
        format!("{}.py", name.replace('.', "/"))
    }
}

fn source_for(graph: &Graph, name: &str) -> Result<String> {
    let edges = graph.get(name).ok_or("missing source module")?;
    let mut source = String::new();
    for target in edges {
        source.push_str(&format!("import {target}\n"));
    }
    source.push_str(&format!("\nMODULE = '{name}'\n"));
    Ok(source)
}

fn reachable(graph: &Graph, seed: &str, reverse: bool) -> BTreeMap<String, usize> {
    let mut found = BTreeMap::from([(seed.to_owned(), 0)]);
    let mut pending = VecDeque::from([seed.to_owned()]);
    while let Some(current) = pending.pop_front() {
        let depth = found[&current];
        let neighbors: Vec<String> = if reverse {
            graph
                .iter()
                .filter(|(_, edges)| edges.contains(&current))
                .map(|(from, _)| from.clone())
                .collect()
        } else {
            graph[&current].iter().cloned().collect()
        };
        for next in neighbors {
            if !found.contains_key(&next) {
                found.insert(next.clone(), depth + 1);
                pending.push_back(next);
            }
        }
    }
    found
}

fn oracle(graph: &Graph, seed: &str, kind: &str) -> Result<Vec<String>> {
    if !graph.contains_key(seed) || !KINDS.iter().any(|(name, _)| *name == kind) {
        return refuse("unknown oracle query");
    }
    let selected: BTreeSet<String> = match kind {
        "focus" => [seed.to_owned()].into_iter().collect(),
        "tests" => {
            let forward = reachable(graph, seed, false);
            let backward = reachable(graph, seed, true);
            forward
                .keys()
                .chain(backward.keys())
                .filter(|name| name.starts_with("tests.") && name.as_str() != seed)
                .cloned()
                .collect()
        }
        "dependencies_direct" | "dependents_direct" => {
            reachable(graph, seed, kind == "dependents_direct")
                .into_iter()
                .filter(|(_, depth)| *depth == 1)
                .map(|(name, _)| name)
                .collect()
        }
        _ => unreachable!(),
    };
    Ok(selected.iter().map(|name| module_path(name)).collect())
}

#[cfg(unix)]
fn children_cpu_ms() -> Result<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage writes the initialized rusage structure on success.
    if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) } != 0 {
        return Err(format!("getrusage: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: the successful getrusage call initialized the structure.
    let usage = unsafe { usage.assume_init() };
    let total_seconds = (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1_000_000.0;
    Ok(total_seconds * 1000.0)
}

#[cfg(not(unix))]
fn children_cpu_ms() -> Result<f64> {
    refuse("relational benchmark requires Unix child-process CPU accounting")
}

fn round3(n: f64) -> f64 {
    (n * 1000.0).round() / 1000.0
}

struct Observation {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
    wall_ms: f64,
    child_cpu_ms: f64,
    timeout: bool,
}

fn observe(command: &mut Command) -> Result<Observation> {
    let before = children_cpu_ms()?;
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {:?}: {e}", command))?;
    let mut stdout = child.stdout.take().ok_or("missing subprocess stdout")?;
    let mut stderr = child.stderr.take().ok_or("missing subprocess stderr")?;
    let reader_out = thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let reader_err = thread::spawn(move || {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).map(|_| output)
    });
    let (status, timed_out) = loop {
        match child.try_wait().map_err(|e| format!("wait: {e}"))? {
            Some(status) => break (status, false),
            None if started.elapsed() >= TIMEOUT => {
                let _ = child.kill();
                break (
                    child
                        .wait()
                        .map_err(|e| format!("reap timed-out process: {e}"))?,
                    true,
                );
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let out = reader_out
        .join()
        .map_err(|_| "stdout reader panicked")?
        .map_err(|e| e.to_string())?;
    let err = reader_err
        .join()
        .map_err(|_| "stderr reader panicked")?
        .map_err(|e| e.to_string())?;
    Ok(Observation {
        stdout: out,
        stderr: err,
        exit_code: if timed_out {
            124
        } else {
            status.code().unwrap_or(1)
        },
        wall_ms: round3(started.elapsed().as_secs_f64() * 1000.0),
        child_cpu_ms: round3((children_cpu_ms()? - before).max(0.0)),
        timeout: timed_out,
    })
}

fn checked(command: &mut Command) -> Result<Observation> {
    let observed = observe(command)?;
    if observed.exit_code != 0 {
        return refuse(format!(
            "command {:?} failed ({}): {}",
            command,
            observed.exit_code,
            String::from_utf8_lossy(&observed.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    Ok(observed)
}

fn git_string(repo: &Path, args: &[&str]) -> Result<String> {
    let data = checked(Command::new("git").arg("-C").arg(repo).args(args))?.stdout;
    String::from_utf8(data)
        .map(|x| x.trim().to_string())
        .map_err(|e| e.to_string())
}

fn fixture(source: &Path, n: usize, fanout: usize, revisions: usize) -> Result<Vec<Revision>> {
    fs::create_dir(source).map_err(|e| format!("create source repo: {e}"))?;
    checked(Command::new("git").arg("init").arg("-q").arg(source))?;
    let mut history = Vec::new();
    for rev in 0..revisions {
        let graph = graph_spec(n, fanout, rev)?;
        for name in graph.keys() {
            let path = source.join(module_path(name));
            fs::create_dir_all(path.parent().ok_or("invalid module path")?)
                .map_err(|e| e.to_string())?;
            fs::write(path, source_for(&graph, name)?).map_err(|e| e.to_string())?;
        }
        let date = format!("2020-01-{:02}T00:00:00+0000", rev + 1);
        checked(
            Command::new("git")
                .arg("-C")
                .arg(source)
                .args(["add", "bench", "tests"]),
        )?;
        checked(
            Command::new("git")
                .arg("-C")
                .arg(source)
                .args([
                    "commit",
                    "-q",
                    "-m",
                    &format!("Generated workload revision {rev}"),
                ])
                .env("GIT_AUTHOR_NAME", "Benchmark")
                .env("GIT_COMMITTER_NAME", "Benchmark")
                .env("GIT_AUTHOR_EMAIL", "benchmark@example.invalid")
                .env("GIT_COMMITTER_EMAIL", "benchmark@example.invalid")
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date),
        )?;
        history.push(Revision {
            commit: git_string(source, &["rev-parse", "HEAD"])?,
            tree: git_string(source, &["rev-parse", "HEAD^{tree}"])?,
            graph,
            ordinal: rev,
        });
    }
    if !git_string(source, &["status", "--porcelain"])?.is_empty() {
        return refuse("generated Git fixture must be clean");
    }
    Ok(history)
}

fn indexed_edges(db: &Path, name: &str, commit: &str) -> Result<BTreeSet<(String, String)>> {
    let connection = Connection::open(db).map_err(|e| e.to_string())?;
    let mut statement = connection.prepare("SELECT d.source_name,d.target_name FROM dependency d JOIN repository r ON r.id=d.repo_id WHERE r.name=? AND d.commit_oid=?")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![name, commit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|e| e.to_string())
}

struct Query<'a> {
    binary: &'a Path,
    db: &'a Path,
    workload: &'a str,
    snapshot: &'a Revision,
    seed: &'a str,
    kind: &'a str,
    arm: &'a str,
    repetition: usize,
}

fn query(request: Query<'_>) -> Result<Value> {
    let Query { binary, db, workload, snapshot, seed, kind, arm, repetition } = request;
    let mut cmd = Command::new(binary);
    cmd.arg("view")
        .arg(if arm == "graph" { "graph" } else { "paths" })
        .args([
            "--db",
            db.to_str().ok_or("non-UTF8 DB path")?,
            "--name",
            workload,
            "--rev",
            &snapshot.commit,
            "--seed",
            seed,
        ]);
    if arm == "graph" {
        cmd.args(["--kind", kind]);
    } else {
        let role = KINDS
            .iter()
            .find(|(name, _)| *name == kind)
            .ok_or("invalid query kind")?
            .1;
        cmd.args(["--role", role]);
    }
    let observation = observe(&mut cmd)?;
    let mut trial = json!({
        "workload":workload, "revision":snapshot.ordinal, "commit":snapshot.commit,
        "seed":seed, "kind":kind, "arm":arm, "repetition":repetition,
        "wall_ms":observation.wall_ms, "child_cpu_ms":observation.child_cpu_ms,
        "stdout_bytes":observation.stdout.len(), "stdout_sha256":digest::sha256(&observation.stdout),
        "exit_code":observation.exit_code,
        "status":if observation.timeout { "timeout" } else if observation.exit_code == 0 { "ok" } else { "error" },
    });
    if observation.exit_code != 0 {
        trial["error"] = String::from_utf8_lossy(&observation.stderr)
            .chars()
            .take(400)
            .collect::<String>()
            .into();
        return Ok(trial);
    }
    let parsed = (|| -> Result<(Vec<String>, usize)> {
        let output: Value =
            serde_json::from_slice(&observation.stdout).map_err(|e| e.to_string())?;
        if output["source_commit"] != snapshot.commit || output["source_tree"] != snapshot.tree {
            return refuse("source revision/tree mismatch");
        }
        let raw_paths = output["source_paths"]
            .as_array()
            .ok_or("missing source_paths array")?;
        let mut paths = Vec::new();
        for path in raw_paths {
            paths.push(path.as_str().ok_or("non-string source path")?.to_string());
        }
        let count = paths.len();
        paths.sort();
        paths.dedup();
        Ok((paths, count))
    })();
    match parsed {
        Ok((paths, original_len)) => {
            let expected = oracle(&snapshot.graph, seed, kind)?;
            let good = paths == expected && paths.len() == original_len;
            trial["source_paths"] = json!(paths);
            trial["source_count"] = json!(paths.len());
            trial["declared_oracle_paths"] = json!(expected);
            trial["duplicate_paths"] = json!(original_len - paths.len());
            trial["exact_oracle_match"] = json!(good);
            if !good {
                trial["status"] = "oracle-mismatch".into();
            }
        }
        Err(message) => {
            trial["status"] = "invalid-output".into();
            trial["error"] = message.into();
        }
    }
    Ok(trial)
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-relations-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).map_err(|e| format!("create scratch directory: {e}"))?;
        Ok(Self(root))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct Job {
    revision: usize,
    seed: String,
    kind: &'static str,
    repetition: usize,
}

fn median(mut values: Vec<f64>) -> Result<f64> {
    if values.is_empty() {
        return refuse("cannot summarize empty observations");
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    Ok(round3(if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    }))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut output = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    output.push(b'\n');
    fs::write(path, output).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn run(config: &Config, binary: &Path) -> Result<PathBuf> {
    config.validate()?;
    if !binary.is_file() {
        return refuse("benchmark binary is missing");
    }
    fs::create_dir(&config.out).map_err(|e| format!("create benchmark output: {e}"))?;
    let output = config
        .out
        .canonicalize()
        .map_err(|e| format!("resolve output directory: {e}"))?;
    let scratch = Scratch::new()?;
    let mut observations: Vec<Value> = Vec::new();
    let mut workloads = Vec::new();
    for &n in &config.sizes {
        for &fanout in &config.fanouts {
            let name = format!("modules{n}_fanout{fanout}");
            let source = scratch.0.join(&name);
            let db = scratch.0.join(format!("{name}.sqlite"));
            let history = fixture(&source, n, fanout, config.revisions)?;
            let bundle_name = format!("{name}.bundle");
            let bundle = output.join(&bundle_name);
            checked(
                Command::new("git")
                    .arg("-C")
                    .arg(&source)
                    .args(["bundle", "create"])
                    .arg(&bundle)
                    .arg("--all"),
            )?;
            let bundle_sha = digest::sha256(&fs::read(&bundle).map_err(|e| e.to_string())?);
            let mut item = json!({"name":name,"size":n,"fanout":fanout,"source_bundle":bundle_name,"bundle_sha256":bundle_sha,"revisions":[]});
            for snapshot in &history {
                let index = checked(
                    Command::new(binary)
                        .arg("index")
                        .arg("--repo")
                        .arg(&source)
                        .arg("--db")
                        .arg(&db)
                        .arg("--name")
                        .arg(&name)
                        .arg("--rev")
                        .arg(&snapshot.commit)
                        .arg("--roots")
                        .arg("bench,tests"),
                )?;
                let expected: BTreeSet<(String, String)> = snapshot
                    .graph
                    .iter()
                    .flat_map(|(from, tos)| tos.iter().map(|to| (from.clone(), to.clone())))
                    .collect();
                let actual = indexed_edges(&db, &name, &snapshot.commit)?;
                let exact = expected == actual;
                let missing: Vec<_> = expected.difference(&actual).cloned().collect();
                let unexpected: Vec<_> = actual.difference(&expected).cloned().collect();
                item["revisions"].as_array_mut().ok_or("missing revisions array")?.push(json!({
                    "commit":snapshot.commit,"tree":snapshot.tree,
                    "index_ms":index.wall_ms,"index_cpu_ms":index.child_cpu_ms,
                    "index_bytes_cumulative":fs::metadata(&db).map_err(|e| e.to_string())?.len(),
                    "expected_edge_count":expected.len(),"indexed_edge_count":actual.len(),
                    "independently_specified_edges_equal":exact,"missing_edges":missing,"unexpected_edges":unexpected
                }));
                if !exact {
                    write_json(&output.join("coverage_failure.json"), &item)?;
                    return refuse(format!(
                        "indexed edges contradict independent fixture: {name} revision {}",
                        snapshot.ordinal
                    ));
                }
            }
            workloads.push(item);
            let mut jobs = Vec::new();
            for snapshot in &history {
                for index in [0, n / 2, n - 1] {
                    for (kind, _) in KINDS {
                        for repetition in 0..config.repetitions {
                            jobs.push(Job {
                                revision: snapshot.ordinal,
                                seed: module_name(index),
                                kind,
                                repetition,
                            });
                        }
                    }
                }
            }
            // Public SHA-256 permutation: recorded, deterministic workload ordering.
            jobs.sort_by_key(|job| {
                digest::sha256(
                    format!(
                        "20261111:{name}:{}:{}:{}:{}",
                        job.revision, job.seed, job.kind, job.repetition
                    )
                    .as_bytes(),
                )
            });
            for (pair_idx, job) in jobs.iter().enumerate() {
                let snapshot = &history[job.revision];
                let arms = if pair_idx % 2 == 0 {
                    ["graph", "view"]
                } else {
                    ["view", "graph"]
                };
                let mut pair = Vec::new();
                for arm in arms {
                    pair.push(query(Query {
                        binary,
                        db: &db,
                        workload: &name,
                        snapshot,
                        seed: &job.seed,
                        kind: job.kind,
                        arm,
                        repetition: job.repetition,
                    })?);
                }
                if pair.iter().all(|trial| trial["status"] == "ok")
                    && pair[0]["source_paths"] != pair[1]["source_paths"]
                {
                    pair[0]["status"] = "pair-mismatch".into();
                    pair[1]["status"] = "pair-mismatch".into();
                }
                observations.extend(pair);
            }
        }
    }
    let mut lines = File::create(output.join("trials.jsonl")).map_err(|e| e.to_string())?;
    for trial in &observations {
        serde_json::to_writer(&mut lines, trial).map_err(|e| e.to_string())?;
        lines.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    let mut counts = serde_json::Map::new();
    let mut med_wall = serde_json::Map::new();
    let mut med_size = serde_json::Map::new();
    let mut totals = serde_json::Map::new();
    for arm in ["graph", "view"] {
        let samples: Vec<&Value> = observations
            .iter()
            .filter(|trial| trial["arm"] == arm)
            .collect();
        let mut states = serde_json::Map::new();
        for state in [
            "ok",
            "timeout",
            "error",
            "invalid-output",
            "oracle-mismatch",
            "pair-mismatch",
        ] {
            states.insert(
                state.into(),
                json!(
                    samples
                        .iter()
                        .filter(|trial| trial["status"] == state)
                        .count()
                ),
            );
        }
        counts.insert(arm.into(), Value::Object(states));
        let wall: Vec<f64> = samples
            .iter()
            .map(|x| {
                x["wall_ms"]
                    .as_f64()
                    .ok_or("missing wall_ms sample".to_owned())
            })
            .collect::<Result<_>>()?;
        let bytes: Vec<f64> = samples
            .iter()
            .map(|x| {
                x["stdout_bytes"]
                    .as_f64()
                    .ok_or("missing stdout_bytes sample".to_owned())
            })
            .collect::<Result<_>>()?;
        med_wall.insert(arm.into(), json!(median(wall.clone())?));
        med_size.insert(arm.into(), json!(median(bytes)?));
        totals.insert(arm.into(), json!(round3(wall.iter().sum())));
    }
    let summary = json!({
        "schema":"excession.relational-retrieval.v1",
        "binary_sha256":digest::sha256(&fs::read(binary).map_err(|e| e.to_string())?),
        "sizes":config.sizes,"fanouts":config.fanouts,"revision_count":config.revisions,
        "repetitions":config.repetitions,"workloads":workloads,
        "query_count_per_arm":observations.len() / 2,
        "status_counts":counts,"median_wall_ms":med_wall,"median_raw_stdout_bytes":med_size,
        "total_query_wall_ms":totals,
        "limitations":"Specified synthetic Python import graphs only; no task-completion metric. Both arms execute one native Rust CLI call per identical query. Per-query costs exclude indexing, which is recorded for every revision; the graph arm verifies the Git tree but does not authenticate a derived view."
    });
    write_json(&output.join("summary.json"), &summary)?;
    let report = super::build_relational(&output)?;
    if observations.iter().any(|trial| trial["status"] != "ok") {
        return refuse(format!(
            "relational benchmark contains failed observations; see {}",
            report.display()
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_oracle_is_independent_and_revision_sensitive() {
        let old = graph_spec(8, 2, 0).unwrap();
        let changed = graph_spec(8, 2, 1).unwrap();
        assert_eq!(old.len(), 11);
        assert_eq!(
            old["bench.m0000"],
            [module_name(1), module_name(2)].into_iter().collect()
        );
        assert!(!old["bench.m0001"].contains("bench.m0000"));
        assert!(changed["bench.m0001"].contains("bench.m0000"));
        assert_eq!(
            oracle(&old, "bench.m0000", "focus").unwrap(),
            vec!["bench/m0000.py"]
        );
        assert_eq!(
            oracle(&old, "bench.m0000", "dependencies_direct").unwrap(),
            vec!["bench/m0001.py", "bench/m0002.py"]
        );
        assert_eq!(
            oracle(&old, "bench.m0000", "dependents_direct").unwrap(),
            vec!["tests/test_links.py"]
        );
        assert_eq!(
            oracle(&old, "bench.m0004", "tests").unwrap(),
            vec!["tests/test_links.py"]
        );
        assert!(oracle(&old, "missing", "focus").is_err());
    }
    #[test]
    fn invalid_dimensions_are_rejected_before_creating_output() {
        for (size, fanout) in [(0, 1), (3, 1), (8, 0), (8, 8)] {
            let cfg = Config {
                out: PathBuf::from("/tmp/unreachable-native-relational"),
                sizes: vec![size],
                fanouts: vec![fanout],
                revisions: 1,
                repetitions: 1,
            };
            assert!(cfg.validate().is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn generated_source_commits_replay() {
        let scratch = Scratch::new().unwrap();
        let source = scratch.0.join("source");
        let history = fixture(&source, 8, 2, 3).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(
            history
                .iter()
                .map(|s| &s.commit)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            history
                .iter()
                .map(|s| &s.tree)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            fs::read_to_string(source.join("bench/m0001.py")).unwrap(),
            source_for(&history[2].graph, "bench.m0001").unwrap()
        );
    }
}
