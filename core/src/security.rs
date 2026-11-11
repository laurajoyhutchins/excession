//! CVE research pilot: a pinned, bounded native-Go storage-prefix invariant.
//! A counterexample to this isolated invariant is not evidence of CVE reachability.
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS security_pilot_run (
 id TEXT PRIMARY KEY, local_commit TEXT NOT NULL, old_upstream_blob TEXT NOT NULL,
 fixed_upstream_blob TEXT NOT NULL, source_blobs_sha256 TEXT NOT NULL,
 observation_sha256 TEXT NOT NULL, case_count INTEGER NOT NULL CHECK(case_count > 0),
 escape_count INTEGER NOT NULL CHECK(escape_count > 0),
 state TEXT NOT NULL CHECK(state = 'BOUNDED_SOURCE_COUNTEREXAMPLE')
);
CREATE TABLE IF NOT EXISTS security_pilot_case (
 run_id TEXT NOT NULL REFERENCES security_pilot_run(id),
 candidate TEXT NOT NULL, old_key TEXT NOT NULL, fixed_key TEXT NOT NULL,
 old_escapes INTEGER NOT NULL CHECK(old_escapes IN (0, 1)),
 fixed_escapes INTEGER NOT NULL CHECK(fixed_escapes IN (0, 1)),
 fixed_rejected INTEGER NOT NULL CHECK(fixed_rejected IN (0, 1)),
 PRIMARY KEY (run_id, candidate)
);
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Case {
    candidate: String,
    old_key: String,
    fixed_key: String,
    old_escapes: bool,
    fixed_escapes: bool,
    fixed_rejected: bool,
}
impl Case {
    fn read(v: &Value, resource_root: &str) -> Result<Self> {
        let c = Self {
            candidate: v.get("candidate")?.str()?.to_owned(),
            old_key: v.get("old_key")?.str()?.to_owned(),
            fixed_key: v.get("fixed_key")?.str()?.to_owned(),
            old_escapes: v.get("old_escapes")?.bool()?,
            fixed_escapes: v.get("fixed_escapes")?.bool()?,
            fixed_rejected: v.get("fixed_rejected")?.bool()?,
        };
        if c.candidate.len() > 512 || c.old_key.len() > 2048 || c.fixed_key.len() > 2048 {
            return refuse("oversized observation case");
        }
        if c.old_escapes == c.old_key.starts_with(resource_root)
            || (c.fixed_rejected && !c.fixed_key.is_empty())
            || (!c.fixed_rejected && c.fixed_escapes == c.fixed_key.starts_with(resource_root))
            || (c.fixed_rejected && c.fixed_escapes)
            || (c.old_escapes && !c.fixed_rejected)
        {
            return refuse(format!(
                "contradictory security observation: {:?}",
                c.candidate
            ));
        }
        Ok(c)
    }
    fn row(&self, run_id: &str) -> Vec<Option<String>> {
        let b = |x: bool| Some((x as u8).to_string());
        vec![
            Some(run_id.to_owned()),
            Some(self.candidate.clone()),
            Some(self.old_key.clone()),
            Some(self.fixed_key.clone()),
            b(self.old_escapes),
            b(self.fixed_escapes),
            b(self.fixed_rejected),
        ]
    }
}

