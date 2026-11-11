//! Contract/evidence projection into the same Git/SQLite catalog.
//! Existing Python contract logic is an authenticated native verifier;
//! Rust checks pinned sources, re-executes observations, checks the certificate,
//! and indexes bounded claims without interpreting a test as a universal proof.
use crate::sqlite::{Db, Param};
use crate::{Result, digest, git, index, json, refuse};
use json::{Value, obj};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS contract_source_authority (
 name TEXT PRIMARY KEY, git_blob_oid TEXT NOT NULL, purpose TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS contract_spec (
 id TEXT PRIMARY KEY, origin TEXT NOT NULL REFERENCES contract_source_authority(name),
 entrypoint TEXT NOT NULL, scope TEXT NOT NULL, requires_json TEXT NOT NULL,
 guarantees_json TEXT NOT NULL, source_implementation TEXT,
 pinned_implementation_oid TEXT
);
CREATE TABLE IF NOT EXISTS contract_dependency (
 contract_id TEXT NOT NULL REFERENCES contract_spec(id),
 prerequisite_id TEXT NOT NULL REFERENCES contract_spec(id),
 origin TEXT NOT NULL REFERENCES contract_source_authority(name),
 PRIMARY KEY(contract_id,prerequisite_id), CHECK(contract_id<>prerequisite_id)
);
CREATE TABLE IF NOT EXISTS contract_closure (
 contract_id TEXT NOT NULL REFERENCES contract_spec(id), path TEXT NOT NULL,
 source_repo INTEGER NOT NULL REFERENCES repository(id),
 source_commit TEXT NOT NULL, blob_oid TEXT NOT NULL,
 PRIMARY KEY(contract_id,path),
 FOREIGN KEY(source_repo,source_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(source_repo,source_commit,path,blob_oid) REFERENCES tree_entry(repo_id,commit_oid,path,oid)
);
CREATE TABLE IF NOT EXISTS bounded_experiment (
 id TEXT PRIMARY KEY, baseline_repo INTEGER NOT NULL REFERENCES repository(id),
 baseline_commit TEXT NOT NULL,candidate_repo INTEGER NOT NULL REFERENCES repository(id),
 candidate_commit TEXT NOT NULL, corpus_sha256 TEXT NOT NULL,
 case_count INTEGER NOT NULL CHECK(case_count>0), certificate_sha256 TEXT NOT NULL,
 certificate_json TEXT NOT NULL, provenance_kind TEXT NOT NULL CHECK(provenance_kind='fresh_probe_plus_pinned_native_checker'),
 FOREIGN KEY(baseline_repo,baseline_commit) REFERENCES revision(repo_id,commit_oid),
 FOREIGN KEY(candidate_repo,candidate_commit) REFERENCES revision(repo_id,commit_oid)
);
CREATE TABLE IF NOT EXISTS bounded_claim (
 id TEXT PRIMARY KEY, contract_id TEXT NOT NULL REFERENCES contract_spec(id),
 scope_relation TEXT NOT NULL CHECK(scope_relation='related_bounded_scope_not_equivalence')
);
CREATE TABLE IF NOT EXISTS bounded_result (
 experiment_id TEXT NOT NULL REFERENCES bounded_experiment(id),
 claim_id TEXT NOT NULL REFERENCES bounded_claim(id),
 status TEXT NOT NULL CHECK(status IN ('FRESHLY_VERIFIED_WITHIN_CORPUS',
   'CARRIED_BY_SOURCE_IDENTITY_AND_RESTRICTED_AST_CHANGE',
   'CARRIED_BY_FINITE_TRACE_CONGRUENCE')),
 kind TEXT NOT NULL CHECK(kind IN ('fresh_observation','source_identity','derived_congruence')),
 rule TEXT NOT NULL,PRIMARY KEY(experiment_id,claim_id)
);
CREATE TABLE IF NOT EXISTS bounded_trace (
 experiment_id TEXT NOT NULL REFERENCES bounded_experiment(id),
 ordinal INTEGER NOT NULL, case_id TEXT NOT NULL,
 writer_trace_sha256 TEXT NOT NULL, reader_observed INTEGER NOT NULL CHECK(reader_observed=1),
 candidate_equal INTEGER NOT NULL CHECK(candidate_equal=1),
 composition_derived INTEGER NOT NULL CHECK(composition_derived=1),
 PRIMARY KEY(experiment_id,ordinal), UNIQUE(experiment_id,case_id)
);
CREATE TABLE IF NOT EXISTS bounded_premise (
 experiment_id TEXT NOT NULL, claim_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
 premise TEXT NOT NULL, bounded_prerequisite TEXT REFERENCES bounded_claim(id),
 PRIMARY KEY(experiment_id,claim_id,ordinal),
 FOREIGN KEY(experiment_id,claim_id) REFERENCES bounded_result(experiment_id,claim_id)
);
"#;

#[derive(Clone)]
pub struct Spec {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub vendor: PathBuf,
    pub contracts: PathBuf,
    pub links: PathBuf,
    pub baseline_observations: PathBuf,
    pub candidate_observations: PathBuf,
    pub certificate: PathBuf,
}
impl Spec {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        let canonical = |name| -> Result<PathBuf> {
            PathBuf::from(v.get(name)?.str()?)
                .canonicalize()
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            baseline: canonical("baseline")?,
            candidate: canonical("candidate")?,
            vendor: canonical("vendor")?,
            contracts: canonical("contracts")?,
            links: canonical("links")?,
            baseline_observations: canonical("baseline_observations")?,
            candidate_observations: canonical("candidate_observations")?,
            certificate: canonical("certificate")?,
        })
    }
}
const SOURCES: [(&str, &str); 5] = [
    ("check.py", "728ed4d46fcb08a16f93952fe55bd2ae4e6570ff"),
    ("domain.py", "4d4c52e81d800da8ebff86a6e7ae4f6271df19ae"),
    ("probe.py", "73e1bde94e332d8afa75d7cbda97191d4b16e3dc"),
    ("contracts.json", "e4c57fc8cceb27acd2fb81b3f8e8a38aa9116998"),
    (
        "h11_claim_links.json",
        "b83a75867e7c8141b5acdb9b3dd8488bf658da9f",
    ),
];
fn check_sources(spec: &Spec) -> Result<()> {
    for (name, oid) in SOURCES {
        let path = match name {
            "contracts.json" => &spec.contracts,
            "h11_claim_links.json" => &spec.links,
            _ => &spec.vendor.join(name),
        };
        let data = fs::read(path).map_err(|e| e.to_string())?;
        if digest::git_blob(&data) != oid {
            return refuse(format!("unpinned native verification source: {name}"));
        }
    }
    Ok(())
}
fn native_probe(spec: &Spec, repo: &Path, role: &str, out: &Path) -> Result<Value> {
    let output = Command::new("python")
        .arg("-B")
        .arg(spec.vendor.join("probe.py"))
        .args([
            "--root",
            &repo.to_string_lossy(),
            "--role",
            role,
            "--output",
            &out.to_string_lossy(),
        ])
        .current_dir(repo)
        .env(
            "PYTHONPATH",
            format!("{}:{}", repo.display(), spec.vendor.display()),
        )
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return refuse(format!(
            "native h11 probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    json::parse(&fs::read(out).map_err(|e| e.to_string())?)
}
fn native_certificate(spec: &Spec, generated: &Path) -> Result<Value> {
    let output = Command::new("python")
        .arg("-B")
        .arg(spec.vendor.join("check.py"))
        .arg("--baseline")
        .arg(&spec.baseline)
        .arg("--candidate")
        .arg(&spec.candidate)
        .arg("--baseline-observation")
        .arg(&spec.baseline_observations)
        .arg("--candidate-observation")
        .arg(&spec.candidate_observations)
        .arg("--output")
        .arg(generated)
        .env("PYTHONPATH", &spec.vendor)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return refuse(format!(
            "pinned native checker refused evidence: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    json::parse(&fs::read(generated).map_err(|e| e.to_string())?)
}
fn source_closure(spec: &Value) -> Result<Vec<String>> {
    spec.get("import_closure")?
        .array_ref()?
        .iter()
        .map(|x| Ok(x.str()?.to_owned()))
        .collect()
}
fn expected_status(name: &str) -> Result<&'static str> {
    match name {
        "chunked.writer.bounded" => Ok("FRESHLY_VERIFIED_WITHIN_CORPUS"),
        "chunked.reader.bounded" => Ok("CARRIED_BY_SOURCE_IDENTITY_AND_RESTRICTED_AST_CHANGE"),
        "chunked.composition.bounded" => Ok("CARRIED_BY_FINITE_TRACE_CONGRUENCE"),
        _ => refuse("unsupported bounded claim"),
    }
}
fn kind(name: &str) -> Result<&'static str> {
    match name {
        "chunked.writer.bounded" => Ok("fresh_observation"),
        "chunked.reader.bounded" => Ok("source_identity"),
        "chunked.composition.bounded" => Ok("derived_congruence"),
        _ => refuse("unsupported bounded claim"),
    }
}
fn field_str<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k)?.str()
}
struct CheckedRevisions<'a> {
    baseline_repo: i64,
    candidate_repo: i64,
    baseline: &'a str,
    candidate: &'a str,
}
fn record(
    db: &Db,
    spec: &Spec,
    cert: &Value,
    original: &Value,
    links: &Value,
    revs: &CheckedRevisions<'_>,
) -> Result<Value> {
    let specs = original.get("contracts")?.array_ref()?;
    let linked = links.get("claims")?.array_ref()?;
    if field_str(original.get("source")?, "commit")? != field_str(links, "source_commit")?
        || field_str(original.get("source")?, "commit")?
            != "62c5068c971579d61fa1b55373390e12f25fd856"
    {
        return refuse("contract origin not pinned upstream h11");
    }
    if specs.len() != 3 || linked.len() != 3 {
        return refuse("unexpected contract graph size");
    }
    let graph = linked
        .iter()
        .map(|x| {
            Ok((
                field_str(x, "id")?.to_owned(),
                x.get("depends_on")?
                    .array_ref()?
                    .iter()
                    .map(|p| Ok(p.str()?.to_owned()))
                    .collect::<Result<Vec<_>>>()?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    if graph.len() != 3 {
        return refuse("duplicate contract identifier");
    }
    let mut specs_ids = BTreeSet::new();
    for s in specs {
        specs_ids.insert(field_str(s, "id")?.to_owned());
    }
    if specs_ids != graph.keys().cloned().collect() {
        return refuse("authored graph does not match contract specifications");
    }
    for deps in graph.values() {
        if deps.iter().any(|d| !specs_ids.contains(d)) {
            return refuse("unknown semantic prerequisite");
        }
    }
    let mut ideals = 0;
    for subset in 0..(1usize << specs_ids.len()) {
        let selected = specs_ids
            .iter()
            .enumerate()
            .filter(|(i, _)| subset & (1 << i) != 0)
            .map(|(_, x)| x.as_str())
            .collect::<BTreeSet<_>>();
        if selected
            .iter()
            .all(|x| graph[*x].iter().all(|d| selected.contains(d.as_str())))
        {
            ideals += 1;
        }
    }
    if ideals != 5 {
        return refuse("contract prerequisite graph changed");
    }
    let origin_scope = field_str(original, "scope")?;
    let evidence_id = cert.sha256();
    let source = cert.get("source")?;
    if field_str(source, "baseline_commit")? != revs.baseline
        || field_str(source, "candidate_commit")? != revs.candidate
    {
        return refuse("certificate Git revision mismatch");
    }
    let cases = cert.get("cases")?.array_ref()?;
    let scope = cert.get("scope")?;
    if scope.get("case_count")?.integer()? != 112 || cases.len() != 112 {
        return refuse("finite corpus extent mismatch");
    }
    if !source.get("writer_body_changed")?.bool()?
        || !source
            .get("reader_and_its_module_initializers_unchanged")?
            .bool()?
    {
        return refuse("source-preservation scope not established");
    }
    let old = git::changed(&spec.candidate, revs.baseline, revs.candidate)?;
    if old != vec!["h11/_writers.py"] {
        return refuse("undeclared source change");
    }
    let allclaims = cert.get("claims")?.array_ref()?;
    if allclaims.len() != 3 {
        return refuse("expected three distinct bounded claims");
    }
    let mut claimids = BTreeSet::new();
    for c in allclaims {
        let id = field_str(c, "id")?;
        if !claimids.insert(id.to_owned()) || field_str(c, "status")? != expected_status(id)? {
            return refuse("duplicated or upgraded bounded evidence status");
        }
    }
    let mapping = BTreeMap::from([
        ("chunked.reader.bounded", "http1.chunked.reader"),
        ("chunked.writer.bounded", "http1.chunked.writer"),
        ("chunked.composition.bounded", "http1.chunked.composition"),
    ]);
    db.exec(SCHEMA)?;
    db.transaction(|db| {
        for (name, oid) in SOURCES {
            db.execute(
                "INSERT INTO contract_source_authority VALUES(?,?,?)",
                &[
                    Param::Text(name),
                    Param::Text(oid),
                    Param::Text("pinned Git object from h11 research"),
                ],
            )?;
        }
        for s in specs {
            let id = field_str(s, "id")?;
            let implementation = s.get("implementation")?;
            let (path, oid) = match (implementation, s.get("blob_sha")?) {
                (Value::String(path), Value::String(oid)) => {
                    git::validate_path(path)?;
                    if git::blob(&spec.baseline, revs.baseline, path)? != *oid {
                        return refuse(format!("declared implementation Git blob differs: {id}"));
                    }
                    (Some(path.as_str()), Some(oid.as_str()))
                }
                (Value::Null, Value::Null) => (None, None),
                _ => return refuse("inconsistent contract implementation"),
            };
            db.execute(
                "INSERT INTO contract_spec VALUES(?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(id),
                    Param::Text("contracts.json"),
                    Param::Text(field_str(s, "entrypoint")?),
                    Param::Text(origin_scope),
                    Param::Text(&s.get("requires")?.canonical()),
                    Param::Text(&s.get("guarantees")?.canonical()),
                    Param::Text(path.unwrap_or("")),
                    Param::Text(oid.unwrap_or("")),
                ],
            )?;
            for file in source_closure(s)? {
                git::validate_path(&file)?;
                let oid = git::blob(&spec.baseline, revs.baseline, &file)?;
                db.execute(
                    "INSERT INTO contract_closure VALUES(?,?,?,?,?)",
                    &[
                        Param::Text(id),
                        Param::Text(&file),
                        Param::Integer(revs.baseline_repo),
                        Param::Text(revs.baseline),
                        Param::Text(&oid),
                    ],
                )?;
            }
        }
        for (id, dependencies) in &graph {
            for dependency in dependencies {
                db.execute(
                    "INSERT INTO contract_dependency VALUES(?,?,?)",
                    &[
                        Param::Text(id),
                        Param::Text(dependency),
                        Param::Text("h11_claim_links.json"),
                    ],
                )?;
            }
        }
        let corpus = field_str(scope, "corpus_sha256")?;
        db.execute(
            "INSERT INTO bounded_experiment VALUES(?,?,?,?,?,?,?,?,?,?)",
            &[
                Param::Text(&evidence_id),
                Param::Integer(revs.baseline_repo),
                Param::Text(revs.baseline),
                Param::Integer(revs.candidate_repo),
                Param::Text(revs.candidate),
                Param::Text(corpus),
                Param::Integer(112),
                Param::Text(&evidence_id),
                Param::Text(&cert.canonical()),
                Param::Text("fresh_probe_plus_pinned_native_checker"),
            ],
        )?;
        for (bound, contract) in mapping {
            db.execute(
                "INSERT INTO bounded_claim VALUES(?,?,?)",
                &[
                    Param::Text(bound),
                    Param::Text(contract),
                    Param::Text("related_bounded_scope_not_equivalence"),
                ],
            )?;
        }
        for claim in allclaims {
            let name = field_str(claim, "id")?;
            db.execute(
                "INSERT INTO bounded_result VALUES(?,?,?,?,?)",
                &[
                    Param::Text(&evidence_id),
                    Param::Text(name),
                    Param::Text(expected_status(name)?),
                    Param::Text(kind(name)?),
                    Param::Text(field_str(claim, "rule")?),
                ],
            )?;
            if let Ok(premises) = claim.get("premises") {
                for (i, p) in premises.array_ref()?.iter().enumerate() {
                    let premise = p.str()?;
                    let prerequisite = if claimids.contains(premise) {
                        premise
                    } else {
                        ""
                    };
                    db.execute(
                        "INSERT INTO bounded_premise VALUES(?,?,?,?,?)",
                        &[
                            Param::Text(&evidence_id),
                            Param::Text(name),
                            Param::Integer(i as i64),
                            Param::Text(premise),
                            if prerequisite.is_empty() {
                                Param::Null
                            } else {
                                Param::Text(prerequisite)
                            },
                        ],
                    )?;
                }
            }
        }
        for (i, row) in cases.iter().enumerate() {
            if field_str(row, "case")?.is_empty()
                || !row.get("baseline_reader_satisfied")?.bool()?
                || !row.get("candidate_writer_equal")?.bool()?
                || !row.get("composition_retained_by_congruence")?.bool()?
            {
                return refuse("invalid finite trace claim");
            }
            let hash = field_str(row, "writer_trace_sha256")?;
            if hash.len() != 64 || !hash.bytes().all(|x| x.is_ascii_hexdigit()) {
                return refuse("invalid trace digest");
            }
            db.execute(
                "INSERT INTO bounded_trace VALUES(?,?,?,?,?,?,?)",
                &[
                    Param::Text(&evidence_id),
                    Param::Integer(i as i64),
                    Param::Text(field_str(row, "case")?),
                    Param::Text(hash),
                    Param::Integer(1),
                    Param::Integer(1),
                    Param::Integer(1),
                ],
            )?;
        }
        db.foreign_keys_valid()
    })?;
    Ok(obj(&[
        ("certificate_sha256", Value::string(evidence_id)),
        ("trace_count", Value::number(112)),
        ("contracts", Value::number(3)),
        ("contract_ideals", Value::number(ideals)),
        (
            "claims",
            Value::array(
                allclaims
                    .iter()
                    .map(|c| {
                        obj(&[
                            ("id", c.get("id").cloned().unwrap_or(Value::Null)),
                            ("status", c.get("status").cloned().unwrap_or(Value::Null)),
                        ])
                    })
                    .collect::<Vec<_>>(),
            ),
        ),
        (
            "provenance",
            Value::string("pinned Git blob sources plus fresh native probes and checker"),
        ),
    ]))
}

pub fn import(db: &Db, spec: &Spec) -> Result<Value> {
    check_sources(spec)?;
    git::clean(&spec.baseline)?;
    git::clean(&spec.candidate)?;
    let baseline = git::rev(&spec.baseline, "HEAD")?;
    let candidate = git::rev(&spec.candidate, "HEAD")?;
    if git::text(&spec.candidate, &["rev-parse", "HEAD^"])? != baseline {
        return refuse("candidate must be direct descendant");
    }
    let old = json::parse(&fs::read(&spec.baseline_observations).map_err(|e| e.to_string())?)?;
    let new = json::parse(&fs::read(&spec.candidate_observations).map_err(|e| e.to_string())?)?;
    let certificate = json::parse(&fs::read(&spec.certificate).map_err(|e| e.to_string())?)?;
    let base_dir = spec.baseline.parent().ok_or("baseline has no parent")?;
    let scratch = base_dir.join(format!(".excession-contract-check-{}", std::process::id()));
    if scratch.exists() {
        return refuse("contract scratch directory exists");
    }
    fs::create_dir(&scratch).map_err(|e| e.to_string())?;
    let validate = || -> Result<()> {
        if native_probe(
            spec,
            &spec.baseline,
            "baseline",
            &scratch.join("fresh-base.json"),
        )? != old
        {
            return refuse("baseline observations do not match fresh pinned probe");
        }
        if native_probe(
            spec,
            &spec.candidate,
            "candidate",
            &scratch.join("fresh-candidate.json"),
        )? != new
        {
            return refuse("candidate observations do not match fresh pinned probe");
        }
        if native_certificate(spec, &scratch.join("recomputed.json"))? != certificate {
            return refuse("certificate differs from recomputation by pinned checker");
        }
        Ok(())
    };
    let validated = validate();
    let _ = fs::remove_dir_all(&scratch);
    validated?;
    let baseline_repo = index::index_revision_with_roots(
        db,
        &spec.baseline,
        "h11-contract-baseline",
        &baseline,
        "preservation baseline",
        &["h11"],
    )?;
    let candidate_repo = index::index_revision_with_roots(
        db,
        &spec.candidate,
        "h11-contract-candidate",
        &candidate,
        "preservation candidate",
        &["h11"],
    )?;
    let contracts = json::parse(&fs::read(&spec.contracts).map_err(|e| e.to_string())?)?;
    let links = json::parse(&fs::read(&spec.links).map_err(|e| e.to_string())?)?;
    record(
        db,
        spec,
        &certificate,
        &contracts,
        &links,
        &CheckedRevisions {
            baseline_repo,
            candidate_repo,
            baseline: &baseline,
            candidate: &candidate,
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_is_not_inferred() {
        assert!(expected_status("made.up.contract").is_err());
        assert_eq!(kind("chunked.writer.bounded").unwrap(), "fresh_observation");
    }
}
