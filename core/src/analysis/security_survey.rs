//! Four-repository, advisory-blind native security survey.
//!
//! The Rust core authenticates the immutable Git source inventory, re-derives
//! source content identities and candidate classifications, and persists exact
//! observational evidence. The native adapter performs finite, loopback-only
//! HTTP tests. A counterexample is not a confirmed security vulnerability.
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, digest, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS security_survey_run (
    run_id TEXT PRIMARY KEY, source_commit TEXT NOT NULL,
    protocol_sha256 TEXT NOT NULL, observation_sha256 TEXT NOT NULL,
    source_count INTEGER NOT NULL, case_count INTEGER NOT NULL,
    finding_count INTEGER NOT NULL, state TEXT NOT NULL
    CHECK(state = 'BOUNDED_NATIVE_OBSERVATIONS')
);
CREATE TABLE IF NOT EXISTS security_survey_source (
    run_id TEXT NOT NULL REFERENCES security_survey_run(run_id),
    repo TEXT NOT NULL, source_json TEXT NOT NULL,
    PRIMARY KEY(run_id,repo)
);
CREATE TABLE IF NOT EXISTS security_survey_case (
    run_id TEXT NOT NULL REFERENCES security_survey_run(run_id),
    repo TEXT NOT NULL, invariant TEXT NOT NULL, case_name TEXT NOT NULL,
    verdict TEXT NOT NULL CHECK(verdict IN ('PASS','FAIL','UNSUPPORTED','ERROR')),
    evidence_json TEXT NOT NULL,
    PRIMARY KEY(run_id,repo,invariant,case_name)
);
CREATE TABLE IF NOT EXISTS security_survey_finding (
    run_id TEXT NOT NULL REFERENCES security_survey_run(run_id),
    repo TEXT NOT NULL, invariant TEXT NOT NULL, case_name TEXT NOT NULL,
    finding_json TEXT NOT NULL,
    PRIMARY KEY(run_id,repo,invariant,case_name)
);
"#;

fn hex_digit(d: u8) -> Option<u8> {
    match d {
        b'0'..=b'9' => Some(d - b'0'),
        b'a'..=b'f' => Some(d - b'a' + 10),
        b'A'..=b'F' => Some(d - b'A' + 10),
        _ => None,
    }
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    let raw = s.as_bytes();
    if raw.len() > 4096 || !raw.len().is_multiple_of(2) {
        return refuse("invalid encoded packet header length");
    }
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|c| {
            let high = hex_digit(c[0]).ok_or_else(|| "invalid hex byte".to_string())?;
            let low = hex_digit(c[1]).ok_or_else(|| "invalid hex byte".to_string())?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hash256_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|x| x.is_ascii_hexdigit())
}

fn walk_sources(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            return refuse("symlink in native source snapshot");
        }
        if ty.is_dir() {
            walk_sources(&path, ext, out)?;
        } else if ty.is_file() && path.extension().and_then(|x| x.to_str()) == Some(ext) {
            out.push(path);
        }
    }
    Ok(())
}

fn source_tree_hash(root: &Path, repo: &str) -> Result<(usize, String)> {
    let base = root.join("sources").join(repo);
    let ext = if repo == "postcss" { "js" } else { "py" };
    let mut entries = Vec::new();
    walk_sources(&base.join(repo), ext, &mut entries)?;
    entries.sort();
    if entries.is_empty() || entries.len() > 300 {
        return refuse("unsupported source inventory size");
    }
    let mut rows = Vec::new();
    for file in &entries {
        let relative = file.strip_prefix(&base).map_err(|e| e.to_string())?;
        let content = fs::read(file).map_err(|e| e.to_string())?;
        rows.push(format!(
            "{} {}",
            relative.display(),
            digest::sha256(&content)
        ));
    }
    Ok((entries.len(), digest::sha256(rows.join("\n").as_bytes())))
}

