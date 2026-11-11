//! Source-bound benchmark reporting, governed by the versioned JSON Schema.
//!
//! The schema validates structure, not the truth of the benchmark's oracle.
//! Raw observations remain the evidence of what was actually measured.
use crate::{Result, digest, json, refuse};
use serde_json::{Value, json as document};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

const SCHEMA: &str = include_str!("../../benchmarks/schemas/benchmark-report.v1.schema.json");
const REPORT_SCHEMA: &str = "excession.benchmark-report.v1";

fn read(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    // Do not let serde silently accept duplicate keys in evidence receipts.
    json::parse(&bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| format!("missing field {key}"))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| format!("{key} is not a string"))
}
fn integer(value: &Value, key: &str) -> Result<u64> {
    field(value, key)?
        .as_u64()
        .ok_or_else(|| format!("{key} is not a nonnegative integer"))
}
fn number(value: &Value, key: &str) -> Result<f64> {
    let result = field(value, key)?
        .as_f64()
        .ok_or_else(|| format!("{key} is not a number"))?;
    if !result.is_finite() || result < 0.0 {
        return refuse(format!("invalid numeric value for {key}"));
    }
    Ok(result)
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    field(value, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{key} is not an array"))
}
fn boolean(value: &Value, key: &str) -> Result<bool> {
    field(value, key)?
        .as_bool()
        .ok_or_else(|| format!("{key} is not a boolean"))
}
fn metric(
    name: &str,
    arm: &str,
    unit: &str,
    statistic: &str,
    value: Value,
    count: u64,
) -> Result<Value> {
    if count == 0 || value.as_f64().is_none_or(|n| !n.is_finite() || n < 0.0) {
        return refuse(
            "measured metrics require a finite nonnegative value and positive sample count",
        );
    }
    Ok(document!({
        "name": name, "arm": arm, "unit": unit, "statistic": statistic,
        "value": value, "sample_count": count, "status": "measured"
    }))
}
fn file_digest(path: &Path) -> Result<String> {
    Ok(digest::sha256(
        &fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?,
    ))
}
fn safe_artifact_path(base: &Path, name: &str) -> Result<std::path::PathBuf> {
    let rel = Path::new(name);
    if name.is_empty()
        || !rel
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return refuse(format!("unsafe artifact path {name:?}"));
    }
    let root = base
        .canonicalize()
        .map_err(|e| format!("canonicalize {}: {e}", base.display()))?;
    let source = base
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("resolve artifact {name:?}: {e}"))?;
    if !source.starts_with(&root) || !source.is_file() {
        return refuse(format!("artifact escapes report directory: {name:?}"));
    }
    Ok(source)
}
fn artifact(base: &Path, name: &str, role: &str, media_type: &str) -> Result<Value> {
    let source = safe_artifact_path(base, name)?;
    let size = source.metadata().map_err(|e| e.to_string())?.len();
    Ok(document!({
        "path": name, "role": role, "media_type": media_type,
        "sha256": file_digest(&source)?, "bytes": size,
    }))
}
fn identity(report: &Value) -> Result<String> {
    let mut body = report.clone();
    body.as_object_mut()
        .ok_or("report must be an object")?
        .remove("run_id");
    let serialized = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let canonical = json::parse(&serialized)?.canonical();
    Ok(digest::sha256(canonical.as_bytes()))
}
fn schema_validator() -> Result<jsonschema::Validator> {
    let schema: Value = serde_json::from_str(SCHEMA).map_err(|e| e.to_string())?;
    jsonschema::draft202012::new(&schema).map_err(|e| format!("compile benchmark schema: {e}"))
}
fn validate_value(report: &Value, base: &Path) -> Result<()> {
    let validator = schema_validator()?;
    if let Err(error) = validator.validate(report) {
        return refuse(format!(
            "invalid benchmark report: {error} at {}",
            error.instance_path()
        ));
    }
    if string(report, "run_id")? != identity(report)? {
        return refuse("report run_id differs from normalized content identity");
    }
    let mut unique = BTreeSet::new();
    for item in array(report, "artifacts")? {
        let path = string(item, "path")?;
        if !unique.insert(path.to_owned()) {
            return refuse(format!("duplicate artifact path: {path}"));
        }
        let expected = artifact(
            base,
            path,
            string(item, "role")?,
            string(item, "media_type")?,
        )?;
        if &expected != item {
            return refuse(format!("artifact digest or size mismatch: {path}"));
        }
    }
    unique.clear();
    for item in array(report, "measurements")? {
        let name = string(item, "name")?;
        let arm = string(item, "arm")?;
        let statistic = string(item, "statistic")?;
        if !unique.insert(format!("{arm}/{name}/{statistic}")) {
            return refuse(format!(
                "duplicate measurement identity: {arm}/{name}/{statistic}"
            ));
        }
    }
    unique.clear();
    for item in array(report, "source_revisions")? {
        let key = format!(
            "{}/{}/{}",
            string(item, "name")?,
            string(item, "commit")?,
            string(item, "tree")?
        );
        if !unique.insert(key) {
            return refuse("duplicate source revision");
        }
    }
    if string(report, "status")? == "passed"
        && array(report, "checks")?
            .iter()
            .any(|check| check["status"] != "passed")
    {
        return refuse("passed report contains failed or unsupported check");
    }
    Ok(())
}

