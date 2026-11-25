use std::ffi::{c_char, c_int};
use std::sync::OnceLock;

use rusqlite::ffi::{SQLITE_OK, sqlite3, sqlite3_api_routines, sqlite3_auto_extension};
use sqlite_vec::sqlite3_vec_init;

type AutoExtEntryPoint =
    unsafe extern "C" fn(*mut sqlite3, *mut *mut c_char, *const sqlite3_api_routines) -> c_int;

static SQLITE_VEC_INIT: OnceLock<Result<(), i32>> = OnceLock::new();

/// Registers the statically linked `sqlite-vec` extension for every future SQLite/SQLCipher
/// connection in this process.
///
/// Idempotent and thread-safe; safe to call before opening any connection.
pub fn register_sqlite_vec() -> Result<(), i32> {
    *SQLITE_VEC_INIT.get_or_init(|| {
        // SAFETY: `sqlite3_vec_init` is compiled statically with `-DSQLITE_CORE` by the
        // `sqlite-vec` crate and has the standard SQLite extension entry-point ABI:
        // `int sqlite3_vec_init(sqlite3 *db, char **pzErrMsg, const sqlite3_api_routines *pApi)`.
        // `sqlite3_auto_extension` registers this function pointer process-wide once.
        let rc = unsafe {
            let init_fn: AutoExtEntryPoint = std::mem::transmute(sqlite3_vec_init as *const ());
            sqlite3_auto_extension(Some(init_fn))
        };
        if rc == SQLITE_OK { Ok(()) } else { Err(rc) }
    })
}