fn check_source_identity(root: &Path) -> Result<(String, String)> {
    git::clean(root)?;
    let commit = git::rev(root, "HEAD")?;
    let protocol = fs::read(root.join("protocol.json")).map_err(|e| e.to_string())?;
    let manifest = json::parse(&protocol)?;
    if manifest.get("protocol")?.str()? != "relgit-advisory-blind-four-repository-survey-v1" {
        return refuse("unsupported frozen survey protocol");
    }
    let selections = manifest.get("targets")?.array_ref()?;
    let expected = ["click", "h11", "postcss", "requests"];
    if selections.len() != expected.len() {
        return refuse("missing or additional selected repository");
    }
    for (item, name) in selections.iter().zip(expected) {
        if item.get("name")?.str()? != name {
            return refuse("survey selection order changed");
        }
        let baseline = item.get("commit")?.str()?;
        if baseline.len() != 40 || !baseline.bytes().all(|b| b.is_ascii_hexdigit()) {
            return refuse("invalid baseline commit identity");
        }
    }
    for p in ["scan.py", "protocol.json", "README.md"] {
        let expected = git::blob(root, "HEAD", p)?;
        let actual = git::text(root, &["hash-object", p])?;
        if expected != actual {
            return refuse(format!("survey manifest or scanner mutated: {p}"));
        }
    }
    Ok((commit, digest::sha256(&protocol)))
}

