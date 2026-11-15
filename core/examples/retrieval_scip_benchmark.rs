//! Deep, model-free retrieval acceptance on an independently indexed Git tree.
//! This example never manufactures a relationship or uses SCIP to produce gold
//! labels. The labels file is revision-pinned and its source anchors are
//! authenticated against Git before any query is evaluated.
//!
//! cargo run --release --example retrieval_scip_benchmark --
//!   REPO COMMIT EVIDENCE_DB LABELS_JSON RESULTS_JSON
use excession::agent_locate;
use excession::git;
use excession::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const LIMIT: usize = 16;
const K: usize = 5;
const GRAPH_SLOTS: usize = 2;

fn source_paths(rows: &[Value]) -> Result<Vec<String>, String> {
    rows.iter()
        .map(|row| Ok(row.get("source")?.get("path")?.str()?.to_owned()))
        .collect()
}

fn hybrid(lexical: &[String], neighbors: &[String]) -> Vec<String> {
    let mut selected = Vec::<String>::new();
    let mut seen = BTreeSet::new();
    if let Some(first) = lexical.first() {
        seen.insert(first.clone());
        selected.push(first.clone());
    }
    for path in neighbors {
        if selected.len() >= 1 + GRAPH_SLOTS {
            break;
        }
        if seen.insert(path.clone()) {
            selected.push(path.clone());
        }
    }
    for path in lexical {
        if selected.len() >= K {
            break;
        }
        if seen.insert(path.clone()) {
            selected.push(path.clone());
        }
    }
    selected.truncate(K);
    selected
}

fn score(ranking: &[String], expected: &BTreeSet<String>) -> (usize, f64, f64) {
    let top = ranking.iter().take(K).collect::<Vec<_>>();
    let hits = top.iter().filter(|p| expected.contains(p.as_str())).count();
    let recall = if expected.is_empty() {
        0.0
    } else {
        hits as f64 / expected.len() as f64
    };
    let reciprocal_rank = top
        .iter()
        .position(|path| expected.contains(path.as_str()))
        .map_or(0.0, |index| 1.0 / (index + 1) as f64);
    (hits, recall, reciprocal_rank)
}

fn float(value: f64) -> Value {
    Value::number(format!("{value:.6}"))
}

fn verify_labels(
    root: &Path,
    commit: &str,
    labels: &Value,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    if labels.get("schema")?.str()? != "excession.retrieval-gold.v1" {
        return Err("unsupported retrieval label schema".into());
    }
    if labels.get("source_commit")?.str()? != commit {
        return Err("labels are pinned to a different Git commit".into());
    }
    if labels.get("review_status")?.str()? != "AUTHOR_SEEDED_UNREVIEWED" {
        return Err("labels cannot claim review without an independent adjudication record".into());
    }
    let mut validated = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for row in labels.get("cases")?.array_ref()? {
        let id = row.get("id")?.str()?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("duplicate label case: {id}"));
        }
        let query = row.get("query")?.str()?;
        if query.trim().len() < 6 || query.len() > 4096 {
            return Err(format!("invalid retrieval query: {id}"));
        }
        let mut expected = BTreeSet::new();
        for item in row.get("gold")?.array_ref()? {
            let path = item.get("path")?.str()?;
            let anchor = item.get("anchor")?.str()?;
            git::validate_path(path)?;
            if anchor.trim().len() < 4 {
                return Err(format!("missing independent source anchor: {id}"));
            }
            let actual_blob = git::blob(root, commit, path)?;
            if item.get("blob")?.str()? != actual_blob {
                return Err(format!("label path/blob mismatch: {id} {path}"));
            }
            let data = git::command(root, &["cat-file", "blob", &actual_blob])?;
            if !String::from_utf8_lossy(&data).contains(anchor) {
                return Err(format!("label source anchor not found: {id} {path}"));
            }
            expected.insert(path.to_owned());
        }
        validated.insert(id.to_owned(), expected);
    }
    if ids.len() < 10 {
        return Err("retrieval evaluation needs at least ten fixed, source-anchored cases".into());
    }
    Ok(validated)
}

