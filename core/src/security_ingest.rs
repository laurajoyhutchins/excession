//! Git-authenticated static-analysis candidate ingestion and independent SQL replay.
//! SARIF/JSON from scanners is untrusted *candidate evidence*, never a proof of a CVE.
use crate::digest;
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS security_ingest_run (
 run_id TEXT PRIMARY KEY, source_commit TEXT NOT NULL,
 report_sha256 TEXT NOT NULL, engine TEXT NOT NULL,
 candidate_count INTEGER NOT NULL, status TEXT NOT NULL
 CHECK(status='UNVERIFIED_STATIC_CANDIDATES')
);
CREATE TABLE IF NOT EXISTS security_ingest_candidate (
 run_id TEXT NOT NULL REFERENCES security_ingest_run(run_id),
 candidate_id TEXT NOT NULL, logical_key TEXT NOT NULL,
 source_path TEXT NOT NULL, source_blob TEXT NOT NULL,
 line INTEGER NOT NULL, rule_id TEXT NOT NULL,
 raw_evidence TEXT NOT NULL,
 PRIMARY KEY(run_id,candidate_id)
);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    id: String,
    logical_key: String,
    path: String,
    blob: String,
    line: i64,
    rule: String,
    evidence: String,
}
fn normalize_path(p: &str) -> Result<String> {
    let value = p.strip_prefix("./").unwrap_or(p);
    if value.starts_with("file:")
        || value.contains('%')
        || value.contains('?')
        || value.contains('#')
    {
        return refuse(
            "unsupported SARIF URI encoding (only plain checkout-relative paths accepted)",
        );
    }
    git::validate_path(value)?;
    Ok(value.to_owned())
}
fn line(value: &Value) -> Result<i64> {
    let l = value.integer()?;
    if !(1..=1_000_000).contains(&l) {
        return refuse("invalid result line number");
    }
    Ok(l)
}
fn rows_from_report(repo: &Path, engine: &str, report: &Value) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let mut append = |path: &str, line: i64, rule: &str, evidence: &Value| -> Result<()> {
        if rule.is_empty() || rule.len() > 300 {
            return refuse("invalid rule id");
        }
        let path = normalize_path(path)?;
        let blob = git::blob(repo, "HEAD", &path)?;
        let mode = git::text(repo, &["ls-tree", "HEAD", "--", &path])?;
        if !mode.starts_with("100644 ") && !mode.starts_with("100755 ") {
            return refuse("source path is not a regular tracked file");
        }
        // Avoid accidentally attributing a static finding to a changed/dirty blob.
        let actual = git::text(repo, &["hash-object", &path])?;
        if actual != blob {
            return refuse("source blob mismatched checkout");
        }
        let bytes = fs::read(repo.join(&path)).map_err(|e| e.to_string())?;
        if bytes.split(|b| *b == b'\n').count() < line as usize {
            return refuse("finding outside source file");
        }
        let logical_key = digest::sha256(format!("{engine}\0{rule}\0{path}\0{line}").as_bytes());
        let evidence = evidence.canonical();
        let id = digest::sha256(format!("{logical_key}\0{blob}\0{evidence}").as_bytes());
        if !seen.insert(id.clone()) {
            return refuse("duplicate candidate");
        }
        candidates.push(Candidate {
            id,
            logical_key,
            path,
            blob,
            line,
            rule: rule.to_owned(),
            evidence,
        });
        Ok(())
    };
    match engine {
        "codeql" => {
            if report.get("version")?.str()? != "2.1.0" {
                return refuse("expected SARIF 2.1.0");
            }
            let runs = report.get("runs")?.array_ref()?;
            if runs.len() != 1 {
                return refuse("expected exactly one CodeQL SARIF run");
            }
            let run = &runs[0];
            let name = run.get("tool")?.get("driver")?.get("name")?.str()?;
            if !name.eq_ignore_ascii_case("codeql") {
                return refuse("SARIF tool is not CodeQL");
            }
            if let Some(invocations) = run.entries()?.get("invocations") {
                for invocation in invocations.array_ref()? {
                    if let Some(ok) = invocation.entries()?.get("executionSuccessful")
                        && !ok.bool()?
                    {
                        return refuse("CodeQL reported an unsuccessful analysis");
                    }
                }
            }
            for result in run.get("results")?.array_ref()? {
                let rule = result.get("ruleId")?.str()?;
                let location = result
                    .get("locations")?
                    .array_ref()?
                    .first()
                    .ok_or("SARIF result lacks primary location")?;
                let phys = location.get("physicalLocation")?;
                let path = phys.get("artifactLocation")?.get("uri")?.str()?;
                let lineno = line(phys.get("region")?.get("startLine")?)?;
                append(path, lineno, rule, result)?;
            }
        }
        "semgrep" => {
            if let Some(errors) = report.entries()?.get("errors")
                && !errors.array_ref()?.is_empty()
            {
                return refuse("Semgrep report contains scan errors");
            }
            for result in report.get("results")?.array_ref()? {
                let rule = result.get("check_id")?.str()?;
                let path = result.get("path")?.str()?;
                let lineno = line(result.get("start")?.get("line")?)?;
                append(path, lineno, rule, result)?;
            }
        }
        _ => return refuse("unsupported scanner: expected codeql or semgrep"),
    }
    if candidates.len() > 10_000 {
        return refuse("candidate count limit exceeded");
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(candidates)
}