fn native_probe(root: &Path) -> Result<Value> {
    let out = Command::new("timeout")
        .args(["45s", "python3", "scan.py"])
        .current_dir(root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| format!("native survey launch: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "native survey refused ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if out.stdout.len() > 256 * 1024 {
        return refuse("native survey response too large");
    }
    json::parse(&out.stdout)
}

type Source = (String, String);
type Case = (String, String, String, String, String);
type Finding = (String, String, String, String);
fn validate_observations(
    root: &Path,
    data: &Value,
) -> Result<(Vec<Source>, Vec<Case>, Vec<Finding>)> {
    if data.get("schema")?.str()? != "relgit-advisory-blind-survey-v1" {
        return refuse("unexpected survey report schema");
    }
    let native_sources = data.get("sources")?.array_ref()?;
    let native_cases = data.get("observations")?.array_ref()?;
    let supplied_findings = data.get("findings")?.array_ref()?;
    let expected_repos = ["click", "h11", "postcss", "requests"];
    if native_sources.len() != expected_repos.len() || !(12..=256).contains(&native_cases.len()) {
        return refuse("survey missing mandatory subjects or cases");
    }
    let mut sources = Vec::new();
    for (item, repo) in native_sources.iter().zip(expected_repos) {
        if item.get("repo")?.str()? != repo {
            return refuse("source order or identity mismatch");
        }
        let (count, digest) = source_tree_hash(root, repo)?;
        if !hash256_valid(item.get("source_tree_sha256")?.str()?) {
            return refuse("invalid native source digest");
        }
        if item.get("source_count")?.integer()? != count as i64
            || item.get("source_tree_sha256")?.str()? != digest
        {
            return refuse(format!("native source tree mismatch: {repo}"));
        }
        let file = item.get("sample_file")?.str()?;
        if !file.starts_with(&format!("sources/{repo}/{repo}/")) {
            return refuse("sample file points outside repository");
        }
        let oid = git::blob(root, "HEAD", file)?;
        let actual = git::text(root, &["hash-object", file])?;
        if oid != actual || oid != item.get("sample_git_blob")?.str()? {
            return refuse(format!("unauthenticated Git source: {repo}"));
        }
        sources.push((repo.into(), item.canonical()));
    }

    let mut cases = Vec::new();
    let mut keys = BTreeSet::new();
    let mut fails = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in native_cases {
        let repo = item.get("repo")?.str()?;
        if !expected_repos.contains(&repo) {
            return refuse("case outside selected repositories");
        }
        let invariant = item.get("invariant")?.str()?;
        let name = item.get("case")?.str()?;
        if invariant.is_empty() || invariant.len() > 100 || name.is_empty() || name.len() > 100 {
            return refuse("invalid case identifier");
        }
        let verdict = item.get("verdict")?.str()?;
        if !["PASS", "FAIL", "UNSUPPORTED", "ERROR"].contains(&verdict) {
            return refuse("invalid native verdict");
        }
        if !keys.insert((repo.to_owned(), invariant.to_owned(), name.to_owned())) {
            return refuse("duplicate native case");
        }
        *counts.entry(repo.into()).or_default() += 1;
        let evidence = item.get("evidence")?;
        if invariant == "http-header-wire-control" {
            let header = hex_decode(evidence.get("header_hex")?.str()?)?;
            if digest::sha256(&header) != evidence.get("header_sha256")?.str()? {
                return refuse("raw native wire witness hash mismatch");
            }
            if !header.is_empty() && !header.starts_with(b"X-Test:") {
                return refuse("wire witness is not expected header");
            }
            let nul = header.contains(&0);
            if nul != evidence.get("nul_wire")?.bool()? {
                return refuse("wire witness NUL classification mismatch");
            }
            if evidence.get("received")?.bool()? != !header.is_empty() {
                return refuse("wire witness presence contradiction");
            }
            let expected_verdict = if header.is_empty() {
                "UNSUPPORTED"
            } else if nul || evidence.get("status")?.str()? != "200" {
                "FAIL"
            } else {
                "PASS"
            };
            if expected_verdict != verdict {
                return refuse("incorrect wire security classification");
            }
        }
        cases.push((
            repo.into(),
            invariant.into(),
            name.into(),
            verdict.into(),
            item.canonical(),
        ));
        if verdict == "FAIL" {
            fails.insert(
                (repo.to_owned(), invariant.to_owned(), name.to_owned()),
                obj(&[
                    ("repo", Value::string(repo)),
                    ("invariant", Value::string(invariant)),
                    ("case", Value::string(name)),
                    ("evidence", evidence.clone()),
                ])
                .canonical(),
            );
        }
    }
    if counts.len() != expected_repos.len() || counts["h11"] < 8 || counts["requests"] < 6 {
        return refuse("missing native coverage or unsupported controls");
    }
    cases.sort();
    let mut provided = BTreeMap::new();
    for finding in supplied_findings {
        let repo = finding.get("repo")?.str()?.to_owned();
        let name = finding.get("case")?.str()?.to_owned();
        let inv = finding.get("invariant")?.str()?.to_owned();
        if provided
            .insert((repo, inv, name), finding.canonical())
            .is_some()
        {
            return refuse("duplicate supplied finding");
        }
    }
    if fails != provided {
        return refuse("findings differ from independently derived FAIL cases");
    }
    let findings = fails
        .into_iter()
        .map(|((r, i, c), j)| (r, i, c, j))
        .collect();
    Ok((sources, cases, findings))
}

pub fn execute(root: &Path, db: &Db, audit: bool) -> Result<Value> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let (commit, protocol_hash) = check_source_identity(&root)?;
    let data = native_probe(&root)?;
    let (sources, cases, findings) = validate_observations(&root, &data)?;
    let (commit2, protocol_hash2) = check_source_identity(&root)?;
    if commit != commit2 || protocol_hash != protocol_hash2 {
        return refuse("survey source changed during native execution");
    }
    let observation_hash = data.sha256();
    let run_id =
        digest::sha256(format!("survey-v1:{commit}:{protocol_hash}:{observation_hash}").as_bytes());
    let header = vec![
        Some(run_id.clone()),
        Some(commit.clone()),
        Some(protocol_hash.clone()),
        Some(observation_hash.clone()),
        Some(sources.len().to_string()),
        Some(cases.len().to_string()),
        Some(findings.len().to_string()),
        Some("BOUNDED_NATIVE_OBSERVATIONS".into()),
    ];
    if audit {
        if db.query("SELECT run_id,source_commit,protocol_sha256,observation_sha256,source_count,case_count,finding_count,state FROM security_survey_run", &[])? != vec![header] {
            return refuse("survey run header tampering or drift");
        }
        let source_rows = sources
            .iter()
            .map(|(r, j)| vec![Some(run_id.clone()), Some(r.clone()), Some(j.clone())])
            .collect::<Vec<_>>();
        let case_rows = cases
            .iter()
            .map(|(r, i, c, v, j)| {
                vec![
                    Some(run_id.clone()),
                    Some(r.clone()),
                    Some(i.clone()),
                    Some(c.clone()),
                    Some(v.clone()),
                    Some(j.clone()),
                ]
            })
            .collect::<Vec<_>>();
        let finding_rows = findings
            .iter()
            .map(|(r, i, c, j)| {
                vec![
                    Some(run_id.clone()),
                    Some(r.clone()),
                    Some(i.clone()),
                    Some(c.clone()),
                    Some(j.clone()),
                ]
            })
            .collect::<Vec<_>>();
        for (sql, expected) in [
            (
                "SELECT run_id,repo,source_json FROM security_survey_source ORDER BY repo",
                source_rows,
            ),
            (
                "SELECT run_id,repo,invariant,case_name,verdict,evidence_json FROM security_survey_case ORDER BY repo,invariant,case_name",
                case_rows,
            ),
            (
                "SELECT run_id,repo,invariant,case_name,finding_json FROM security_survey_finding ORDER BY repo,invariant,case_name",
                finding_rows,
            ),
        ] {
            if db.query(sql, &[])? != expected {
                return refuse("independent survey observation table mismatch");
            }
        }
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query("SELECT run_id FROM security_survey_run", &[])?
                .is_empty()
            {
                return refuse("survey database must be initially empty");
            }
            db.execute(
                "INSERT INTO security_survey_run VALUES(?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&run_id),
                    Param::Text(&commit),
                    Param::Text(&protocol_hash),
                    Param::Text(&observation_hash),
                    Param::Integer(sources.len() as i64),
                    Param::Integer(cases.len() as i64),
                    Param::Integer(findings.len() as i64),
                    Param::Text("BOUNDED_NATIVE_OBSERVATIONS"),
                ],
            )?;
            for (r, j) in &sources {
                db.execute(
                    "INSERT INTO security_survey_source VALUES(?,?,?)",
                    &[Param::Text(&run_id), Param::Text(r), Param::Text(j)],
                )?;
            }
            for (r, i, c, v, j) in &cases {
                db.execute(
                    "INSERT INTO security_survey_case VALUES(?,?,?,?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(r),
                        Param::Text(i),
                        Param::Text(c),
                        Param::Text(v),
                        Param::Text(j),
                    ],
                )?;
            }
            for (r, i, c, j) in &findings {
                db.execute(
                    "INSERT INTO security_survey_finding VALUES(?,?,?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(r),
                        Param::Text(i),
                        Param::Text(c),
                        Param::Text(j),
                    ],
                )?;
            }
            Ok(())
        })?;
    }
    db.foreign_keys_valid()?;
    Ok(obj(&[
        ("state", Value::string("BOUNDED_NATIVE_OBSERVATIONS")),
        ("audited", Value::Bool(audit)),
        ("source_commit", Value::string(commit)),
        ("protocol_sha256", Value::string(protocol_hash)),
        ("observation_sha256", Value::string(observation_hash)),
        ("run_id", Value::string(run_id)),
        ("sources", Value::number(sources.len())),
        ("cases", Value::number(cases.len())),
        ("findings", Value::number(findings.len())),
        (
            "finding_evidence",
            Value::array(
                findings.iter().map(|(_, _, _, j)| {
                    json::parse(j.as_bytes()).expect("validated evidence JSON")
                }),
            ),
        ),
        (
            "claim_boundary",
            Value::string(
                "blind-selected native parser discrepancies; not exploitability proof or CVE mapping",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_hex_refuses_invalid_or_oversized_data() {
        assert_eq!(hex_decode("582d546573743a200074").unwrap(), b"X-Test: \0t");
        assert!(hex_decode("0xz").is_err());
        assert!(hex_decode("0").is_err());
        assert!(hex_decode(&"a".repeat(4098)).is_err());
    }
    #[test]
    fn sha256_validation() {
        assert!(hash256_valid(&"a".repeat(64)));
        assert!(!hash256_valid("abc"));
    }
}
