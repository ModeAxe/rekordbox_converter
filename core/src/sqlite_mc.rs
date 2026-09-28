//! Minimal bindings to the bundled SQLite3 Multiple Ciphers amalgamation.
//!
//! Used only to read and update SQLCipher 4 databases (`exportLibrary.db`).

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

use crate::error::{Error, Result};

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;

enum Sqlite3 {}
enum Sqlite3Stmt {}

extern "C" {
    fn sqlite3_open(filename: *const c_char, db: *mut *mut Sqlite3) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: *mut c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        sql: *const c_char,
        nbyte: c_int,
        stmt: *mut *mut Sqlite3Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut Sqlite3Stmt,
        index: c_int,
        value: *const c_char,
        n: c_int,
        destructor: *const c_void,
    ) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut Sqlite3Stmt, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_null(stmt: *mut Sqlite3Stmt, index: c_int) -> c_int;
    fn sqlite3_step(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_text(stmt: *mut Sqlite3Stmt, col: c_int) -> *const u8;
    fn sqlite3_finalize(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_reset(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_clear_bindings(stmt: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_changes(db: *mut Sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    fn sqlite3_free(ptr: *mut c_void);
}

#[cfg(test)]
extern "C" {
    fn sqlite3_column_int64(stmt: *mut Sqlite3Stmt, col: c_int) -> i64;
}

pub struct Db {
    raw: *mut Sqlite3,
}

// The amalgamation is built SQLITE_THREADSAFE=1. Connections are not shared.
unsafe impl Send for Db {}

impl Drop for Db {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.raw);
        }
    }
}

impl Db {
    /// Opens a SQLCipher 4 database. `key` is the passphrase, not a raw key.
    pub fn open_sqlcipher4(path: &Path, key: &str) -> Result<Self> {
        let path = CString::new(path.to_string_lossy().as_ref())
            .map_err(|e| Error::Database(e.to_string()))?;
        let mut raw = ptr::null_mut();
        let rc = unsafe { sqlite3_open(path.as_ptr(), &mut raw) };
        if rc != SQLITE_OK {
            let message = errmsg(raw);
            if !raw.is_null() {
                unsafe { sqlite3_close(raw) };
            }
            return Err(Error::Database(message));
        }
        let db = Self { raw };
        // ChaCha20 is sqlite3mc's default; rekordbox uses SQLCipher 4.
        db.exec("PRAGMA cipher = 'sqlcipher';")?;
        db.exec("PRAGMA legacy = 4;")?;
        let key_sql = format!("PRAGMA key = '{}';", key.replace('\'', "''"));
        db.exec(&key_sql)?;
        db.exec("SELECT count(*) FROM sqlite_master;")?;
        Ok(db)
    }

    pub fn exec(&self, sql: &str) -> Result<()> {
        let sql = CString::new(sql).map_err(|e| Error::Database(e.to_string()))?;
        let mut err = ptr::null_mut();
        let rc = unsafe {
            sqlite3_exec(
                self.raw,
                sql.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut err,
            )
        };
        if rc != SQLITE_OK {
            let message = if err.is_null() {
                errmsg(self.raw)
            } else {
                let text = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
                unsafe { sqlite3_free(err.cast()) };
                text
            };
            return Err(Error::Database(message));
        }
        Ok(())
    }

    pub fn prepare(&self, sql: &str) -> Result<Stmt> {
        let sql = CString::new(sql).map_err(|e| Error::Database(e.to_string()))?;
        let mut raw = ptr::null_mut();
        let rc = unsafe {
            sqlite3_prepare_v2(self.raw, sql.as_ptr(), -1, &mut raw, ptr::null_mut())
        };
        if rc != SQLITE_OK {
            return Err(Error::Database(errmsg(self.raw)));
        }
        Ok(Stmt { raw, db: self.raw })
    }

    pub fn changes(&self) -> u32 {
        unsafe { sqlite3_changes(self.raw) as u32 }
    }
}

pub struct Stmt {
    raw: *mut Sqlite3Stmt,
    db: *mut Sqlite3,
}

impl Drop for Stmt {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.raw);
        }
    }
}

impl Stmt {
    pub fn reset(&self) -> Result<()> {
        check(self.db, unsafe { sqlite3_reset(self.raw) })?;
        check(self.db, unsafe { sqlite3_clear_bindings(self.raw) })?;
        Ok(())
    }

    pub fn bind_text(&self, index: c_int, value: &str) -> Result<()> {
        let value = CString::new(value).map_err(|e| Error::Database(e.to_string()))?;
        // SQLITE_TRANSIENT ((void(*)(void*))-1) copies the text before return.
        let transient = -1isize as *const c_void;
        check(
            self.db,
            unsafe { sqlite3_bind_text(self.raw, index, value.as_ptr(), -1, transient) },
        )
    }

    pub fn bind_i64(&self, index: c_int, value: i64) -> Result<()> {
        check(self.db, unsafe { sqlite3_bind_int64(self.raw, index, value) })
    }

    pub fn bind_null(&self, index: c_int) -> Result<()> {
        check(self.db, unsafe { sqlite3_bind_null(self.raw, index) })
    }

    pub fn step(&self) -> Result<Step> {
        match unsafe { sqlite3_step(self.raw) } {
            SQLITE_ROW => Ok(Step::Row),
            SQLITE_DONE => Ok(Step::Done),
            rc => Err(Error::Database(format!(
                "{} ({rc})",
                errmsg(self.db)
            ))),
        }
    }

    pub fn column_text(&self, col: c_int) -> String {
        let ptr = unsafe { sqlite3_column_text(self.raw, col) };
        if ptr.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(ptr.cast()) }
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(test)]
    pub fn column_i64(&self, col: c_int) -> i64 {
        unsafe { sqlite3_column_int64(self.raw, col) }
    }
}

pub enum Step {
    Row,
    Done,
}

fn errmsg(db: *mut Sqlite3) -> String {
    if db.is_null() {
        return "sqlite open failed".into();
    }
    let ptr = unsafe { sqlite3_errmsg(db) };
    if ptr.is_null() {
        return "unknown sqlite error".into();
    }
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

fn check(db: *mut Sqlite3, rc: c_int) -> Result<()> {
    if rc == SQLITE_OK {
        Ok(())
    } else {
        Err(Error::Database(format!("{} ({rc})", errmsg(db))))
    }
}
