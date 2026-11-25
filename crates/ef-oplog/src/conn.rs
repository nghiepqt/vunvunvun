use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

const SYNC_PROTECTED_TABLES: &[&str] = &[
    "task",
    "note",
    "inbox_item",
    "user_profile",
    "transcript",
    "project",
    "op_log",
    "field_clock",
    "hlc_state",
    "schema_version",
];

fn is_sync_protected(table_name: &str) -> bool {
    SYNC_PROTECTED_TABLES
        .iter()
        .any(|protected| table_name.eq_ignore_ascii_case(protected))
}

struct AuthorizerGuard<'a> {
    conn: &'a mut Connection,
}

impl Drop for AuthorizerGuard<'_> {
    fn drop(&mut self) {
        self.conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

/// Exclusive write handle to syncable SQLite tables.
///
/// The inner `rusqlite::Connection` field is private to `ef-oplog`, enforcing at compile time
/// that no external crate can issue raw `INSERT`/`UPDATE`/`DELETE` statements against synced
/// tables (`CONTEXT.md` Section 6). Device-local writes must pass through
/// [`SyncWriteConn::with_device_local_conn`], which attaches a runtime SQLite authorizer
/// blocking writes to sync-protected tables.
pub struct SyncWriteConn {
    conn: Connection,
}

impl SyncWriteConn {
    #[must_use]
    pub fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Runs `VACUUM` on the underlying connection.
    ///
    /// `VACUUM` internally attaches a temporary database and copies all tables (including
    /// syncable tables), so it is exposed as a fixed maintenance method rather than running
    /// under `with_device_local_conn`'s authorizer.
    pub fn vacuum(&mut self) -> Result<(), rusqlite::Error> {
        self.conn.execute_batch("VACUUM;")
    }

    /// Runs a device-local write closure while a SQLite authorizer blocks any DML/DDL on
    /// syncable tables (`task`, `note`, `inbox_item`, `user_profile`, `transcript`, `project`)
    /// and op-log metadata tables.
    pub fn with_device_local_conn<T, E, F>(&mut self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E>,
    {
        self.conn
            .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
                AuthAction::Insert { table_name }
                | AuthAction::Update { table_name, .. }
                | AuthAction::Delete { table_name }
                | AuthAction::DropTable { table_name }
                | AuthAction::CreateTrigger { table_name, .. }
                | AuthAction::DropTrigger { table_name, .. }
                | AuthAction::AlterTable { table_name, .. }
                    if is_sync_protected(table_name) =>
                {
                    Authorization::Deny
                }
                AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
                _ => Authorization::Allow,
            }));
        let guard = AuthorizerGuard {
            conn: &mut self.conn,
        };
        f(guard.conn)
    }

    /// Runs a read-only closure while a SQLite authorizer denies all DML and DDL statements.
    pub fn with_readonly<T, E, F>(&mut self, f: F) -> Result<T, E>
    where
        F: FnOnce(&Connection) -> Result<T, E>,
    {
        self.conn
            .authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
                AuthAction::Select
                | AuthAction::Read { .. }
                | AuthAction::Function { .. }
                | AuthAction::Pragma { .. }
                | AuthAction::Transaction { .. }
                | AuthAction::Savepoint { .. }
                | AuthAction::Recursive => Authorization::Allow,
                _ => Authorization::Deny,
            }));
        let guard = AuthorizerGuard {
            conn: &mut self.conn,
        };
        f(guard.conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_local_authorizer_blocks_synced_tables_and_allows_local_tables() {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch(
            "CREATE TABLE task (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             CREATE TABLE inbox_item (id TEXT PRIMARY KEY, raw_text TEXT NOT NULL);
             CREATE TABLE op_log (seq INTEGER PRIMARY KEY, op_id TEXT NOT NULL);
             CREATE TABLE activity_sample (id INTEGER PRIMARY KEY, process_name TEXT NOT NULL);",
        )
        .expect("create test tables");

        let mut write_conn = SyncWriteConn::from_connection(conn);
        write_conn
            .with_device_local_conn(|c| -> Result<(), rusqlite::Error> {
                c.execute(
                    "INSERT INTO activity_sample(process_name) VALUES ('vun.exe')",
                    [],
                )?;
                for sql in [
                    "INSERT INTO task(id, title) VALUES ('1', 'bypass')",
                    "UPDATE task SET title = 'bypass'",
                    "DELETE FROM task",
                    "INSERT INTO inbox_item(id, raw_text) VALUES ('1', 'bypass')",
                    "INSERT INTO op_log(op_id) VALUES ('1')",
                ] {
                    let err = c
                        .execute(sql, [])
                        .expect_err("sync table DML must be denied");
                    assert!(
                        matches!(err, rusqlite::Error::SqliteFailure(_, _)),
                        "sql={sql}"
                    );
                }
                Ok(())
            })
            .expect("device local block");

        write_conn
            .with_readonly(|c| -> Result<(), rusqlite::Error> {
                let count: i64 =
                    c.query_row("SELECT count(*) FROM activity_sample", [], |r| r.get(0))?;
                assert_eq!(count, 1);
                let err = c
                    .execute(
                        "INSERT INTO activity_sample(process_name) VALUES ('bad')",
                        [],
                    )
                    .expect_err("readonly must block insert");
                assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
                Ok(())
            })
            .expect("readonly block");
    }
}