fn report_parent(path: &Path) -> &Path {
    // A filename such as "native-calibration.report.json" has an empty parent,
    // which is not a resolvable directory. Treat it as the current directory.
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub fn validate(path: &Path) -> Result<()> {
    let report = read(path)?;
    validate_value(&report, report_parent(path))
}

fn environment() -> Value {
    let mut value = document!({
        "system": std::env::consts::OS,
        "machine": std::env::consts::ARCH,
    });
    if let Ok(runner) = std::env::var("RUNNER_NAME")
        && !runner.is_empty()
    {
        value["runner"] = runner.into();
    }
    value
}
fn write_report(directory: &Path, name: &str, mut report: Value) -> Result<std::path::PathBuf> {
    if !name.ends_with(".json")
        || Path::new(name).file_name().and_then(|x| x.to_str()) != Some(name)
    {
        return refuse("report filename must be a plain JSON filename");
    }
    report["run_id"] = identity(&report)?.into();
    validate_value(&report, directory)?;
    let dest = directory.join(name);
    let temporary = directory.join(format!(".{name}.tmp"));
    let mut output = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    output.push(b'\n');
    fs::write(&temporary, output).map_err(|e| format!("write temporary report: {e}"))?;
    if let Err(e) = fs::rename(&temporary, &dest) {
        let _ = fs::remove_file(&temporary);
        return refuse(format!("publish report: {e}"));
    }
    validate(&dest)?;
    Ok(dest)
}

pub fn build_native(directory: &Path, input: &str) -> Result<std::path::PathBuf> {
    let source = safe_artifact_path(directory, input)?;
    if source.parent() != directory.canonicalize().ok().as_deref() {
        return refuse("native calibration input must be a file in its report directory");
    }
    if !input.ends_with(".json")
        || !input
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return refuse("native calibration name must be a plain JSON filename");
    }
    let raw = read(&source)?;
    if string(&raw, "schema")? != "excession.native-paired-calibration.v1" {
        return refuse("expected native paired calibration v1");
    }
    let paired = integer(&raw, "paired_queries")?;
    let parity = integer(&raw, "source_set_matches")? == paired;
    if paired == 0 {
        return refuse("native calibration requires at least one pair");
    }
    let samples = array(&raw, "samples")?;
    let mut measurements = Vec::new();
    for arm in ["graph", "paths"] {
        let count = samples.iter().filter(|s| s["arm"] == arm).count() as u64;
        if count != paired {
            return refuse(format!("native calibration samples missing for {arm}"));
        }
        let summary = field(field(&raw, "summary")?, arm)?;
        for (name, unit, key) in [
            ("query_wall_time", "ms", "wall_ms"),
            ("child_cpu_time", "ms", "child_cpu_ms"),
            ("stdout_size", "bytes", "stdout_bytes"),
        ] {
            let item = field(summary, key)?;
            measurements.push(metric(
                name,
                arm,
                unit,
                "median",
                document!(number(item, "median")?),
                count,
            )?);
            measurements.push(metric(
                name,
                arm,
                unit,
                "p95",
                document!(number(item, "nearest_rank_p95")?),
                count,
            )?);
        }
    }
    let name = input
        .strip_suffix(".json")
        .ok_or("expected JSON filename")?;
    let report = document!({
        "schema": REPORT_SCHEMA, "suite": "native-paired-calibration",
        "status": if parity { "passed" } else { "failed" },
        "subject": {"name": "excession", "binary_sha256": string(&raw, "binary_sha256")?},
        "oracle": {
            "kind": "shared-index", "independent": false,
            "description": "Path sets derive from the same indexed source and authenticated full view, not an independent semantic oracle"
        },
        "source_revisions": [{
            "name": "h11", "commit": string(&raw, "source_commit")?,
            "tree": string(&raw, "source_tree")?,
            "bundle_sha256": string(&raw, "bundle_sha256")?,
        }],
        "environment": environment(),
        "configuration": {"paired_queries": paired, "repetitions": integer(&raw, "repetitions")?},
        "measurements": measurements,
        "checks": [{
            "name": "source_set_parity",
            "description": "Both native arms returned source sets matching the authenticated full view",
            "sample_count": paired,
            "status": if parity { "passed" } else { "failed" },
        }],
        "artifacts": [artifact(directory, input, "raw-samples", "application/json")?],
        "limitations": [string(&raw, "limitations")?],
    });
    write_report(directory, &format!("{name}.report.json"), report)
}

