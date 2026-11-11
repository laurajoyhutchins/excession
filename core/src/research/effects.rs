//! Bounded semantic effect observations across four authenticated Git revisions.
//! No extrapolation beyond the named cases. Native Python/Node probes execute in
//! trusted local checkouts and are rerun on independent audit.
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type EvidenceRow = Vec<Option<String>>;
type EvidenceRows = (Vec<EvidenceRow>, Vec<EvidenceRow>);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observed_effect_run(
 id TEXT PRIMARY KEY, base TEXT NOT NULL, left_revision TEXT NOT NULL,
 right_revision TEXT NOT NULL, merged_revision TEXT NOT NULL,
 source_path TEXT NOT NULL, command_sha256 TEXT NOT NULL,
 expected_sha256 TEXT NOT NULL, case_count INTEGER NOT NULL CHECK(case_count>0)
);
CREATE TABLE IF NOT EXISTS observed_effect_case(
 run_id TEXT NOT NULL REFERENCES observed_effect_run(id),
 case_name TEXT NOT NULL, base_value TEXT NOT NULL, left_value TEXT NOT NULL,
 right_value TEXT NOT NULL, merged_value TEXT NOT NULL,
 left_changed INTEGER NOT NULL CHECK(left_changed IN (0,1)),
 right_changed INTEGER NOT NULL CHECK(right_changed IN (0,1)),
 interaction INTEGER NOT NULL CHECK(interaction IN (0,1)),
 PRIMARY KEY(run_id,case_name)
);
"#;

#[derive(Clone)]
pub struct Spec {
    roots: [PathBuf; 4],
    file: String,
    probe: Vec<String>,
    expected: BTreeMap<String, Value>,
}
impl Spec {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        if !v.get("trusted_local")?.bool()? {
            return refuse("semantic probes are permitted only for trusted local checkouts");
        }
        let root = |name: &str| -> Result<PathBuf> {
            PathBuf::from(v.get(name)?.str()?)
                .canonicalize()
                .map_err(|e| format!("{name} path: {e}"))
        };
        let file = v.get("file")?.str()?.to_owned();
        git::validate_path(&file)?;
        let argv = v.get("probe")?.array_ref()?;
        if argv.is_empty() || argv.len() > 12 {
            return refuse("invalid probe argv length");
        }
        let probe = argv
            .iter()
            .map(|x| x.str().map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        if !matches!(probe[0].as_str(), "python" | "python3" | "node" | "go")
            || probe
                .iter()
                .any(|s| s.is_empty() || s.contains('\0') || s.len() > 65536)
        {
            return refuse("unsupported native probe command; no shell execution");
        }
        if probe[0] == "go" && !matches!(probe.get(1).map(String::as_str), Some("test" | "run")) {
            return refuse("Go semantic probes restricted to go test/run");
        }
        let expected = v.get("expected_interactions")?.entries()?.clone();
        if expected.len() > 128 || expected.keys().any(|k| k.is_empty() || k.len() > 128) {
            return refuse("invalid expected interaction labels");
        }
        let roots = [
            root("base")?,
            root("left")?,
            root("right")?,
            root("merged")?,
        ];
        if roots.iter().collect::<BTreeSet<_>>().len() != 4 {
            return refuse("four independent worktrees required");
        }
        Ok(Self {
            roots,
            file,
            probe,
            expected,
        })
    }
}

