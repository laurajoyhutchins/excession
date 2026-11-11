//! Language-neutral declared-input identity and historical transport.
//!
//! No parser is privileged: Python, Go, JavaScript, Rust and future adapters
//! describe inputs through the same strict contract. Git authenticates file
//! identity, not completeness of a language adapter's dependency discovery.
//! Matching declared inputs is NOT a target test pass or a semantic proof.
use crate::digest;
use crate::git;
use crate::json::{self, Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const PLAN_SCHEMA: &str = "excession.input-declaration.v1";
const CONTRACT_SCHEMA: &str = "excession.input-contract.v1";
const TRANSPORT_SCHEMA: &str = "excession.input-transport.v1";
const SCOPE: &str = "DECLARED_INPUTS_ONLY";
const MAX_FILES: usize = 512;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_JSON: usize = 5 * 1024 * 1024;
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS declared_input_transport (
 transport_id TEXT PRIMARY KEY,
 source_commit TEXT NOT NULL, target_commit TEXT NOT NULL,
 source_contract_id TEXT NOT NULL, target_contract_id TEXT NOT NULL,
 source_json TEXT NOT NULL, target_json TEXT NOT NULL,
 comparison_json TEXT NOT NULL,
 decision TEXT NOT NULL CHECK(decision IN ('DECLARED_INPUTS_MATCH_ONLY','REVERIFY_REQUIRED')),
 target_behavior_verified INTEGER NOT NULL CHECK(target_behavior_verified=0)
);
CREATE INDEX IF NOT EXISTS declared_input_transport_by_source
 ON declared_input_transport(source_contract_id,target_commit);
"#;

fn read(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_JSON {
        return refuse("contract JSON exceeds 5 MiB");
    }
    json::parse(&bytes)
}
fn hash64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn language(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return refuse("invalid language adapter identifier");
    }
    Ok(())
}
fn role(s: &str) -> Result<()> {
    if !matches!(s, "source" | "asset" | "configuration") {
        return refuse("input role must be source, asset, or configuration");
    }
    Ok(())
}
fn assumptions(value: &Value) -> Result<()> {
    let map = value.entries()?;
    if map.len() > 32 {
        return refuse("too many declared environment assumptions");
    }
    for (key, v) in map {
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return refuse("invalid environment assumption key");
        }
        let text = v.str()?;
        if text.is_empty() || text.len() > 512 || text.contains('\0') {
            return refuse("invalid environment assumption value");
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Entry {
    path: String,
    mode: String,
    blob: String,
    sha256: String,
    bytes: i64,
    role: String,
}
impl Entry {
    fn value(&self) -> Value {
        obj(&[
            ("path", Value::string(&self.path)),
            ("mode", Value::string(&self.mode)),
            ("blob", Value::string(&self.blob)),
            ("sha256", Value::string(&self.sha256)),
            ("bytes", Value::number(self.bytes)),
            ("role", Value::string(&self.role)),
        ])
    }
}

/// Map independently resolved Git paths and roles to comparable identities.
/// This primitive is shared with the legacy Python slice transport adapter.
pub(crate) fn changed_paths(
    left: &BTreeMap<String, String>,
    right: &BTreeMap<String, String>,
) -> Vec<String> {
    left.keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|p| left.get(p) != right.get(p))
        .collect()
}

fn entries(repo: &Path, commit: &str, specs: &[(String, String)]) -> Result<Vec<Entry>> {
    if specs.is_empty() || specs.len() > MAX_FILES {
        return refuse("invalid input count");
    }
    let mut result = Vec::new();
    let mut total = 0usize;
    let mut last = "";
    for (path, r) in specs {
        git::validate_path(path)?;
        role(r)?;
        if path.as_str() <= last {
            return refuse("input paths must be strictly sorted and unique");
        }
        last = path;
        let ls = git::text(repo, &["ls-tree", commit, "--", path])?;
        let (header, shown) = ls.split_once('\t').ok_or("input not a tracked Git path")?;
        if shown != path {
            return refuse("Git tree path differs from input declaration");
        }
        let pieces = header.split(' ').collect::<Vec<_>>();
        if pieces.len() != 3 || pieces[1] != "blob" || !matches!(pieces[0], "100644" | "100755") {
            return refuse("input is not a regular tracked Git blob");
        }
        let blob = git::oid(pieces[2])?;
        let size = git::text(repo, &["cat-file", "-s", &blob])?
            .parse::<usize>()
            .map_err(|_| "invalid Git blob size")?;
        total = total.checked_add(size).ok_or("input size overflow")?;
        if total > MAX_BYTES {
            return refuse("input source bytes exceed 32 MiB");
        }
        let data = git::command(repo, &["cat-file", "blob", &blob])?;
        if data.len() != size {
            return refuse("Git blob size differs from content");
        }
        result.push(Entry {
            path: path.clone(),
            role: r.clone(),
            mode: pieces[0].into(),
            blob,
            sha256: digest::sha256(&data),
            bytes: size as i64,
        });
    }
    Ok(result)
}
fn identity(body: &Value, key: &str) -> Result<String> {
    let mut copy = body.clone();
    let claimed = copy
        .entries_mut()?
        .remove(key)
        .ok_or("missing contract identity")?;
    let hash = claimed.str()?;
    if !hash64(hash) || hash != copy.sha256() {
        return refuse("contract identity mismatch");
    }
    Ok(hash.to_owned())
}

/// Plan a contract from an explicit input declaration, without executing any
/// code and without depending on the current checkout or language runtime.
pub fn plan(repo: &Path, revision: &str, declaration: &Value) -> Result<Value> {
    if declaration.get("schema")?.str()? != PLAN_SCHEMA
        || declaration.get("coverage")?.str()? != SCOPE
    {
        return refuse("unsupported input declaration or coverage");
    }
    git::verify_repo(repo)?;
    if declaration.entries()?.len() != 6 {
        return refuse("unexpected input declaration fields");
    }
    if revision != "HEAD" {
        git::oid(revision)?;
    }
    let name = declaration.get("language")?.str()?;
    language(name)?;
    let subject = declaration.get("subject")?.str()?;
    git::validate_path(subject)?;
    let assumption = declaration.get("assumptions")?;
    assumptions(assumption)?;
    let raw = declaration.get("inputs")?.array_ref()?;
    let mut specs = Vec::new();
    for item in raw {
        specs.push((
            item.get("path")?.str()?.to_owned(),
            item.get("role")?.str()?.to_owned(),
        ));
        if item.entries()?.len() != 2 {
            return refuse("unexpected input declaration field");
        }
    }
    if !specs.iter().any(|(p, _)| p == subject) {
        return refuse("subject missing from declared input set");
    }
    let commit = git::rev(repo, &format!("{revision}^{{commit}}"))?;
    let tree = git::rev(repo, &format!("{commit}^{{tree}}"))?;
    let selected = entries(repo, &commit, &specs)?;
    let mut result = obj(&[
        ("schema", Value::string(CONTRACT_SCHEMA)),
        ("coverage", Value::string(SCOPE)),
        ("commit", Value::string(commit)),
        ("tree", Value::string(tree)),
        ("language", Value::string(name)),
        ("subject", Value::string(subject)),
        ("assumptions", assumption.clone()),
        ("entries", Value::array(selected.iter().map(Entry::value))),
        ("target_behavior_verified", Value::Bool(false)),
    ]);
    let hash = result.sha256();
    result
        .entries_mut()?
        .insert("contract_id".into(), Value::string(hash));
    Ok(result)
}

#[derive(Clone)]
struct Snapshot {
    id: String,
    commit: String,
    language: String,
    subject: String,
    assumptions: Value,
    inputs: BTreeMap<String, String>,
}
fn verify(repo: &Path, manifest: &Value) -> Result<Snapshot> {
    if manifest.get("schema")?.str()? != CONTRACT_SCHEMA
        || manifest.get("coverage")?.str()? != SCOPE
        || manifest.get("target_behavior_verified")?.bool()?
    {
        return refuse("unsupported input contract status");
    }
    let id = identity(manifest, "contract_id")?;
    let commit = git::oid(manifest.get("commit")?.str()?)?;
    let raw = manifest.get("entries")?.array_ref()?;
    let specs: Vec<(String, String)> = raw
        .iter()
        .map(|v| {
            Ok((
                v.get("path")?.str()?.to_owned(),
                v.get("role")?.str()?.to_owned(),
            ))
        })
        .collect::<Result<_>>()?;
    let declaration = obj(&[
        ("schema", Value::string(PLAN_SCHEMA)),
        ("coverage", Value::string(SCOPE)),
        ("language", manifest.get("language")?.clone()),
        ("subject", manifest.get("subject")?.clone()),
        ("assumptions", manifest.get("assumptions")?.clone()),
        (
            "inputs",
            Value::array(
                specs
                    .iter()
                    .map(|(p, r)| obj(&[("path", Value::string(p)), ("role", Value::string(r))])),
            ),
        ),
    ]);
    let expected = plan(repo, &commit, &declaration)?;
    if &expected != manifest {
        return refuse("contract differs from independently derived Git source");
    }
    let mut inputs = BTreeMap::new();
    for e in raw {
        let v = format!(
            "{}:{}:{}:{}",
            e.get("blob")?.str()?,
            e.get("mode")?.str()?,
            e.get("sha256")?.str()?,
            e.get("role")?.str()?
        );
        inputs.insert(e.get("path")?.str()?.to_owned(), v);
    }
    Ok(Snapshot {
        id,
        commit,
        language: manifest.get("language")?.str()?.to_owned(),
        subject: manifest.get("subject")?.str()?.to_owned(),
        assumptions: manifest.get("assumptions")?.clone(),
        inputs,
    })
}

fn compare(source: &Snapshot, target: &Snapshot) -> Value {
    let changed = changed_paths(&source.inputs, &target.inputs);
    let compatible = source.language == target.language
        && source.subject == target.subject
        && source.assumptions == target.assumptions;
    let matched = changed.is_empty() && compatible;
    obj(&[
        ("schema", Value::string(TRANSPORT_SCHEMA)),
        ("source_commit", Value::string(&source.commit)),
        ("target_commit", Value::string(&target.commit)),
        ("source_contract_id", Value::string(&source.id)),
        ("target_contract_id", Value::string(&target.id)),
        (
            "changed_input_paths",
            Value::array(changed.into_iter().map(Value::string)),
        ),
        (
            "subject_unchanged",
            Value::Bool(source.subject == target.subject),
        ),
        (
            "language_unchanged",
            Value::Bool(source.language == target.language),
        ),
        (
            "assumptions_unchanged",
            Value::Bool(source.assumptions == target.assumptions),
        ),
        (
            "decision",
            Value::string(if matched {
                "DECLARED_INPUTS_MATCH_ONLY"
            } else {
                "REVERIFY_REQUIRED"
            }),
        ),
        ("scope", Value::string(SCOPE)),
        ("source_behavior_verified", Value::Bool(false)),
        ("target_behavior_verified", Value::Bool(false)),
    ])
}

/// Store an exact-source comparison, then independently replay it on demand.
/// This does NOT assume a prior test run, a complete dependency graph, or
/// any source-to-target behavioral equivalence.
pub fn transport(
    repo: &Path,
    left_path: &Path,
    right_path: &Path,
    db: &Db,
    audit: bool,
) -> Result<Value> {
    let left_json = read(left_path)?;
    let right_json = read(right_path)?;
    let left = verify(repo, &left_json)?;
    let right = verify(repo, &right_json)?;
    let result = compare(&left, &right);
    let transport_id =
        digest::sha256(format!("excession-input-transport-v1\0{}", result.canonical()).as_bytes());
    let decision = result.get("decision")?.str()?;
    let source_json = left_json.canonical();
    let target_json = right_json.canonical();
    let compare_json = result.canonical();
    let expected = vec![vec![
        Some(transport_id.clone()),
        Some(left.commit.clone()),
        Some(right.commit.clone()),
        Some(left.id.clone()),
        Some(right.id.clone()),
        Some(source_json.clone()),
        Some(target_json.clone()),
        Some(compare_json.clone()),
        Some(decision.to_owned()),
        Some("0".into()),
    ]];
    if audit {
        let rows = db.query("SELECT transport_id,source_commit,target_commit,source_contract_id,target_contract_id,source_json,target_json,comparison_json,decision,target_behavior_verified FROM declared_input_transport WHERE transport_id=?", &[Param::Text(&transport_id)])?;
        if rows != expected {
            return refuse("declared-input transport differs from independent audit");
        }
    } else {
        db.exec(SCHEMA)?;
        db.transaction(|db| {
            if !db
                .query(
                    "SELECT transport_id FROM declared_input_transport WHERE transport_id=?",
                    &[Param::Text(&transport_id)],
                )?
                .is_empty()
            {
                return refuse("duplicate declared-input transport; use audit");
            }
            db.execute(
                "INSERT INTO declared_input_transport VALUES(?,?,?,?,?,?,?,?,?,?)",
                &[
                    Param::Text(&transport_id),
                    Param::Text(&left.commit),
                    Param::Text(&right.commit),
                    Param::Text(&left.id),
                    Param::Text(&right.id),
                    Param::Text(&source_json),
                    Param::Text(&target_json),
                    Param::Text(&compare_json),
                    Param::Text(decision),
                    Param::Integer(0),
                ],
            )?;
            Ok(())
        })?;
    }
    db.foreign_keys_valid()?;
    let mut out = result;
    out.entries_mut()?
        .insert("transport_id".into(), Value::string(transport_id));
    out.entries_mut()?
        .insert("independently_audited".into(), Value::Bool(audit));
    Ok(out)
}

pub fn plan_file(repo: &Path, revision: &str, declaration_path: &Path) -> Result<Value> {
    plan(repo, revision, &read(declaration_path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_change_detector() {
        let a = BTreeMap::from([("one".into(), "abc".into()), ("two".into(), "def".into())]);
        let b = BTreeMap::from([("one".into(), "abc".into()), ("two".into(), "xyz".into())]);
        assert_eq!(changed_paths(&a, &b), vec!["two"]);
    }
    #[test]
    fn declared_metadata_validation() {
        assert!(language("javascript").is_ok());
        assert!(language("rust").is_ok());
        assert!(language("go").is_ok());
        assert!(language("c++").is_err());
        assert!(role("generated").is_err());
        assert!(assumptions(&obj(&[("compiler", Value::string("go1.25"))])).is_ok());
        assert!(assumptions(&obj(&[("Compiler", Value::string("bad"))])).is_err());
    }
}
