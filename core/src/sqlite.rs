//! Minimal SQLite C-API wrapper; every SQL value is bound (never interpolated).
//! All unsafe code is isolated here. The raw connection is owned and never shared.
use crate::{Result, refuse};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::{self, NonNull};

#[repr(C)]
struct Sqlite {
    _opaque: [u8; 0],
}
#[repr(C)]
struct Stmt {
    _opaque: [u8; 0],
}
const OK: c_int = 0;
const ROW: c_int = 100;
const DONE: c_int = 101;
const NULL: c_int = 5;

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open(path: *const c_char, db: *mut *mut Sqlite) -> c_int;
    fn sqlite3_close(db: *mut Sqlite) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite) -> *const c_char;
    fn sqlite3_exec(
        db: *mut Sqlite,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        data: *mut c_void,
        error: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(data: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut Sqlite,
        sql: *const c_char,
        nbyte: c_int,
        stmt: *mut *mut Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Stmt,
        idx: c_int,
        data: *const c_char,
        nbyte: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Stmt, idx: c_int, value: i64) -> c_int;
    fn sqlite3_bind_null(stmt: *mut Stmt, idx: c_int) -> c_int;
    fn sqlite3_step(stmt: *mut Stmt) -> c_int;
    fn sqlite3_column_count(stmt: *mut Stmt) -> c_int;
    fn sqlite3_column_type(stmt: *mut Stmt, idx: c_int) -> c_int;
    fn sqlite3_column_text(stmt: *mut Stmt, idx: c_int) -> *const u8;
    fn sqlite3_column_bytes(stmt: *mut Stmt, idx: c_int) -> c_int;
    fn sqlite3_finalize(stmt: *mut Stmt) -> c_int;
}

pub enum Param<'a> {
    Null,
    Text(&'a str),
    Integer(i64),
}

