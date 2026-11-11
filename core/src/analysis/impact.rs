//! Conservative source-identity eligibility across original/projected histories.
//! Never promotes original evidence to projected code, regardless of passing smoke tests.
use crate::sqlite::{Db, Param};
use crate::{Result, git, index, json, refuse};
use json::{Value, obj};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
const POLICY: &str = "v6-pinned-baseline-whole-h11-package-v1";
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS source_assessment (
 id TEXT NOT NULL, viewpoint TEXT NOT NULL,
 claim_id TEXT NOT NULL REFERENCES bounded_claim(id),
 repo_id INTEGER NOT NULL, commit_oid TEXT NOT NULL,
 policy TEXT NOT NULL CHECK(policy='v6-pinned-baseline-whole-h11-package-v1'),
 decision TEXT NOT NULL CHECK(decision IN ('SOURCE_MATCH_ONLY','REVERIFY_REQUIRED','UNVERIFIED_PROJECTION')),
 matching_inputs INTEGER NOT NULL, total_inputs INTEGER NOT NULL,
 PRIMARY KEY(id,viewpoint,claim_id),
 FOREIGN KEY(repo_id,commit_oid) REFERENCES revision(repo_id,commit_oid)
);
CREATE TABLE IF NOT EXISTS source_assessment_input (
 id TEXT NOT NULL, viewpoint TEXT NOT NULL, claim_id TEXT NOT NULL,
 expected_path TEXT NOT NULL, actual_path TEXT NOT NULL,
 pinned_oid TEXT NOT NULL, observed_oid TEXT,
 comparison TEXT NOT NULL CHECK(comparison IN ('IDENTICAL','DIFFERENT','MISSING')),
 PRIMARY KEY(id,viewpoint,claim_id,expected_path),
 FOREIGN KEY(id,viewpoint,claim_id) REFERENCES source_assessment(id,viewpoint,claim_id)
);
"#;
#[derive(Clone)]
pub struct ImpactPlan {
    pub source: PathBuf,
    pub source_base: String,
    pub source_change: String,
    pub projection: PathBuf,
    pub projection_base: String,
    pub replay: PathBuf,
    pub replay_commit: String,
    pub mapping: BTreeMap<String, String>,
    pub id: String,
}
impl ImpactPlan {
    pub fn read(path: &Path) -> Result<Self> {
        let v = json::parse(&fs::read(path).map_err(|e| e.to_string())?)?;
        let string = |k| -> Result<String> { Ok(v.get(k)?.str()?.to_owned()) };
        let real = |k| -> Result<PathBuf> {
            PathBuf::from(string(k)?)
                .canonicalize()
                .map_err(|e| e.to_string())
        };
        let mapping = v
            .get("mapping")?
            .entries()?
            .iter()
            .map(|(k, val)| Ok((k.clone(), val.str()?.to_owned())))
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Self {
            source: real("source")?,
            source_base: git::oid(&string("source_base")?)?,
            source_change: git::oid(&string("source_change")?)?,
            projection: real("projection")?,
            projection_base: git::oid(&string("projection_base")?)?,
            replay: real("replay")?,
            replay_commit: git::oid(&string("replay_commit")?)?,
            mapping,
            id: string("id")?,
        })
    }
}
fn read_blob(db: &Db, rid: i64, commit: &str, path: &str) -> Result<Option<String>> {
    let rows = db.query(
        "SELECT oid FROM tree_entry WHERE repo_id=? AND commit_oid=? AND path=? AND kind='blob'",
        &[Param::Integer(rid), Param::Text(commit), Param::Text(path)],
    )?;
    if rows.len() > 1 {
        return refuse("ambiguous source blob");
    }
    Ok(rows.first().and_then(|x| x[0].clone()))
}
pub fn assess(db: &Db, plan: &ImpactPlan) -> Result<Value> {
    if plan.id.is_empty() {
        return refuse("missing assessment identity");
    }
    for (k, v) in &plan.mapping {
        git::validate_path(k)?;
        git::validate_path(v)?;
    }
    git::clean(&plan.source)?;
    git::clean(&plan.projection)?;
    git::clean(&plan.replay)?;
    if git::rev(&plan.source, "HEAD")? != plan.source_change
        || git::rev(&plan.projection, "HEAD")? != plan.projection_base
        || git::rev(&plan.replay, "HEAD")? != plan.replay_commit
    {
        return refuse("assessment source not at pinned HEAD");
    }
    let exps = db.query(
        "SELECT id,baseline_repo,baseline_commit FROM bounded_experiment ORDER BY id",
        &[],
    )?;
    if exps.len() != 1 {
        return refuse("expected a verified bounded preservation experiment");
    }
    let experiment = exps[0][0].clone().ok_or("missing experiment id")?;
    let baseline_repo = exps[0][1]
        .as_ref()
        .ok_or("null baseline repo")?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    let baseline_commit = exps[0][2].clone().ok_or("null baseline commit")?;
    let inputs=db.query("SELECT path,oid FROM tree_entry WHERE repo_id=? AND commit_oid=? AND kind='blob' AND path LIKE 'h11/%' ORDER BY path",
  &[Param::Integer(baseline_repo),Param::Text(&baseline_commit)])?;
    let input_items = inputs
        .iter()
        .map(|r| {
            Ok((
                r[0].clone().ok_or("null path")?,
                r[1].clone().ok_or("null oid")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if !input_items.iter().any(|(p, _)| p == "h11/__init__.py")
        || !input_items.iter().any(|(p, _)| p == "h11/_readers.py")
        || !input_items.iter().any(|(p, _)| p == "h11/_writers.py")
    {
        return refuse("incomplete whole-package source baseline");
    }
    let declared = db
        .query("SELECT DISTINCT path FROM contract_closure", &[])?
        .into_iter()
        .filter_map(|r| r[0].clone())
        .collect::<BTreeSet<_>>();
    if !declared.is_subset(&input_items.iter().map(|(p, _)| p.clone()).collect()) {
        return refuse("declared closure exceeds pinned tracked package");
    }
    let claims = db
        .query("SELECT id FROM bounded_claim ORDER BY id", &[])?
        .into_iter()
        .map(|r| r[0].clone().ok_or_else(|| "null claim".to_owned()))
        .collect::<Result<Vec<_>>>()?;
    if claims.len() != 3 {
        return refuse("invalid bounded claim inventory");
    }
    let views = [
        (
            "original_parent",
            &plan.source,
            &plan.source_base,
            false,
            false,
        ),
        (
            "original_change",
            &plan.source,
            &plan.source_change,
            false,
            false,
        ),
        (
            "projection_parent",
            &plan.projection,
            &plan.projection_base,
            true,
            false,
        ),
        (
            "projection_replay",
            &plan.replay,
            &plan.replay_commit,
            true,
            true,
        ),
    ];
    db.exec(SCHEMA)?;
    let mut reported = Vec::new();
    let mut computed = Vec::new();
    for (view, repo, commit, projected, is_replay) in views {
        let name = if projected {
            format!(
                "assessment-projection-{}",
                if is_replay { "replay" } else { "parent" }
            )
        } else {
            "assessment-original".to_owned()
        };
        let rid = index::index_revision_with_roots(
            db,
            repo,
            &name,
            commit,
            "contract impact snapshot",
            &["h11"],
        )?;
        let tree = index::reconstruct_tree(db, rid, commit)?;
        if tree != git::rev(repo, &format!("{commit}^{{tree}}"))? {
            return refuse("native Git tree differs from SQLite");
        }
        let mut observations = Vec::new();
        let mut matched = 0;
        for (original, expected) in &input_items {
            let target = if projected {
                plan.mapping.get(original).unwrap_or(original)
            } else {
                original
            };
            let observed = read_blob(db, rid, commit, target)?;
            if let Some(actual) = &observed
                && git::blob(repo, commit, target)? != *actual
            {
                return refuse("SQLite blob differs from Git");
            }
            let cmp = match &observed {
                None => "MISSING",
                Some(x) if x == expected => "IDENTICAL",
                Some(_) => "DIFFERENT",
            };
            if cmp == "IDENTICAL" {
                matched += 1;
            }
            observations.push((
                original.clone(),
                target.clone(),
                expected.clone(),
                observed,
                cmp,
            ));
        }
        let decision = if projected {
            "UNVERIFIED_PROJECTION"
        } else if matched == input_items.len() {
            "SOURCE_MATCH_ONLY"
        } else {
            "REVERIFY_REQUIRED"
        };
        computed.push((view, rid, commit, decision, matched, observations));
        reported.push(obj(&[
            ("view", Value::string(view)),
            ("decision", Value::string(decision)),
            ("matching_inputs", Value::number(matched)),
            ("total_inputs", Value::number(input_items.len())),
        ]));
    }
    db.transaction(|db| {
        for (view, rid, commit, decision, matched, observations) in &computed {
            for claim in &claims {
                db.execute(
                    "INSERT INTO source_assessment VALUES(?,?,?,?,?,?,?,?,?)",
                    &[
                        Param::Text(&plan.id),
                        Param::Text(view),
                        Param::Text(claim),
                        Param::Integer(*rid),
                        Param::Text(commit),
                        Param::Text(POLICY),
                        Param::Text(decision),
                        Param::Integer(*matched as i64),
                        Param::Integer(input_items.len() as i64),
                    ],
                )?;
                for (original, target, pinned, observed, cmp) in observations {
                    db.execute(
                        "INSERT INTO source_assessment_input VALUES(?,?,?,?,?,?,?,?)",
                        &[
                            Param::Text(&plan.id),
                            Param::Text(view),
                            Param::Text(claim),
                            Param::Text(original),
                            Param::Text(target),
                            Param::Text(pinned),
                            observed.as_deref().map_or(Param::Null, Param::Text),
                            Param::Text(cmp),
                        ],
                    )?;
                }
            }
        }
        db.foreign_keys_valid()
    })?;
    Ok(obj(&[
        ("policy", Value::string(POLICY)),
        ("source_certificate", Value::string(experiment)),
        ("snapshots", Value::array(reported)),
        (
            "warning",
            Value::string(
                "SOURCE_MATCH_ONLY is not a fresh observation; projected layouts remain unverified by source identity",
            ),
        ),
    ]))
}