fn observe(root: &Path, argv: &[String]) -> Result<BTreeMap<String, Value>> {
    // GNU timeout bounds a trusted local native test process. No process is
    // permitted to advertise partial success on a timeout or signal.
    let out = Command::new("timeout")
        .arg("20s")
        .arg(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .env("PYTHONPATH", root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .map_err(|e| format!("native semantic probe launch: {e}"))?;
    if !out.status.success() {
        return refuse(format!(
            "native semantic probe failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if out.stdout.len() > 65536 {
        return refuse("semantic observation exceeds 64 KiB");
    }
    let value = json::parse(&out.stdout)?;
    let map = value.entries()?.clone();
    if map.is_empty() || map.len() > 128 || map.keys().any(|k| k.is_empty() || k.len() > 128) {
        return refuse("semantic observation requires 1-128 named cases");
    }
    git::clean(root)?;
    Ok(map)
}

struct Prepared {
    id: String,
    commits: [String; 4],
    command_sha: String,
    expected_sha: String,
    rows: Vec<Vec<String>>,
    report: Value,
}
fn prepare(s: &Spec) -> Result<Prepared> {
    for root in &s.roots {
        git::clean(root)?;
    }
    let commits = [
        git::rev(&s.roots[0], "HEAD")?,
        git::rev(&s.roots[1], "HEAD")?,
        git::rev(&s.roots[2], "HEAD")?,
        git::rev(&s.roots[3], "HEAD")?,
    ];
    if commits[1] == commits[2]
        || git::rev(&s.roots[1], "HEAD^")? != commits[0]
        || git::rev(&s.roots[2], "HEAD^")? != commits[0]
        || git::rev(&s.roots[3], "HEAD^1")? != commits[1]
        || git::rev(&s.roots[3], "HEAD^2")? != commits[2]
    {
        return refuse(
            "semantic evidence requires distinct sibling branches and exact two-parent merge",
        );
    }
    let mut verified_mode = None;
    for (root, commit) in s.roots.iter().zip(commits.iter()) {
        let oid = git::blob(root, commit, &s.file)?;
        let entry = git::entries(root, commit)?
            .into_iter()
            .find(|e| e.path == s.file)
            .ok_or("missing evidence source")?;
        if !matches!(entry.mode.as_str(), "100644" | "100755")
            || entry.kind != "blob"
            || entry.oid != oid
        {
            return refuse("unexpected source mode or Git blob");
        }
        if verified_mode
            .as_ref()
            .is_some_and(|expected| expected != &entry.mode)
        {
            return refuse("source mode differs between semantic revisions");
        }
        verified_mode = Some(entry.mode.clone());
    }
    for i in [1, 2, 3] {
        if git::changed(&s.roots[i], &commits[0], &commits[i])? != vec![s.file.clone()] {
            return refuse("semantic evidence has undeclared changed source files");
        }
    }
    let results = s
        .roots
        .iter()
        .map(|r| observe(r, &s.probe))
        .collect::<Result<Vec<_>>>()?;
    let names: Vec<String> = results[0].keys().cloned().collect();
    if results
        .iter()
        .any(|m| m.keys().cloned().collect::<Vec<_>>() != names)
    {
        return refuse("native semantic probes disagree about input case coverage");
    }
    let mut rows = Vec::new();
    let mut interacting = Vec::new();
    let mut left_effects = Vec::new();
    let mut right_effects = Vec::new();
    for name in names {
        let [b, l, r, m] = std::array::from_fn(|i| results[i][&name].canonical());
        let lc = l != b;
        let rc = r != b;
        if lc {
            left_effects.push(Value::string(&name));
        }
        if rc {
            right_effects.push(Value::string(&name));
        }
        let independent = if lc && rc {
            None
        } else if lc {
            Some(l.as_str())
        } else if rc {
            Some(r.as_str())
        } else {
            Some(b.as_str())
        };
        let interaction = independent != Some(m.as_str());
        if interaction {
            interacting.push(Value::string(&name));
            match s.expected.get(&name) {
                Some(expected) if expected.canonical() == m => {}
                _ => {
                    return refuse(format!(
                        "undeclared or incorrect joint semantic effect on {name}"
                    ));
                }
            }
        } else if s.expected.contains_key(&name) {
            return refuse(format!("unused expected interaction: {name}"));
        }
        rows.push(vec![
            name,
            b,
            l,
            r,
            m,
            i64::from(lc).to_string(),
            i64::from(rc).to_string(),
            i64::from(interaction).to_string(),
        ]);
    }
    for name in s.expected.keys() {
        if !rows.iter().any(|row| &row[0] == name) {
            return refuse("undeclared probe case in expected interactions");
        }
    }
    let cmd = json::Value::array(s.probe.iter().map(Value::string));
    let expected = json::Value::object(s.expected.clone());
    let command_sha = cmd.sha256();
    let expected_sha = expected.sha256();
    let identity = obj(&[
        ("commits", Value::array(commits.iter().map(Value::string))),
        ("file", Value::string(&s.file)),
        ("probe_sha256", Value::string(&command_sha)),
        ("expected_sha256", Value::string(&expected_sha)),
    ]);
    let id = identity.sha256();
    let report = obj(&[
        ("status", Value::string("FINITE_OBSERVED_EFFECTS_ONLY")),
        ("run_id", Value::string(&id)),
        ("case_count", Value::number(rows.len())),
        ("left_changed", Value::array(left_effects)),
        ("right_changed", Value::array(right_effects)),
        ("interaction_cases", Value::array(interacting)),
        ("source_commit", Value::string(&commits[3])),
    ]);
    Ok(Prepared {
        id,
        commits,
        command_sha,
        expected_sha,
        rows,
        report,
    })
}
fn stored(db: &Db, id: &str) -> Result<EvidenceRows> {
    Ok((
        db.query(
            "SELECT * FROM observed_effect_run WHERE id=?",
            &[Param::Text(id)],
        )?,
        db.query(
            "SELECT * FROM observed_effect_case WHERE run_id=? ORDER BY case_name",
            &[Param::Text(id)],
        )?,
    ))
}
fn expected_rows(p: &Prepared, s: &Spec) -> EvidenceRows {
    let run = vec![vec![
        p.id.clone(),
        p.commits[0].clone(),
        p.commits[1].clone(),
        p.commits[2].clone(),
        p.commits[3].clone(),
        s.file.clone(),
        p.command_sha.clone(),
        p.expected_sha.clone(),
        p.rows.len().to_string(),
    ]];
    let cases: Vec<Vec<String>> = p
        .rows
        .iter()
        .map(|row| {
            std::iter::once(p.id.clone())
                .chain(row.iter().cloned())
                .collect()
        })
        .collect();
    (
        run.into_iter()
            .map(|r| r.into_iter().map(Some).collect())
            .collect(),
        cases
            .into_iter()
            .map(|r: Vec<String>| r.into_iter().map(Some).collect())
            .collect(),
    )
}
pub fn execute(db: &Db, s: &Spec, audit: bool) -> Result<Value> {
    let p = prepare(s)?;
    db.exec(SCHEMA)?;
    if audit {
        let actual = stored(db, &p.id)?;
        if actual != expected_rows(&p, s) {
            return refuse("forged or stale finite semantic observation evidence");
        }
        // An attacker may append forged rows using unrelated run IDs. For the
        // portable bounded experiment require a database dedicated to this run.
        if db.scalar("SELECT COUNT(*) FROM observed_effect_run", &[])? != "1"
            || db
                .scalar("SELECT COUNT(*) FROM observed_effect_case", &[])?
                .parse::<usize>()
                .ok()
                != Some(p.rows.len())
        {
            return refuse("unexpected extra semantic observation rows");
        }
    } else {
        db.exec("BEGIN IMMEDIATE")?;
        db.execute(
            "INSERT INTO observed_effect_run VALUES(?,?,?,?,?,?,?,?,?)",
            &[
                Param::Text(&p.id),
                Param::Text(&p.commits[0]),
                Param::Text(&p.commits[1]),
                Param::Text(&p.commits[2]),
                Param::Text(&p.commits[3]),
                Param::Text(&s.file),
                Param::Text(&p.command_sha),
                Param::Text(&p.expected_sha),
                Param::Integer(p.rows.len() as i64),
            ],
        )?;
        for row in &p.rows {
            db.execute(
                "INSERT INTO observed_effect_case VALUES(?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&p.id),
                    Param::Text(&row[0]),
                    Param::Text(&row[1]),
                    Param::Text(&row[2]),
                    Param::Text(&row[3]),
                    Param::Text(&row[4]),
                    Param::Integer(row[5].parse().map_err(|_| "invalid left changed")?),
                    Param::Integer(row[6].parse().map_err(|_| "invalid right changed")?),
                    Param::Integer(row[7].parse().map_err(|_| "invalid interaction flag")?),
                ],
            )?;
        }
        db.exec("COMMIT")?;
    }
    Ok(p.report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_rejects_malformed() {
        assert!(json::parse(b"{\"x\":1,\"x\":2}").is_err());
        assert!(json::parse(b"{\"x\":NaN}").is_err());
    }
    #[test]
    fn merge_truth_table() {
        // Distinct changed inputs may be carried on observed cases, whereas
        // overlapping changes have no mechanically determined combined output.
        let b = "0";
        let l = "1";
        let r = "0";
        assert_eq!(
            if l != b && r != b {
                None
            } else if l != b {
                Some(l)
            } else if r != b {
                Some(r)
            } else {
                Some(b)
            },
            Some("1")
        );
        let r = "2";
        assert_eq!(
            if l != b && r != b {
                None
            } else if l != b {
                Some(l)
            } else if r != b {
                Some(r)
            } else {
                Some(b)
            },
            None
        );
    }
}
