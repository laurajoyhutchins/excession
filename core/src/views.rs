//! Query-only views over the Git/SQLite index. These are descriptive, not
//! permissions to skip tests or merge without fresh verification.
use crate::index;
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};

pub fn repo_id(db: &Db, name: &str) -> Result<i64> {
    db.scalar(
        "SELECT id FROM repository WHERE name=?",
        &[Param::Text(name)],
    )?
    .parse()
    .map_err(|e| format!("invalid repo ID: {e}"))
}

pub fn impact(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Value> {
    let id = repo_id(db, name)?;
    let present = db.query(
        "SELECT 1 FROM module WHERE repo_id=? AND commit_oid=? AND name=?",
        &[Param::Integer(id), Param::Text(revision), Param::Text(seed)],
    )?;
    if present.len() != 1 {
        return refuse("unknown indexed module");
    }
    let rows=db.query("WITH RECURSIVE impacted(name) AS (SELECT ? UNION SELECT source_name FROM dependency d JOIN impacted i ON i.name=d.target_name WHERE d.repo_id=? AND d.commit_oid=?) SELECT m.name,m.path,m.is_test FROM impacted i JOIN module m ON m.name=i.name WHERE m.repo_id=? AND m.commit_oid=? ORDER BY m.name",&[Param::Text(seed),Param::Integer(id),Param::Text(revision),Param::Integer(id),Param::Text(revision)])?;
    let affected = rows
        .iter()
        .map(|r| Value::string(r[0].as_deref().unwrap_or_default()))
        .collect::<Vec<_>>();
    let tests = rows
        .iter()
        .filter(|r| r[2].as_deref() == Some("1"))
        .map(|r| Value::string(r[1].as_deref().unwrap_or_default()))
        .collect::<Vec<_>>();
    let hazards=db.query("SELECT module_name,line,reason FROM hazard WHERE repo_id=? AND commit_oid=? ORDER BY module_name,line,reason",&[Param::Integer(id),Param::Text(revision)])?.into_iter().map(|r|obj(&[
        ("module_name",Value::string(r[0].as_deref().unwrap_or_default())),
        ("line",Value::number(r[1].as_deref().unwrap_or("0"))),
        ("reason",Value::string(r[2].as_deref().unwrap_or_default())),
    ])).collect::<Vec<_>>();
    Ok(obj(&[
        ("seed", Value::string(seed)),
        ("affected_modules", Value::array(affected)),
        ("candidate_tests", Value::array(tests)),
        ("global_hazards", Value::array(hazards)),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]))
}

pub fn review(db: &Db, name: &str, old: &str, new: &str) -> Result<Value> {
    let id = repo_id(db, name)?;
    let changed = index::changed_paths(db, id, old, new)?
        .into_iter()
        .map(|(p, a, b)| {
            obj(&[
                ("path", Value::string(p)),
                ("old", a.map(Value::string).unwrap_or(Value::Null)),
                ("new", b.map(Value::string).unwrap_or(Value::Null)),
            ])
        })
        .collect::<Vec<_>>();
    type SymbolTable = BTreeMap<(String, String), (String, String, String)>;
    fn symbols(db: &Db, id: i64, revision: &str) -> Result<SymbolTable> {
        let rows=db.query("SELECT module_name,qualname,kind,signature,body_fingerprint FROM symbol WHERE repo_id=? AND commit_oid=?",&[Param::Integer(id),Param::Text(revision)])?;
        let mut result = BTreeMap::new();
        for r in rows {
            let strings = r
                .into_iter()
                .map(|v| v.ok_or_else(|| "null symbol metadata".to_string()))
                .collect::<Result<Vec<_>>>()?;
            result.insert(
                (strings[0].clone(), strings[1].clone()),
                (strings[2].clone(), strings[3].clone(), strings[4].clone()),
            );
        }
        Ok(result)
    }
    let a = symbols(db, id, old)?;
    let b = symbols(db, id, new)?;
    let ka = a.keys().cloned().collect::<BTreeSet<_>>();
    let kb = b.keys().cloned().collect::<BTreeSet<_>>();
    let change = |keys: Vec<(String, String)>| {
        Value::array(
            keys.into_iter()
                .map(|(module, symbol)| {
                    obj(&[
                        ("module", Value::string(module)),
                        ("symbol", Value::string(symbol)),
                    ])
                })
                .collect::<Vec<_>>(),
        )
    };
    let added = kb.difference(&ka).cloned().collect();
    let removed = ka.difference(&kb).cloned().collect();
    let modified = ka
        .intersection(&kb)
        .filter(|key| a.get(*key) != b.get(*key))
        .cloned()
        .collect();
    Ok(obj(&[
        ("files_changed", Value::array(changed)),
        ("symbols_added", change(added)),
        ("symbols_removed", change(removed)),
        ("symbols_modified", change(modified)),
        (
            "limitation",
            Value::string(
                "Symbol identities are module-qualified; moves and runtime dynamic behavior need separate analysis.",
            ),
        ),
    ]))
}

pub fn manifest(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let id = repo_id(db, name)?;
    let result = db.query(
        "SELECT path,mode,kind,oid FROM tree_entry WHERE repo_id=? AND commit_oid=? ORDER BY path",
        &[Param::Integer(id), Param::Text(revision)],
    )?;
    Ok(Value::array(
        result
            .into_iter()
            .map(|row| {
                obj(&[
                    ("path", Value::string(row[0].as_deref().unwrap_or_default())),
                    ("mode", Value::string(row[1].as_deref().unwrap_or_default())),
                    ("kind", Value::string(row[2].as_deref().unwrap_or_default())),
                    ("oid", Value::string(row[3].as_deref().unwrap_or_default())),
                ])
            })
            .collect::<Vec<_>>(),
    ))
}

pub fn dependencies(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let id = repo_id(db, name)?;
    let result=db.query("SELECT source_name,target_name,line,syntax FROM dependency WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name,line",&[Param::Integer(id),Param::Text(revision)])?;
    Ok(Value::array(
        result
            .into_iter()
            .map(|row| {
                obj(&[
                    (
                        "source_name",
                        Value::string(row[0].as_deref().unwrap_or_default()),
                    ),
                    (
                        "target_name",
                        Value::string(row[1].as_deref().unwrap_or_default()),
                    ),
                    ("line", Value::number(row[2].as_deref().unwrap_or("0"))),
                    (
                        "syntax",
                        Value::string(row[3].as_deref().unwrap_or_default()),
                    ),
                ])
            })
            .collect::<Vec<_>>(),
    ))
}