fn run_go(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    // Trusted local fixture ONLY. Timeout is not an isolation boundary.
    let result = Command::new("timeout")
        .args(["60s", "go"])
        .args(args)
        .current_dir(root)
        .env("GOPROXY", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GOSUMDB", "off")
        .output()
        .map_err(|e| format!("Go launch: {e}"))?;
    if !result.status.success() {
        return refuse(format!(
            "Go {:?} failed: {}",
            args,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    if result.stdout.len() > 1024 * 1024 {
        return refuse("Go probe output too large");
    }
    Ok(result.stdout)
}

fn source_fingerprints(root: &Path) -> Result<(String, String)> {
    git::clean(root)?;
    let commit = git::rev(root, "HEAD")?;
    let mut identities = Vec::new();
    for path in [
        "go.mod",
        "keys.go",
        "keys_test.go",
        "cmd/probe/main.go",
        "manifest.json",
    ] {
        let oid = git::blob(root, "HEAD", path)?;
        let actual = git::text(root, &["hash-object", path])?;
        if oid != actual {
            return refuse(format!("working source differs from Git: {path}"));
        }
        identities.push(format!("{path}:{oid}"));
    }
    Ok((
        commit,
        crate::digest::sha256(identities.join("\n").as_bytes()),
    ))
}

fn manifest(root: &Path) -> Result<(String, String)> {
    let value = json::parse(&fs::read(root.join("manifest.json")).map_err(|e| e.to_string())?)?;
    if !value.get("trusted_local")?.bool()?
        || value.get("type")?.str()? != "security-prefix-pilot-v1"
        || value.get("source_kind")?.str()? != "bounded-transcription-not-upstream-blob"
        || value.get("upstream_repository")?.str()? != "kubernetes/kubernetes"
        || value.get("upstream_path")?.str()?
            != "staging/src/k8s.io/apiserver/pkg/storage/etcd3/store.go"
        || value.get("vulnerable_ref")?.str()? != "v1.25.3"
        || value.get("patched_ref")?.str()? != "v1.25.4"
    {
        return refuse("unsupported upstream identity or local trust claim");
    }
    Ok((
        git::oid(value.get("vulnerable_blob")?.str()?)?,
        git::oid(value.get("patched_blob")?.str()?)?,
    ))
}

pub fn execute(root: &Path, db: &Db, audit: bool) -> Result<Value> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let (commit, source_digest) = source_fingerprints(&root)?;
    let (old_blob, fixed_blob) = manifest(&root)?;
    run_go(&root, &["test", "./..."])?;
    let raw = run_go(&root, &["run", "./cmd/probe"])?;
    let result = json::parse(&raw)?;
    if result.get("type")?.str()? != "prefix-isolation-v1" {
        return refuse("wrong probe kind");
    }
    let prefix = result.get("prefix")?.str()?;
    if prefix != "/registry/authorized.example.io" {
        return refuse("unexpected resource prefix");
    }
    let resource_root = format!("{prefix}/");
    let arr = result.get("cases")?.array_ref()?;
    if !(40..=256).contains(&arr.len()) {
        return refuse("outside finite probe coverage");
    }
    let mut cases = Vec::new();
    let mut seen = BTreeSet::new();
    let mut escape = 0usize;
    let mut control = 0usize;
    for v in arr {
        let c = Case::read(v, &resource_root)?;
        if !seen.insert(c.candidate.clone()) {
            return refuse("duplicate security candidate");
        }
        if c.old_escapes {
            escape += 1;
        }
        if !c.old_escapes && !c.fixed_rejected {
            control += 1;
        }
        cases.push(c);
    }
    if escape == 0 || control == 0 {
        return refuse("missing negative/positive controls");
    }
    if !cases
        .iter()
        .any(|c| c.candidate.starts_with("../") && c.old_escapes && c.fixed_rejected)
    {
        return refuse("missing traversal-specific counterexample");
    }
    git::clean(&root)?;
    // Canonical case output is deterministically ordered by probe generation.
    let observation_sha = result.sha256();
    let run_id =
        crate::digest::sha256(format!("{commit}:{source_digest}:{observation_sha}").as_bytes());
    let details = vec![
        Some(run_id.clone()),
        Some(commit.clone()),
        Some(old_blob.clone()),
        Some(fixed_blob.clone()),
        Some(source_digest.clone()),
        Some(observation_sha.clone()),
        Some(cases.len().to_string()),
        Some(escape.to_string()),
        Some("BOUNDED_SOURCE_COUNTEREXAMPLE".into()),
    ];
    if audit {
        let prev=db.query("SELECT id,local_commit,old_upstream_blob,fixed_upstream_blob,source_blobs_sha256,observation_sha256,case_count,escape_count,state FROM security_pilot_run",&[])?;
        if prev != vec![details] {
            return refuse("security run evidence differs on independent audit");
        }
        let prior=db.query("SELECT run_id,candidate,old_key,fixed_key,old_escapes,fixed_escapes,fixed_rejected FROM security_pilot_case ORDER BY candidate",&[])?;
        let current = cases.iter().map(|c| c.row(&run_id)).collect::<Vec<_>>();
        if prior != current {
            return refuse("security case evidence differs on independent audit");
        }
        db.foreign_keys_valid()?;
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query("SELECT id FROM security_pilot_run", &[])?
                .is_empty()
            {
                return refuse("security DB must be fresh");
            }
            db.execute(
                "INSERT INTO security_pilot_run VALUES(?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&run_id),
                    Param::Text(&commit),
                    Param::Text(&old_blob),
                    Param::Text(&fixed_blob),
                    Param::Text(&source_digest),
                    Param::Text(&observation_sha),
                    Param::Integer(cases.len() as i64),
                    Param::Integer(escape as i64),
                    Param::Text("BOUNDED_SOURCE_COUNTEREXAMPLE"),
                ],
            )?;
            for c in &cases {
                let flag = |x: bool| if x { 1 } else { 0 };
                db.execute(
                    "INSERT INTO security_pilot_case VALUES(?,?,?,?,?,?,?)",
                    &[
                        Param::Text(&run_id),
                        Param::Text(&c.candidate),
                        Param::Text(&c.old_key),
                        Param::Text(&c.fixed_key),
                        Param::Integer(flag(c.old_escapes)),
                        Param::Integer(flag(c.fixed_escapes)),
                        Param::Integer(flag(c.fixed_rejected)),
                    ],
                )?;
            }
            Ok(())
        })?;
        db.foreign_keys_valid()?;
    }
    Ok(obj(&[
        ("state", Value::string("BOUNDED_SOURCE_COUNTEREXAMPLE")),
        ("audit", Value::Bool(audit)),
        ("cases", Value::number(cases.len())),
        ("old_prefix_escapes", Value::number(escape)),
        ("safe_controls", Value::number(control)),
        ("run_id", Value::string(run_id)),
        ("observation_sha256", Value::string(observation_sha)),
        ("local_fixture_commit", Value::string(commit)),
        (
            "claim_boundary",
            Value::string(
                "minimal upstream-logic transcription; no full apiserver/RBAC reachability",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::Case;
    use crate::json;
    #[test]
    fn reject_bogus_fixed_escape_claim() {
        let bad=json::parse(br#"{"candidate":"../x","old_key":"/registry/other/","fixed_key":"/registry/other/","old_escapes":true,"fixed_escapes":true,"fixed_rejected":false}"#).unwrap();
        assert!(Case::read(&bad, "/registry/authorized.example.io/").is_err());
    }
    #[test]
    fn accept_exact_traversal_case() {
        let good=json::parse(br#"{"candidate":"../x","old_key":"/registry/other/","fixed_key":"","old_escapes":true,"fixed_escapes":false,"fixed_rejected":true}"#).unwrap();
        assert!(Case::read(&good, "/registry/authorized.example.io/").is_ok());
    }
}
