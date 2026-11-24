#![forbid(unsafe_code)]

use rusqlite::Connection;
use thiserror::Error;

/// Embedding vector dimensionality for `multilingual-e5-small`.
/// Treated as a migration-versioned constant (Phase 01).
pub const EMBEDDING_DIM: usize = 384;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Manages device-local tables (`activity_sample`, `energy_report`, `nudge_event`,
/// `llm_call`, `response_cache`, `ml_state`, `fragment`, `fragment_fts`).
pub struct DeviceLocalStore {
    conn: Connection,
}

impl DeviceLocalStore {
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(Self { conn })
    }

    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts5_and_energy_self_report_check_work_in_bundled_sqlite() {
        let store = DeviceLocalStore::open_in_memory().expect("open store");
        store
            .connection()
            .execute_batch(
                "CREATE VIRTUAL TABLE fragment_fts USING fts5(text, tokenize='unicode61 remove_diacritics 0');
                 INSERT INTO fragment_fts(text) VALUES ('Báo cáo quý ba'), ('Bao cao quy ba');
                 CREATE TABLE energy_report (
                     id TEXT PRIMARY KEY,
                     level INTEGER NOT NULL,
                     source TEXT NOT NULL CHECK (source = 'self_report')
                 );",
            )
            .expect("create tables");

        let count: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM fragment_fts WHERE fragment_fts MATCH 'Báo'",
                [],
                |row| row.get(0),
            )
            .expect("fts5 vietnamese diacritics query");
        assert_eq!(count, 1);

        let err = store
            .connection()
            .execute(
                "INSERT INTO energy_report(id, level, source) VALUES ('1', 2, 'inferred')",
                [],
            )
            .expect_err("non-self_report energy entry must fail CHECK constraint");
        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }
}
