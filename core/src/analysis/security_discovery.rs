//! CVE-label-independent native security boundary discovery.
//!
//! Rust authenticates Git-pinned native code, reruns the bounded native probe,
//! verifies source bindings and all observation relations, and stores exact
//! findings in SQLite. It does not call itself an exploitability proof.
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS security_discovery_run (
    run_id TEXT PRIMARY KEY, checkout_commit TEXT NOT NULL,
    observation_sha256 TEXT NOT NULL, implementation TEXT NOT NULL,
    runtime_version TEXT NOT NULL, source_count INTEGER NOT NULL,
    case_count INTEGER NOT NULL, finding_count INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state='NATIVE_BOUNDARY_OBSERVATIONS')
);
CREATE TABLE IF NOT EXISTS security_discovery_source (
    run_id TEXT NOT NULL REFERENCES security_discovery_run(run_id),
    module TEXT NOT NULL, source_json TEXT NOT NULL,
    PRIMARY KEY(run_id,module)
);
CREATE TABLE IF NOT EXISTS security_discovery_case (
    run_id TEXT NOT NULL REFERENCES security_discovery_run(run_id),
    module TEXT NOT NULL, policy TEXT NOT NULL, member TEXT NOT NULL,
    observation_json TEXT NOT NULL,
    PRIMARY KEY(run_id,module,policy,member)
);
CREATE TABLE IF NOT EXISTS security_discovery_finding (
    run_id TEXT NOT NULL REFERENCES security_discovery_run(run_id),
    module TEXT NOT NULL, evidence_json TEXT NOT NULL,
    PRIMARY KEY(run_id,module)
);
"#;

