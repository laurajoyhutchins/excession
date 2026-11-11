//! Read-only queries over indexed Git revisions.
//! A source relationship is descriptive, not permission to omit verification.
use crate::index;
use crate::json::{Value, obj};
use crate::sqlite::{Db, Param};
use crate::{Result, refuse};
use std::collections::{BTreeMap, BTreeSet};

type SqlRow = Vec<Option<String>>;
type SymbolKey = (String, String);
type SymbolTable = BTreeMap<SymbolKey, SymbolFingerprint>;

#[derive(Debug, PartialEq, Eq)]
struct SymbolFingerprint {
    kind: String,
    signature: String,
    body: String,
}

/// SQLite's dynamic row representation must not silently turn a missing value
/// into an empty symbol, path or source identity.
fn required<'a>(row: &'a SqlRow, column: usize, name: &str) -> Result<&'a str> {
    row.get(column)
        .and_then(Option::as_deref)
        .ok_or_else(|| format!("missing SQL {name}"))
}

fn required_integer(row: &SqlRow, column: usize, name: &str) -> Result<i64> {
    required(row, column, name)?
        .parse::<i64>()
        .map_err(|_| format!("invalid SQL integer {name}"))
}

pub fn repo_id(db: &Db, name: &str) -> Result<i64> {
    db.scalar("SELECT id FROM repository WHERE name=?", &[Param::Text(name)])?
        .parse()
        .map_err(|e| format!("invalid repo ID: {e}"))
}