fn read_expected(
    repo: &Path,
    report_path: &Path,
    engine: &str,
) -> Result<(String, String, String, Vec<Candidate>)> {
    git::clean(repo)?;
    let commit = git::rev(repo, "HEAD")?;
    let contents = fs::read(report_path).map_err(|e| e.to_string())?;
    if contents.len() > 16 * 1024 * 1024 {
        return refuse("scanner report exceeds 16 MiB");
    }
    let hash = digest::sha256(&contents);
    let val = json::parse(&contents)?;
    let rows = rows_from_report(repo, engine, &val)?;
    let run = digest::sha256(format!("relgit-security-v1\0{commit}\0{hash}\0{engine}").as_bytes());
    Ok((run, commit, hash, rows))
}

pub fn execute(repo: &Path, report: &Path, db: &Db, engine: &str, audit: bool) -> Result<Value> {
    let (run_id, commit, report_sha, candidates) = read_expected(repo, report, engine)?;
    if !audit {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            db.execute("INSERT INTO security_ingest_run VALUES(?1,?2,?3,?4,?5,'UNVERIFIED_STATIC_CANDIDATES')", &[
                Param::Text(&run_id), Param::Text(&commit), Param::Text(&report_sha),
                Param::Text(engine), Param::Integer(candidates.len() as i64),
            ])?;
            for c in &candidates {
                db.execute("INSERT INTO security_ingest_candidate VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", &[
                    Param::Text(&run_id), Param::Text(&c.id), Param::Text(&c.logical_key),
                    Param::Text(&c.path), Param::Text(&c.blob), Param::Integer(c.line),
                    Param::Text(&c.rule), Param::Text(&c.evidence),
                ])?;
            }
            Ok(())
        })?;
    } else {
        let runs = db.query("SELECT source_commit,report_sha256,engine,candidate_count,status FROM security_ingest_run WHERE run_id=?", &[Param::Text(&run_id)])?;
        let expected = vec![
            Some(commit.clone()),
            Some(report_sha.clone()),
            Some(engine.to_owned()),
            Some(candidates.len().to_string()),
            Some("UNVERIFIED_STATIC_CANDIDATES".into()),
        ];
        if runs != vec![expected] {
            return refuse("run metadata mismatch");
        }
        let stored = db.query("SELECT candidate_id,logical_key,source_path,source_blob,line,rule_id,raw_evidence FROM security_ingest_candidate WHERE run_id=? ORDER BY candidate_id", &[Param::Text(&run_id)])?;
        let reconstructed = candidates
            .iter()
            .map(|c| {
                vec![
                    Some(c.id.clone()),
                    Some(c.logical_key.clone()),
                    Some(c.path.clone()),
                    Some(c.blob.clone()),
                    Some(c.line.to_string()),
                    Some(c.rule.clone()),
                    Some(c.evidence.clone()),
                ]
            })
            .collect::<Vec<_>>();
        if stored != reconstructed {
            return refuse("candidate evidence differs from independent replay");
        }
        db.foreign_keys_valid()?;
    }
    Ok(obj(&[
        ("schema", Value::string("relgit.security.ingest.receipt.v1")),
        ("run_id", Value::string(run_id)),
        ("source_commit", Value::string(commit)),
        ("report_sha256", Value::string(report_sha)),
        ("engine", Value::string(engine)),
        ("candidate_count", Value::number(candidates.len())),
        ("status", Value::string("UNVERIFIED_STATIC_CANDIDATES")),
        ("independently_replayed", Value::Bool(audit)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_paths_only() {
        for x in [
            "/etc/passwd",
            "../foo",
            "a//b",
            "a/./b",
            "a\\b",
            "file:///x",
            "a%20b",
        ] {
            assert!(normalize_path(x).is_err(), "{x}");
        }
        assert_eq!(normalize_path("./src/main.go").unwrap(), "src/main.go");
    }
    #[test]
    fn scanners_are_separate() {
        assert!(rows_from_report(Path::new("/tmp"), "osv", &Value::Null).is_err());
    }
    #[test]
    fn line_range_checks() {
        assert!(line(&Value::number(0)).is_err());
        assert!(line(&Value::number(1000001)).is_err());
        assert_eq!(line(&Value::number(1)).unwrap(), 1);
    }
}