fn native_scan(root: &Path) -> Result<Value> {
    let output = Command::new("timeout")
        .args(["45s", "python3", "scan_archives.py"])
        .current_dir(root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| format!("native security scanner launch: {e}"))?;
    if !output.status.success() {
        return refuse(format!(
            "native security scanner refused ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if output.stdout.len() > 256 * 1024 {
        return refuse("native security scan output exceeds limit");
    }
    json::parse(&output.stdout)
}

fn check_tracked(root: &Path) -> Result<String> {
    git::clean(root)?;
    let commit = git::rev(root, "HEAD")?;
    for name in [
        "scan_archives.py",
        "sources/tarfile.py",
        "sources/zipfile.py",
        "sources/LICENSE.Python.txt",
        "README.md",
    ] {
        let expected = git::blob(root, "HEAD", name)?;
        let actual = git::text(root, &["hash-object", name])?;
        if expected != actual {
            return refuse(format!("security scanner source changed: {name}"));
        }
    }
    Ok(commit)
}

type SourceRow = (String, String);
type ObservationRow = (String, String, String, String);
type FindingRow = (String, String);
type ValidatedRows = (Vec<SourceRow>, Vec<ObservationRow>, Vec<FindingRow>);

fn verified_write_location(location: &str, under_destination: bool) -> Result<()> {
    if location.starts_with('/')
        || location.contains('\0')
        || location.split('/').any(|p| matches!(p, "" | "." | ".."))
    {
        return refuse("unsafe observed output path");
    }
    if under_destination != location.starts_with("destination/") {
        return refuse("observation containment flag inconsistent with output location");
    }
    Ok(())
}

fn check_result(root: &Path, data: &Value) -> Result<ValidatedRows> {
    if data.get("schema")?.str()? != "relgit-native-boundary-discovery-v1" {
        return refuse("unsupported security discovery schema");
    }
    if data.get("runtime")?.get("implementation")?.str()? != "CPython" {
        return refuse("unsupported native Python implementation");
    }
    let version = data.get("runtime")?.get("version")?.str()?;
    if !version.starts_with("3.") || version.len() > 30 {
        return refuse("invalid native runtime version");
    }
    let sources = data.get("sources")?.array_ref()?;
    let cases = data.get("observations")?.array_ref()?;
    let findings = data.get("findings")?.array_ref()?;
    if !(2..=16).contains(&sources.len()) || !(8..=256).contains(&cases.len()) {
        return refuse("unsupported security inventory or probe coverage");
    }
    let mut src = Vec::new();
    let mut source_names = BTreeMap::new();
    for item in sources {
        let module = item.get("module")?.str()?;
        if module.len() > 80
            || !module
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return refuse("invalid native module identifier");
        }
        if source_names.contains_key(module) {
            return refuse("duplicate native source");
        }
        let git_path = format!("sources/{module}.py");
        let oid = git::blob(root, "HEAD", &git_path)?;
        if item.get("source_git_blob")?.str()? != oid {
            return refuse(format!("source Git blob not authenticated: {module}"));
        }
        let digest = item.get("source_sha256")?.str()?;
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return refuse("invalid source SHA-256");
        }
        let entrypoint = item.get("entrypoint")?.str()?;
        if entrypoint.len() > 120 || !entrypoint.ends_with(".extractall") {
            return refuse("unsupported public extraction entrypoint");
        }
        let methods = item.get("reachable_methods")?.array_ref()?;
        if !methods.iter().any(|v| v.str().ok() == Some("extractall")) {
            return refuse("untraced native extraction method");
        }
        let paths = item.get("path_operations")?.array_ref()?;
        if paths.is_empty() {
            return refuse("no reachable path operations");
        }
        source_names.insert(module.to_string(), entrypoint.to_string());
        src.push((module.to_string(), item.canonical()));
    }
    src.sort();

    let mut obs = Vec::new();
    let mut seen = BTreeSet::new();
    let mut default_escapes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut safe_controls = 0usize;
    for v in cases {
        let module = v.get("module")?.str()?;
        if !source_names.contains_key(module) {
            return refuse("observation refers to untracked module");
        }
        let policy = v.get("policy")?.str()?;
        if policy != "default" && policy != "data-filter" {
            return refuse("unexpected extraction policy");
        }
        let member = v.get("member")?.str()?;
        if member.is_empty()
            || member.len() > 180
            || member.starts_with('/')
            || member.contains('\0')
        {
            return refuse("invalid archive member candidate");
        }
        let key = (module.to_string(), policy.to_string(), member.to_string());
        if !seen.insert(key.clone()) {
            return refuse("duplicate native security observation");
        }
        let writes = v.get("writes")?.array_ref()?;
        let mut escape = false;
        let mut contained = !writes.is_empty();
        for w in writes {
            let loc = w.get("relative_to_sandbox")?.str()?;
            let under = w.get("under_destination")?.bool()?;
            verified_write_location(loc, under)?;
            let hash = w.get("sha256")?.str()?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return refuse("malformed output digest");
            }
            escape |= !under;
            contained &= under;
        }
        if escape != v.get("escape")?.bool()? || contained != v.get("contained")?.bool()? {
            return refuse("contradictory native filesystem witness");
        }
        let exception = v.get("exception")?.str()?;
        if exception.len() > 100
            || !exception
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return refuse("invalid native exception field");
        }
        if policy == "default" && escape {
            default_escapes
                .entry(module.to_owned())
                .or_default()
                .push(member.to_owned());
        }
        if policy != "default" && !escape {
            safe_controls += 1;
        }
        if policy == "default" && contained {
            safe_controls += 1;
        }
        obs.push((key.0, key.1, key.2, v.canonical()));
    }
    if safe_controls == 0 {
        return refuse("no normal or hardened negative controls");
    }
    obs.sort();
    let mut expected_findings = Vec::new();
    for (module, members) in default_escapes {
        let item = obj(&[
            ("module", Value::string(&module)),
            ("entrypoint", Value::string(source_names[&module].clone())),
            (
                "property",
                Value::string("archive-extraction-destination-containment"),
            ),
            (
                "counterexamples",
                Value::array(members.into_iter().map(Value::string)),
            ),
        ]);
        expected_findings.push((module, item.canonical()));
    }
    let mut provided = Vec::new();
    for v in findings {
        provided.push((v.get("module")?.str()?.to_string(), v.canonical()));
    }
    provided.sort();
    if provided != expected_findings {
        return refuse(
            "native discovery findings do not match independently recomputed observations",
        );
    }
    Ok((src, obs, expected_findings))
}