pub fn impact(db: &Db, name: &str, revision: &str, seed: &str) -> Result<Value> {
    let repo = repo_id(db, name)?;
    let present = db.query(
        "SELECT 1 FROM module WHERE repo_id=? AND commit_oid=? AND name=?",
        &[Param::Integer(repo), Param::Text(revision), Param::Text(seed)],
    )?;
    if present.len() != 1 {
        return refuse("unknown indexed module");
    }

    // Reverse transitive dependency closure, including the seed itself.
    let rows = db.query(
        "WITH RECURSIVE impacted(name) AS (
            SELECT ? UNION
            SELECT source_name FROM dependency d
            JOIN impacted i ON i.name=d.target_name
            WHERE d.repo_id=? AND d.commit_oid=?
        )
        SELECT m.name,m.path,m.is_test FROM impacted i
        JOIN module m ON m.name=i.name
        WHERE m.repo_id=? AND m.commit_oid=? ORDER BY m.name",
        &[
            Param::Text(seed),
            Param::Integer(repo),
            Param::Text(revision),
            Param::Integer(repo),
            Param::Text(revision),
        ],
    )?;

    let mut affected = Vec::with_capacity(rows.len());
    let mut tests = Vec::new();
    for row in &rows {
        let name = required(row, 0, "module name")?;
        let path = required(row, 1, "module path")?;
        let is_test = required_integer(row, 2, "test flag")?;
        if !matches!(is_test, 0 | 1) {
            return refuse("invalid indexed test flag");
        }
        affected.push(Value::string(name));
        if is_test == 1 {
            tests.push(Value::string(path));
        }
    }

    let hazards = db
        .query(
            "SELECT module_name,line,reason FROM hazard
             WHERE repo_id=? AND commit_oid=? ORDER BY module_name,line,reason",
            &[Param::Integer(repo), Param::Text(revision)],
        )?
        .into_iter()
        .map(|row| {
            Ok(obj(&[
                ("module_name", Value::string(required(&row, 0, "hazard module")?)),
                ("line", Value::number(required_integer(&row, 1, "hazard line")?)),
                ("reason", Value::string(required(&row, 2, "hazard reason")?)),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(obj(&[
        ("seed", Value::string(seed)),
        ("affected_modules", Value::array(affected)),
        ("candidate_tests", Value::array(tests)),
        ("global_hazards", Value::array(hazards)),
        ("safe_to_skip_unlisted_tests", Value::Bool(false)),
    ]))
}

fn load_symbols(db: &Db, repo: i64, revision: &str) -> Result<SymbolTable> {
    let rows = db.query(
        "SELECT module_name,qualname,kind,signature,body_fingerprint FROM symbol
         WHERE repo_id=? AND commit_oid=?",
        &[Param::Integer(repo), Param::Text(revision)],
    )?;
    let mut symbols = BTreeMap::new();
    for row in rows {
        let key = (
            required(&row, 0, "symbol module")?.to_owned(),
            required(&row, 1, "symbol qualified name")?.to_owned(),
        );
        let fingerprint = SymbolFingerprint {
            kind: required(&row, 2, "symbol kind")?.to_owned(),
            signature: required(&row, 3, "symbol signature")?.to_owned(),
            body: required(&row, 4, "symbol body fingerprint")?.to_owned(),
        };
        if symbols.insert(key, fingerprint).is_some() {
            return refuse("duplicate indexed symbol identity");
        }
    }
    Ok(symbols)
}

fn symbol_list(keys: impl IntoIterator<Item = SymbolKey>) -> Value {
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
}

pub fn review(db: &Db, name: &str, old: &str, new: &str) -> Result<Value> {
    let repo = repo_id(db, name)?;
    let files_changed = index::changed_paths(db, repo, old, new)?
        .into_iter()
        .map(|(path, before, after)| {
            obj(&[
                ("path", Value::string(path)),
                ("old", before.map(Value::string).unwrap_or(Value::Null)),
                ("new", after.map(Value::string).unwrap_or(Value::Null)),
            ])
        })
        .collect::<Vec<_>>();

    let before = load_symbols(db, repo, old)?;
    let after = load_symbols(db, repo, new)?;
    let old_keys = before.keys().cloned().collect::<BTreeSet<_>>();
    let new_keys = after.keys().cloned().collect::<BTreeSet<_>>();

    let added = new_keys.difference(&old_keys).cloned();
    let removed = old_keys.difference(&new_keys).cloned();
    let modified = old_keys
        .intersection(&new_keys)
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned();

    Ok(obj(&[
        ("files_changed", Value::array(files_changed)),
        ("symbols_added", symbol_list(added)),
        ("symbols_removed", symbol_list(removed)),
        ("symbols_modified", symbol_list(modified)),
        (
            "limitation",
            Value::string(
                "Symbol identities are module-qualified; moves and runtime dynamic behavior need separate analysis.",
            ),
        ),
    ]))
}

pub fn manifest(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let repo = repo_id(db, name)?;
    let rows = db.query(
        "SELECT path,mode,kind,oid FROM tree_entry
         WHERE repo_id=? AND commit_oid=? ORDER BY path",
        &[Param::Integer(repo), Param::Text(revision)],
    )?;
    let entries = rows
        .into_iter()
        .map(|row| {
            Ok(obj(&[
                ("path", Value::string(required(&row, 0, "tree path")?)),
                ("mode", Value::string(required(&row, 1, "tree mode")?)),
                ("kind", Value::string(required(&row, 2, "object kind")?)),
                ("oid", Value::string(required(&row, 3, "object ID")?)),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::array(entries))
}

pub fn dependencies(db: &Db, name: &str, revision: &str) -> Result<Value> {
    let repo = repo_id(db, name)?;
    let rows = db.query(
        "SELECT source_name,target_name,line,syntax FROM dependency
         WHERE repo_id=? AND commit_oid=? ORDER BY source_name,target_name,line",
        &[Param::Integer(repo), Param::Text(revision)],
    )?;
    let edges = rows
        .into_iter()
        .map(|row| {
            Ok(obj(&[
                ("source_name", Value::string(required(&row, 0, "dependency source")?)),
                ("target_name", Value::string(required(&row, 1, "dependency target")?)),
                ("line", Value::number(required_integer(&row, 2, "dependency line")?)),
                ("syntax", Value::string(required(&row, 3, "dependency syntax")?)),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::array(edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_sql_rows_fail_closed() {
        let missing = vec![Some("src/module.py".to_owned()), None];
        assert_eq!(required(&missing, 0, "path").unwrap(), "src/module.py");
        assert!(required(&missing, 1, "mode").is_err());
        assert!(required(&missing, 3, "OID").is_err());

        let invalid_line = vec![Some("not a number".to_owned())];
        assert!(required_integer(&invalid_line, 0, "line").is_err());
    }

    #[test]
    fn modified_symbol_fingerprint_preserves_symbol_identity() {
        let a = SymbolFingerprint {
            kind: "function".into(),
            signature: "f(x)".into(),
            body: "digest-a".into(),
        };
        let b = SymbolFingerprint {
            kind: "function".into(),
            signature: "f(x)".into(),
            body: "digest-b".into(),
        };
        assert_ne!(a, b);
        assert_eq!(
            symbol_list([("package.mod".into(), "f".into())]).canonical(),
            r#"[{"module":"package.mod","symbol":"f"}]"#
        );
    }
}