fn paired_key(trial: &Value) -> Result<String> {
    Ok(format!(
        "{}:{}:{}:{}:{}:{}",
        string(trial, "workload")?,
        integer(trial, "revision")?,
        string(trial, "commit")?,
        string(trial, "seed")?,
        string(trial, "kind")?,
        integer(trial, "repetition")?
    ))
}
pub fn build_relational(directory: &Path) -> Result<std::path::PathBuf> {
    let summary = read(&directory.join("summary.json"))?;
    if string(&summary, "schema")? != "excession.relational-retrieval.v1" {
        return refuse("expected relational retrieval v1 summary");
    }
    let trials_raw = fs::read_to_string(directory.join("trials.jsonl"))
        .map_err(|e| format!("read trials: {e}"))?;
    let mut pairs: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut trials = Vec::new();
    for (line, text) in trials_raw.lines().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        json::parse(text.as_bytes())
            .map_err(|e| format!("invalid trial line {}: {e}", line + 1))?;
        let trial: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let arm = string(&trial, "arm")?;
        if !matches!(arm, "graph" | "view") {
            return refuse(format!("invalid trial arm: {arm}"));
        }
        let entry = pairs.entry(paired_key(&trial)?).or_default();
        if entry.insert(arm.to_owned(), trial.clone()).is_some() {
            return refuse("duplicate arm for paired query");
        }
        trials.push(trial);
    }
    let count = integer(&summary, "query_count_per_arm")?;
    if count == 0 || trials.len() as u64 != count * 2 {
        return refuse("trial count disagrees with raw summary");
    }
    let mut sources = Vec::new();
    let mut checks = Vec::new();
    let mut artifacts = vec![
        artifact(directory, "summary.json", "summary", "application/json")?,
        artifact(
            directory,
            "trials.jsonl",
            "raw-samples",
            "application/x-ndjson",
        )?,
    ];
    let mut index_ms = 0.0;
    let mut index_cpu_ms = 0.0;
    let mut index_count = 0;
    for workload in array(&summary, "workloads")? {
        let bundle = string(workload, "source_bundle")?;
        let source_bundle = artifact(
            directory,
            bundle,
            "source-bundle",
            "application/octet-stream",
        )?;
        if source_bundle["sha256"].as_str() != Some(string(workload, "bundle_sha256")?) {
            return refuse(format!("bundle digest does not match summary: {bundle}"));
        }
        artifacts.push(source_bundle);
        for (i, revision) in array(workload, "revisions")?.iter().enumerate() {
            let label = format!("{}-r{i}", string(workload, "name")?);
            sources.push(document!({
                "name": label, "commit": string(revision, "commit")?,
                "tree": string(revision, "tree")?,
                "bundle_sha256": string(workload, "bundle_sha256")?,
            }));
            let good = boolean(revision, "independently_specified_edges_equal")?;
            checks.push(document!({
                "name": "indexed_edges_match_source_spec",
                "source": label,
                "description": "Indexed edges equal the independently authored fixture graph",
                "status": if good { "passed" } else { "failed" },
            }));
            index_ms += number(revision, "index_ms")?;
            index_cpu_ms += number(revision, "index_cpu_ms")?;
            index_count += 1;
        }
    }
    let paired = pairs.len() as u64 == count
        && pairs.values().all(|arms| {
            if arms.len() != 2 {
                return false;
            }
            let Some(graph) = arms.get("graph") else {
                return false;
            };
            let Some(view) = arms.get("view") else {
                return false;
            };
            graph["status"] == "ok"
                && view["status"] == "ok"
                && graph["exact_oracle_match"] == true
                && view["exact_oracle_match"] == true
                && graph["source_paths"] == view["source_paths"]
        });
    checks.push(document!({
        "name": "source_written_oracle_and_pair_parity",
        "description": "Both arms match the source-written oracle and each other for every paired query",
        "sample_count": pairs.len(),
        "status": if paired { "passed" } else { "failed" },
    }));
    let mut measurements = Vec::new();
    for arm in ["graph", "view"] {
        let found = trials
            .iter()
            .filter(|trial| trial["arm"] == arm)
            .collect::<Vec<_>>();
        if found.len() as u64 != count {
            return refuse(format!("missing trials for {arm}"));
        }
        let declared = field(field(&summary, "status_counts")?, arm)?;
        for (state, amount) in declared
            .as_object()
            .ok_or("status_counts must be an object")?
        {
            if found
                .iter()
                .filter(|trial| trial["status"] == *state)
                .count() as u64
                != amount.as_u64().ok_or("invalid status count")?
            {
                return refuse(format!("summary status count disagrees for {arm}/{state}"));
            }
        }
        let stats = |key: &str| -> Result<Value> { Ok(field(field(&summary, key)?, arm)?.clone()) };
        measurements.push(metric(
            "query_wall_time",
            arm,
            "ms",
            "median",
            stats("median_wall_ms")?,
            count,
        )?);
        measurements.push(metric(
            "query_wall_time",
            arm,
            "ms",
            "sum",
            stats("total_query_wall_ms")?,
            count,
        )?);
        measurements.push(metric(
            "stdout_size",
            arm,
            "bytes",
            "median",
            stats("median_raw_stdout_bytes")?,
            count,
        )?);
        measurements.push(metric(
            "query_count",
            arm,
            "count",
            "count",
            document!(count),
            count,
        )?);
    }
    measurements.push(metric(
        "index_wall_time",
        "index",
        "ms",
        "sum",
        document!((index_ms * 1000.0).round() / 1000.0),
        index_count,
    )?);
    measurements.push(metric(
        "index_cpu_time",
        "index",
        "ms",
        "sum",
        document!((index_cpu_ms * 1000.0).round() / 1000.0),
        index_count,
    )?);
    let passed = checks.iter().all(|check| check["status"] == "passed");
    let report = document!({
        "schema": REPORT_SCHEMA, "suite": "relational-retrieval",
        "status": if passed { "passed" } else { "failed" },
        "subject": {"name": "excession", "binary_sha256": string(&summary, "binary_sha256")?},
        "oracle": {
            "kind": "source-written", "independent": true,
            "description": "Expected import edges and paths are defined by the fixture generator before indexing"
        },
        "source_revisions": sources, "environment": environment(),
        "configuration": {
            "sizes": field(&summary, "sizes")?, "fanouts": field(&summary, "fanouts")?,
            "revisions": integer(&summary, "revision_count")?,
            "repetitions": integer(&summary, "repetitions")?,
        },
        "measurements": measurements, "checks": checks, "artifacts": artifacts,
        "limitations": [string(&summary, "limitations")?],
    });
    write_report(directory, "report.json", report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "excession-benchmark-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn native_input(path: &Path) {
        let stats = document!({
            "wall_ms": {"median": 1.25, "nearest_rank_p95": 1.5},
            "child_cpu_ms": {"median": 0.75, "nearest_rank_p95": 1.0},
            "stdout_bytes": {"median": 240, "nearest_rank_p95": 320}
        });
        let receipt = document!({
            "schema": "excession.native-paired-calibration.v1",
            "binary_sha256": "a".repeat(64), "bundle_sha256": "b".repeat(64),
            "source_commit": "c".repeat(40), "source_tree": "d".repeat(40),
            "paired_queries": 2, "source_set_matches": 2, "repetitions": 2,
            "summary": {"graph": stats, "paths": stats},
            "samples": [{"arm": "graph"}, {"arm": "paths"}, {"arm": "graph"}, {"arm": "paths"}],
            "limitations": "Protocol test fixture only, no empirical correctness claimed."
        });
        fs::write(
            path.join("native.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn plain_relative_report_filename_uses_working_directory() {
        assert_eq!(
            report_parent(Path::new("native-calibration.report.json")),
            Path::new(".")
        );
        assert_eq!(
            report_parent(Path::new("sub/report.json")),
            Path::new("sub")
        );
    }
    #[test]
    fn schema_is_valid_draft_2020_12() {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        assert!(jsonschema::draft202012::meta::is_valid(&schema));
        schema_validator().unwrap();
    }
    #[test]
    fn native_roundtrip_and_raw_integrity() {
        let temp = Temp::new();
        native_input(temp.path());
        let report = build_native(temp.path(), "native.json").unwrap();
        validate(&report).unwrap();
        let content = read(&report).unwrap();
        assert_eq!(content["status"], "passed");
        assert_eq!(content["oracle"]["independent"], false);
        assert_eq!(content["measurements"].as_array().unwrap().len(), 12);
        assert_eq!(
            content["artifacts"][0]["sha256"],
            file_digest(&temp.path().join("native.json")).unwrap()
        );
        fs::write(temp.path().join("native.json"), b"{}").unwrap();
        assert!(
            validate(&report)
                .unwrap_err()
                .contains("digest or size mismatch")
        );
    }
    #[test]
    fn reject_invalid_fields_and_false_success() {
        let temp = Temp::new();
        native_input(temp.path());
        let path = build_native(temp.path(), "native.json").unwrap();
        let original = read(&path).unwrap();
        for mut value in [
            {
                let mut v = original.clone();
                v["measurements"][0]["unit"] = "invented".into();
                v
            },
            {
                let mut v = original.clone();
                v["measurements"][0]["value"] = document!(-1);
                v
            },
            {
                let mut v = original.clone();
                v["artifacts"][0]["path"] = "../native.json".into();
                v
            },
            {
                let mut v = original.clone();
                v["checks"][0]["status"] = "failed".into();
                v
            },
        ] {
            value["run_id"] = identity(&value).unwrap().into();
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(validate(&path).is_err());
        }
    }
    #[test]
    fn reject_forged_metric_and_unavailable_value_without_reason() {
        let temp = Temp::new();
        native_input(temp.path());
        let path = build_native(temp.path(), "native.json").unwrap();
        let mut value = read(&path).unwrap();
        value["measurements"][0]["value"] = document!(9000);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(validate(&path).unwrap_err().contains("run_id"));
        value["measurements"][0]["status"] = "not_measured".into();
        value["measurements"][0]
            .as_object_mut()
            .unwrap()
            .remove("value");
        value["run_id"] = identity(&value).unwrap().into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(validate(&path).is_err());
        value["measurements"][0]["reason"] = "Not observed by this runner".into();
        value["run_id"] = identity(&value).unwrap().into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        validate(&path).unwrap();
    }
    #[test]
    fn cannot_escape_directory_or_accept_non_finite_json() {
        let temp = Temp::new();
        native_input(temp.path());
        assert!(build_native(temp.path(), "../native.json").is_err());
        fs::write(temp.path().join("native.json"), b"{\"value\":NaN}").unwrap();
        assert!(build_native(temp.path(), "native.json").is_err());
    }
}