pub struct Db {
    ptr: NonNull<Sqlite>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // SQLite accepts UTF-8 filenames on Unix and Windows. Never silently
        // replace unrepresentable path characters when opening evidence stores.
        let pathname = path.to_str().ok_or("non-UTF-8 database path")?;
        let cpath = CString::new(pathname).map_err(|_| "NUL in db path")?;
        let mut raw = ptr::null_mut();
        // SAFETY: sqlite3_open initializes raw or returns an error handle; path is NUL-terminated.
        let rc = unsafe { sqlite3_open(cpath.as_ptr(), &mut raw) };
        if rc != OK {
            let msg = if raw.is_null() {
                "sqlite3_open failed".to_owned()
            } else {
                // SAFETY: non-null SQLite error handle remains valid until close.
                unsafe { CStr::from_ptr(sqlite3_errmsg(raw)) }
                    .to_string_lossy()
                    .into_owned()
            };
            if !raw.is_null() {
                unsafe { sqlite3_close(raw) };
            }
            return refuse(msg);
        }
        let ptr = NonNull::new(raw).ok_or("sqlite3_open returned null")?;
        let db = Db { ptr };
        db.exec("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        Ok(db)
    }
    fn error(&self) -> String {
        // SAFETY: `self.ptr` is valid throughout Db's lifetime.
        unsafe { CStr::from_ptr(sqlite3_errmsg(self.ptr.as_ptr())) }
            .to_string_lossy()
            .into_owned()
    }
    pub fn exec(&self, sql: &str) -> Result<()> {
        let s = CString::new(sql).map_err(|_| "SQL contains NUL")?;
        let mut err = ptr::null_mut();
        // SAFETY: pointers remain valid for sqlite3_exec's synchronous call.
        let status = unsafe {
            sqlite3_exec(
                self.ptr.as_ptr(),
                s.as_ptr(),
                None,
                ptr::null_mut(),
                &mut err,
            )
        };
        if status != OK {
            let detail = if err.is_null() {
                self.error()
            } else {
                // SAFETY: SQLite owns this allocated NUL-terminated error until sqlite3_free.
                let msg = unsafe { CStr::from_ptr(err) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { sqlite3_free(err.cast()) };
                msg
            };
            return refuse(format!("SQLite {status}: {detail}"));
        }
        Ok(())
    }
    pub fn query(&self, sql: &str, args: &[Param<'_>]) -> Result<Vec<Vec<Option<String>>>> {
        let csql = CString::new(sql).map_err(|_| "SQL contains NUL")?;
        let mut raw = ptr::null_mut();
        // SAFETY: pointer to `raw` valid and SQL NUL-terminated.
        let rc = unsafe {
            sqlite3_prepare_v2(
                self.ptr.as_ptr(),
                csql.as_ptr(),
                -1,
                &mut raw,
                ptr::null_mut(),
            )
        };
        if rc != OK {
            return refuse(format!("SQLite prepare: {}", self.error()));
        }
        let stmt = NonNull::new(raw).ok_or("SQLite prepared null statement")?;
        struct Guard(NonNull<Stmt>);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe { sqlite3_finalize(self.0.as_ptr()) };
            }
        }
        let guard = Guard(stmt);
        for (i, param) in args.iter().enumerate() {
            let idx = c_int::try_from(i + 1).map_err(|_| "too many bind parameters")?;
            // `None` destructor binds static pointers: args and each str remain alive
            // until stepping finishes below. Rust references forbid concurrent mutation.
            let status = match param {
                Param::Null => unsafe { sqlite3_bind_null(guard.0.as_ptr(), idx) },
                Param::Text(s) => {
                    let len = c_int::try_from(s.len())
                        .map_err(|_| "text parameter exceeds SQLite limit")?;
                    unsafe {
                        sqlite3_bind_text(guard.0.as_ptr(), idx, s.as_ptr().cast(), len, None)
                    }
                }
                Param::Integer(n) => unsafe { sqlite3_bind_int64(guard.0.as_ptr(), idx, *n) },
            };
            if status != OK {
                return refuse(format!("SQLite bind: {}", self.error()));
            }
        }
        let mut result = Vec::new();
        loop {
            // SAFETY: prepared statement valid; bound borrowed buffers still in scope.
            match unsafe { sqlite3_step(guard.0.as_ptr()) } {
                ROW => {
                    let columns = unsafe { sqlite3_column_count(guard.0.as_ptr()) };
                    let mut row = Vec::new();
                    for col in 0..columns {
                        if unsafe { sqlite3_column_type(guard.0.as_ptr(), col) } == NULL {
                            row.push(None);
                            continue;
                        }
                        let ptr = unsafe { sqlite3_column_text(guard.0.as_ptr(), col) };
                        let len = unsafe { sqlite3_column_bytes(guard.0.as_ptr(), col) };
                        if len < 0 || ptr.is_null() {
                            return refuse("SQLite returned invalid column bytes");
                        }
                        // SAFETY: SQLite column data remains valid until next step/finalize;
                        // copy immediately into owned Rust String.
                        let data = unsafe {
                            std::slice::from_raw_parts(
                                ptr,
                                usize::try_from(len).map_err(|_| "negative length")?,
                            )
                        };
                        let str =
                            String::from_utf8(data.to_vec()).map_err(|_| "non-UTF-8 SQL result")?;
                        row.push(Some(str));
                    }
                    result.push(row);
                }
                DONE => break,
                _ => return refuse(format!("SQLite step: {}", self.error())),
            }
        }
        Ok(result)
    }
    pub fn execute(&self, sql: &str, args: &[Param<'_>]) -> Result<()> {
        let rows = self.query(sql, args)?;
        if !rows.is_empty() {
            return refuse("SQL mutation returned unexpected rows");
        }
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
    pub fn transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.exec("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(value) => {
                if let Err(e) = self.exec("COMMIT") {
                    let _ = self.exec("ROLLBACK");
                    return Err(e);
                }
                Ok(value)
            }
            Err(e) => {
                let _ = self.exec("ROLLBACK");
                Err(e)
            }
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        unsafe { sqlite3_close(self.ptr.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fk_and_binding() {
        let d = std::env::temp_dir().join(format!(
            "excession-sqlite-test-{}-{}.db",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
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
