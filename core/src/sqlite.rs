//! SQLite persistence through rusqlite's bundled SQLite engine.
//! Preserve the existing SQL, schema and string-row facade for callers.
//! No raw SQLite FFI, platform-specific linking or process-global state.
use crate::{Result, refuse};
use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, ToSql, params_from_iter};
use std::path::Path;

pub enum Param<'a> {
    Null,
    Text(&'a str),
    Integer(i64),
}

impl ToSql for Param<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let value = match self {
            Self::Null => ValueRef::Null,
            Self::Text(text) => ValueRef::Text(text.as_bytes()),
            Self::Integer(value) => ValueRef::Integer(*value),
        };
        Ok(ToSqlOutput::Borrowed(value))
    }
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        // Retain the original refusal for paths that cannot be represented in
        // UTF-8 instead of silently changing persisted evidence store names.
        path.to_str().ok_or("non-UTF-8 database path")?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        // Indexing repeats a bounded family of INSERT and SELECT statements
        // for every source row. Keep their SQLite VM programs per connection.
        conn.set_prepared_statement_cache_capacity(64);
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
            .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn version() -> &'static str {
        rusqlite::version()
    }

    pub fn exec(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql).map_err(|e| e.to_string())
    }

    /// Preserve the existing row contract: every non-NULL SQLite column is
    /// returned as UTF-8 text. Numeric values are decimal strings.
    pub fn query(&self, sql: &str, args: &[Param<'_>]) -> Result<Vec<Vec<Option<String>>>> {
        let mut statement = self.conn.prepare_cached(sql).map_err(|e| e.to_string())?;
        let column_count = statement.column_count();
        let mut cursor = statement
            .query(params_from_iter(args.iter()))
            .map_err(|e| e.to_string())?;
        let mut output = Vec::new();
        while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
            let mut fields = Vec::with_capacity(column_count);
            for index in 0..column_count {
                let field = match row.get_ref(index).map_err(|e| e.to_string())? {
                    ValueRef::Null => None,
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => Some(
                        std::str::from_utf8(bytes)
                            .map_err(|_| "non-UTF-8 SQL result")?
                            .to_owned(),
                    ),
                    ValueRef::Integer(value) => Some(value.to_string()),
                    ValueRef::Real(value) => Some(value.to_string()),
                };
                fields.push(field);
            }
            output.push(fields);
        }
        Ok(output)
    }

    pub fn execute(&self, sql: &str, args: &[Param<'_>]) -> Result<()> {
        // Mutations must not materialize result rows. Prepared statements are
        // reused by the same SQLite connection, without a second SQL path.
        let mut statement = self.conn.prepare_cached(sql).map_err(|e| e.to_string())?;
        // SQLite RETURNING can perform mutations before execute() notices
        // rows. Reject row-producing statements before stepping the VM.
        if statement.column_count() != 0 {
            return refuse("SQL mutation returned unexpected rows");
        }
        statement
            .execute(params_from_iter(args.iter()))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn scalar(&self, sql: &str, args: &[Param<'_>]) -> Result<String> {
        let rows = self.query(sql, args)?;
        if rows.len() != 1 || rows[0].len() != 1 {
            return refuse("expected exactly one SQL scalar");
        }
        rows[0][0].clone().ok_or("unexpected SQL NULL".into())
    }

    pub fn init_schema(&self) -> Result<()> {
        self.exec(include_str!("schema.sql"))
    }

    pub fn foreign_keys_valid(&self) -> Result<()> {
        let bad = self.query("PRAGMA foreign_key_check", &[])?;
        if !bad.is_empty() {
            return refuse(format!("foreign-key violations: {bad:?}"));
        }
        Ok(())
    }

    fn run_transaction<T>(&self, begin: &str, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.exec(begin)?;
        match f(self) {
            Ok(value) => {
                if let Err(error) = self.exec("COMMIT") {
                    let _ = self.exec("ROLLBACK");
                    return Err(error);
                }
                Ok(value)
            }
            Err(error) => {
                let _ = self.exec("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.run_transaction("BEGIN IMMEDIATE", f)
    }

    /// Read a consistent SQLite snapshot without acquiring a write reservation.
    /// The first read pins the snapshot for the complete inspection/audit.
    pub fn read_transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.run_transaction("BEGIN", f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

    fn temp_database(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}.db",
            std::process::id(),
            NEXT_DATABASE.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn existing_file_and_row_contract() {
        let file = temp_database("excession-bundled-sqlite");
        let _ = std::fs::remove_file(&file);
        {
            let db = Db::open(&file).expect("create");
            db.exec("CREATE TABLE sample(id INTEGER PRIMARY KEY, description TEXT, raw BLOB)")
                .expect("schema");
            db.execute(
                "INSERT INTO sample(id, description, raw) VALUES (?1, ?2, ?3)",
                &[
                    Param::Integer(42),
                    Param::Text("quoted ' literal"),
                    Param::Text("raw text"),
                ],
            )
            .expect("bound insert");
            assert_eq!(
                db.query("SELECT id, description, raw, NULL FROM sample", &[])
                    .unwrap(),
                vec![vec![
                    Some("42".to_owned()),
                    Some("quoted ' literal".to_owned()),
                    Some("raw text".to_owned()),
                    None,
                ]]
            );
            db.transaction(|db| {
                db.execute(
                    "INSERT INTO sample(id, description) VALUES (?, ?)",
                    &[Param::Integer(43), Param::Text("committed")],
                )?;
                Ok(())
            })
            .expect("commit");
            assert!(
                db.transaction::<()>(|db| {
                    db.execute("INSERT INTO sample(id) VALUES (?)", &[Param::Integer(44)])?;
                    refuse("intentional rollback")
                })
                .is_err()
            );
            assert_eq!(
                db.read_transaction(|db| db.scalar("SELECT count(*) FROM sample", &[]))
                    .unwrap(),
                "2"
            );
            assert!(
                db.read_transaction::<()>(|db| {
                    assert_eq!(db.scalar("SELECT count(*) FROM sample", &[])?, "2");
                    refuse("intentional read-transaction rollback")
                })
                .is_err()
            );
            assert_eq!(db.scalar("SELECT count(*) FROM sample", &[]).unwrap(), "2");
            assert!(db.execute("SELECT * FROM sample", &[]).is_err());
            assert!(
                db.execute(
                    "INSERT INTO sample(id,description) VALUES(45,'must refuse') RETURNING id",
                    &[],
                )
                .is_err()
            );
            assert_eq!(
                db.scalar("SELECT COUNT(*) FROM sample", &[]).unwrap(),
                "2",
                "refusing a returning mutation must leave the transaction unchanged"
            );
            for id in 50..70 {
                db.execute(
                    "INSERT INTO sample(id,description) VALUES(?,?)",
                    &[Param::Integer(id), Param::Text("prepared cached")],
                )
                .unwrap();
            }
            assert_eq!(db.scalar("SELECT COUNT(*) FROM sample", &[]).unwrap(), "22");
        }
        // This is an ordinary SQLite file, readable after closing and
        // independently of the connection that originally created it.
        let header = std::fs::read(&file).expect("persisted database");
        assert!(header.starts_with(b"SQLite format 3\0"));
        {
            let db = Db::open(&file).expect("reopen");
            assert_eq!(
                db.scalar("SELECT description FROM sample WHERE id=43", &[])
                    .unwrap(),
                "committed"
            );
            assert_eq!(db.scalar("SELECT count(*) FROM sample", &[]).unwrap(), "22");
        }
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn fk_and_binding() {
        let d = temp_database("excession-sqlite-test");
        let _ = std::fs::remove_file(&d);
        let db = Db::open(&d).expect("open");
        db.init_schema().expect("schema");
        db.execute(
            "INSERT INTO repository(name,git_dir,provenance) VALUES(?,?,?)",
            &[
                Param::Text("some ' quote"),
                Param::Text("/tmp"),
                Param::Text("test"),
            ],
        )
        .expect("insert");
        assert_eq!(
            db.scalar("SELECT name FROM repository", &[])
                .expect("select"),
            "some ' quote"
        );
        assert!(
            db.execute(
                "INSERT INTO git_object(repo_id,oid,kind) VALUES(77,'dead','blob')",
                &[]
            )
            .is_err()
        );
        db.foreign_keys_valid().expect("fks");
        drop(db);
        let _ = std::fs::remove_file(d);
    }
}