fn evaluate(root: &Path, revision: &str, db: &str, labels_file: &Path) -> Result<Value, String> {
    let commit = git::rev(root, revision)?;
    let labels = json::parse(&fs::read(labels_file).map_err(|e| e.to_string())?)?;
    let expected = verify_labels(root, &commit, &labels)?;
    let mut rows = Vec::new();
    let mut total_expected = 0usize;
    let mut lexical_hits = 0usize;
    let mut hybrid_hits = 0usize;
    let mut lexical_mrr = 0.0;
    let mut hybrid_mrr = 0.0;
    let mut positive = 0usize;
    let mut negative = 0usize;
    let mut negative_lexical_matches = 0usize;
    let mut negative_hybrid_matches = 0usize;
    let mut cases_with_native_neighbors = 0usize;
    let mut eligible_native_edges = 0usize;
    let mut worst_case_ms = 0u128;
    let mut index_state: Option<Value> = None;
    for (number, row) in labels.get("cases")?.array_ref()?.iter().enumerate() {
        let id = row.get("id")?.str()?;
        let task = row.get("query")?.str()?;
        let tick = Instant::now();
        let report =
            agent_locate::locate_with_budget(root, &commit, task, LIMIT, None, Some(db), 256)?;
        let elapsed_ms = tick.elapsed().as_millis();
        worst_case_ms = worst_case_ms.max(elapsed_ms);
        if report.get("source_commit")?.str()? != commit {
            return Err("retrieval produced a mismatched commit".into());
        }
        if report.get("index")?.get("pending_segments")?.integer()? != 0 {
            return Err("benchmark attempted to rank a partially built index".into());
        }
        if number == 0 {
            index_state = Some(report.get("index")?.clone());
        }
        let base = source_paths(report.get("candidates")?.array_ref()?)?;
        let neighbor_records = report.get("relationship_candidates")?.array_ref()?;
        let mut neighbor_paths = Vec::new();
        for item in neighbor_records {
            let relation = item.get("relation")?;
            let assessment = item.get("assessment")?.str()?;
            if relation.get("kind")?.str()? == "SCIP_REFERENCE" {
                if assessment != "OBSERVED" || relation.get("evidence")?.array_ref()?.is_empty() {
                    return Err("invalid native SCIP evidence at retrieval boundary".into());
                }
                eligible_native_edges += 1;
            }
            neighbor_paths.push(item.get("source")?.get("path")?.str()?.to_owned());
        }
        if neighbor_records.iter().any(|x| {
            x.get("relation")
                .and_then(|r| r.get("kind"))
                .and_then(Value::str)
                .ok()
                == Some("SCIP_REFERENCE")
        }) {
            cases_with_native_neighbors += 1;
        }
        let alt = hybrid(&base, &neighbor_paths);
        let gold = expected.get(id).ok_or("unknown label case")?;
        let (base_hits, _, base_rr) = score(&base, gold);
        let (alt_hits, _, alt_rr) = score(&alt, gold);
        if gold.is_empty() {
            negative += 1;
            negative_lexical_matches += usize::from(!base.is_empty());
            negative_hybrid_matches += usize::from(!alt.is_empty());
        } else {
            positive += 1;
            total_expected += gold.len();
            lexical_hits += base_hits;
            hybrid_hits += alt_hits;
            lexical_mrr += base_rr;
            hybrid_mrr += alt_rr;
        }
        rows.push(json::obj(&[
            ("id", Value::string(id)),
            ("task", Value::string(task)),
            ("gold", Value::array(gold.iter().map(Value::string))),
            (
                "lexical",
                Value::array(base.iter().take(K).map(Value::string)),
            ),
            ("hybrid", Value::array(alt.iter().map(Value::string))),
            (
                "native_neighbor_count",
                Value::number(neighbor_records.len()),
            ),
            ("lexical_hits", Value::number(base_hits)),
            ("hybrid_hits", Value::number(alt_hits)),
            ("runtime_ms", Value::number(elapsed_ms)),
        ]));
    }
    // Absence of edges is not a success: the benchmark must ingest and use a
    // genuine cross-file SCIP corpus or fail rather than comparing two copies
    // of lexical retrieval under a "graph" name.
    if eligible_native_edges == 0 || cases_with_native_neighbors == 0 {
        return Err("no native SCIP cross-file references reached evaluated queries".into());
    }
    if positive == 0 || total_expected == 0 {
        return Err("retrieval benchmark has no positive labels".into());
    }
    let result = json::obj(&[
        ("schema", Value::string("excession.retrieval-benchmark.v1")),
        ("source_commit", Value::string(&commit)),
        (
            "source_tree",
            Value::string(git::rev(root, &format!("{commit}^{{tree}}"))?),
        ),
        ("review_status", labels.get("review_status")?.clone()),
        (
            "interpretation",
            Value::string("DESCRIPTIVE_AUTHOR_SEEDED_NOT_INDEPENDENTLY_VALIDATED"),
        ),
        (
            "ranking_policy",
            Value::string("LEXICAL_TOP_1_THEN_UP_TO_2_GRAPH_NEIGHBORS_THEN_LEXICAL"),
        ),
        ("k", Value::number(K)),
        ("cases", Value::array(rows)),
        ("positive_cases", Value::number(positive)),
        ("negative_cases", Value::number(negative)),
        ("gold_path_count", Value::number(total_expected)),
        (
            "lexical_recall_at_k",
            float(lexical_hits as f64 / total_expected as f64),
        ),
        (
            "hybrid_recall_at_k",
            float(hybrid_hits as f64 / total_expected as f64),
        ),
        ("lexical_mrr_at_k", float(lexical_mrr / positive as f64)),
        ("hybrid_mrr_at_k", float(hybrid_mrr / positive as f64)),
        (
            "negative_lexical_any_match",
            Value::number(negative_lexical_matches),
        ),
        (
            "negative_hybrid_any_match",
            Value::number(negative_hybrid_matches),
        ),
        (
            "cases_with_native_scip_neighbors",
            Value::number(cases_with_native_neighbors),
        ),
        (
            "observed_native_edges",
            Value::number(eligible_native_edges),
        ),
        ("max_query_ms", Value::number(worst_case_ms)),
        ("index", index_state.unwrap_or(Value::Null)),
    ]);
    Ok(result)
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 5 {
        eprintln!("usage: retrieval_scip_benchmark REPO REV EVIDENCE_DB LABELS_JSON RESULTS_JSON");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[0]);
    match evaluate(&root, &args[1], &args[2], Path::new(&args[3])) {
        Ok(report) => {
            let destination = Path::new(&args[4]);
            if let Err(error) = fs::write(destination, report.canonical() + "\n") {
                eprintln!("write results: {error}");
                std::process::exit(1);
            }
            println!("{}", report.canonical());
        }
        Err(error) => {
            eprintln!("retrieval evaluation failed: {error}");
            std::process::exit(1);
        }
    }
}