pub fn execute(root: &Path, db: &Db, audit: bool) -> Result<Value> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let commit = check_tracked(&root)?;
    let data = native_scan(&root)?;
    let (sources, cases, findings) = check_result(&root, &data)?;
    check_tracked(&root)?;
    let digest = data.sha256();
    let run_id = crate::digest::sha256(format!("discovery-v1:{commit}:{digest}").as_bytes());
    let version = data.get("runtime")?.get("version")?.str()?;
    let implementation = data.get("runtime")?.get("implementation")?.str()?;
    let header = vec![
        Some(run_id.clone()),
        Some(commit.clone()),
        Some(digest.clone()),
        Some(implementation.to_owned()),
        Some(version.to_owned()),
        Some(sources.len().to_string()),
        Some(cases.len().to_string()),
        Some(findings.len().to_string()),
        Some("NATIVE_BOUNDARY_OBSERVATIONS".into()),
    ];
    if audit {
        if db.query("SELECT run_id,checkout_commit,observation_sha256,implementation,runtime_version,source_count,case_count,finding_count,state FROM security_discovery_run", &[])? != vec![header] {
            return refuse("independent discovery run mismatch");
        }
        let source_rows = sources
            .iter()
            .map(|(m, j)| vec![Some(run_id.clone()), Some(m.clone()), Some(j.clone())])
            .collect::<Vec<_>>();
        let observation_rows = cases
            .iter()
            .map(|(m, p, k, j)| {
                vec![
                    Some(run_id.clone()),
                    Some(m.clone()),
                    Some(p.clone()),
                    Some(k.clone()),
                    Some(j.clone()),
                ]
            })
            .collect::<Vec<_>>();
        let finding_rows = findings
            .iter()
            .map(|(m, j)| vec![Some(run_id.clone()), Some(m.clone()), Some(j.clone())])
            .collect::<Vec<_>>();
        for (sql, expected) in [
            (
                "SELECT run_id,module,source_json FROM security_discovery_source ORDER BY module",
                source_rows,
            ),
            (
                "SELECT run_id,module,policy,member,observation_json FROM security_discovery_case ORDER BY module,policy,member",
                observation_rows,
            ),
            (
                "SELECT run_id,module,evidence_json FROM security_discovery_finding ORDER BY module",
                finding_rows,
            ),
        ] {
            if db.query(sql, &[])? != expected {
                return refuse("independent observation table mismatch");
            }
        }
        db.foreign_keys_valid()?;
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query("SELECT run_id FROM security_discovery_run", &[])?
                .is_empty()
            {
                return refuse("security discovery requires a fresh database");
            }
            db.execute(
                "INSERT INTO security_discovery_run VALUES(?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&run_id),
                    Param::Text(&commit),
                    Param::Text(&digest),
                    Param::Text(implementation),
                    Param::Text(version),
                    Param::Integer(sources.len() as i64),
                    Param::Integer(cases.len() as i64),
                    Param::Integer(findings.len() as i64),
                    Param::Text("NATIVE_BOUNDARY_OBSERVATIONS"),
                ],
            )?;
            for (module, encoded) in &sources {
                db.execute(
                    "INSERT INTO security_discovery_source VALUES(?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(module),
                        Param::Text(encoded),
                    ],
                )?;
            }
            for (module, policy, member, encoded) in &cases {
                db.execute(
                    "INSERT INTO security_discovery_case VALUES(?,?,?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(module),
                        Param::Text(policy),
                        Param::Text(member),
                        Param::Text(encoded),
                    ],
                )?;
            }
            for (module, encoded) in &findings {
                db.execute(
                    "INSERT INTO security_discovery_finding VALUES(?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(module),
                        Param::Text(encoded),
                    ],
                )?;
            }
            Ok(())
        })?;
        db.foreign_keys_valid()?;
    }
    Ok(obj(&[
        ("state", Value::string("NATIVE_BOUNDARY_OBSERVATIONS")),
        ("audited", Value::Bool(audit)),
        ("source_commit", Value::string(commit)),
        ("observation_sha256", Value::string(digest)),
        ("run_id", Value::string(run_id)),
        ("sources", Value::number(sources.len())),
        ("cases", Value::number(cases.len())),
        ("findings", Value::number(findings.len())),
        (
            "finding_evidence",
            Value::array(
                findings
                    .iter()
                    .map(|(_, j)| json::parse(j.as_bytes()).expect("validated JSON")),
            ),
        ),
        (
            "claim_boundary",
            Value::string(
                "native source-bound counterexamples; full downstream exploitability and CVE mapping are separate",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_locations_are_fail_closed() {
        assert!(verified_write_location("destination/normal.txt", true).is_ok());
        assert!(verified_write_location("marker.txt", false).is_ok());
        for (path, flag) in [
            ("marker.txt", true),
            ("destination/normal.txt", false),
            ("destination/../marker.txt", true),
            ("/tmp/marker.txt", false),
            ("destination//marker.txt", true),
            ("destination/./normal.txt", true),
        ] {
            assert!(verified_write_location(path, flag).is_err(), "{path}");
        }
    }
    #[test]
    fn observation_rows_are_canonical() {
        let x=json::parse(br#"{"member":"../marker.txt","escape":true,"writes":[{"relative_to_sandbox":"marker.txt","under_destination":false}]}"#).unwrap();
        assert!(x.canonical().contains("\"under_destination\":false"));
    }
}
