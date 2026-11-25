use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ef_oplog::{ApplyOutcome, OpLog, OpRecord, SyncMutation, SyncWriteConn};
use ef_win32::{DbKey, register_sqlite_vec};
use rusqlite::{Connection, OpenFlags};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::StorageError;
use crate::migrations::run_migrations;

const WRITER_QUEUE_CAPACITY: usize = 256;
const READ_POOL_SIZE: usize = 3;

#[must_use]
pub fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

enum WriterJob {
    ApplySync {
        mutation: SyncMutation,
        now_ms: i64,
        reply: oneshot::Sender<Result<Vec<OpRecord>, StorageError>>,
    },
    ApplyRemote {
        op: OpRecord,
        now_ms: i64,
        reply: oneshot::Sender<Result<ApplyOutcome, StorageError>>,
    },
    ListUnsynced {
        reply: oneshot::Sender<Result<Vec<OpRecord>, StorageError>>,
    },
    Vacuum {
        reply: oneshot::Sender<Result<(), StorageError>>,
    },
    WriteLocal {
        task: Box<dyn FnOnce(&mut Connection) + Send>,
    },
    ReadOnly {
        task: Box<dyn FnOnce(&Connection) + Send>,
    },
}

struct ReadPool {
    conns: Arc<Mutex<Vec<Connection>>>,
    semaphore: Arc<Semaphore>,
}

struct PooledConnGuard {
    conn: Option<Connection>,
    pool: Arc<Mutex<Vec<Connection>>>,
}

impl Drop for PooledConnGuard {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take()
            && let Ok(mut guard) = self.pool.lock()
        {
            guard.push(conn);
        }
    }
}

impl ReadPool {
    fn open(path: &Path, key: Option<&DbKey>, size: usize) -> Result<Self, StorageError> {
        let mut conns = Vec::with_capacity(size);
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        for _ in 0..size {
            let conn = Connection::open_with_flags(path, flags)?;
            if let Some(k) = key {
                apply_sqlcipher_key(&conn, k)?;
            }
            conn.execute_batch(
                "PRAGMA query_only = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA foreign_keys = ON;",
            )?;
            conns.push(conn);
        }
        Ok(Self {
            conns: Arc::new(Mutex::new(conns)),
            semaphore: Arc::new(Semaphore::new(size)),
        })
    }

    async fn read<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
    {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        let pool = Arc::clone(&self.conns);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let conn = {
                let mut guard = pool.lock().map_err(|_| StorageError::WriterClosed)?;
                guard.pop().ok_or(StorageError::WriterClosed)?
            };
            let conn_guard = PooledConnGuard {
                conn: Some(conn),
                pool,
            };
            let conn_ref = conn_guard.conn.as_ref().ok_or(StorageError::WriterClosed)?;
            f(conn_ref)
        })
        .await
        .map_err(|_| StorageError::WriterClosed)?
    }
}

fn apply_sqlcipher_key(conn: &Connection, key: &DbKey) -> Result<(), StorageError> {
    let pragma = format!("PRAGMA key = \"{}\";", key.to_sqlcipher_pragma_hex());
    conn.execute_batch(&pragma)?;
    Ok(())
}

fn configure_writer_connection(
    conn: &mut Connection,
    key: Option<&DbKey>,
    wal: bool,
    now_ms: i64,
) -> Result<(), StorageError> {
    if let Some(k) = key {
        apply_sqlcipher_key(conn, k)?;
    }
    if wal {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
    } else {
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
    }
    run_migrations(conn, now_ms)?;
    Ok(())
}

/// Thread-safe, cloneable handle to the encrypted SQLite/SQLCipher persistence layer.
///
/// All writes serialize on a dedicated OS thread (`ef-sqlite-writer`), while file-backed
/// reads dispatch through a bounded read-only connection pool without ever holding a
/// `rusqlite::Connection` lock across a `tokio` `.await`.
#[derive(Clone)]
pub struct Database {
    writer_tx: mpsc::Sender<WriterJob>,
    read_pool: Option<Arc<ReadPool>>,
}

impl Database {
    /// Opens (or creates) an encrypted SQLCipher database at `path` using `key`,
    /// runs pending migrations, restores `Hlc`, and starts the writer thread + read pool.
    pub fn open_encrypted(path: impl AsRef<Path>, key: &DbKey) -> Result<Self, StorageError> {
        Self::open_file_internal(path.as_ref(), Some(key), current_unix_ms())
    }

    /// Opens an encrypted file-backed database with an explicit clock seed (primarily for tests).
    pub fn open_encrypted_at(
        path: impl AsRef<Path>,
        key: &DbKey,
        now_ms: i64,
    ) -> Result<Self, StorageError> {
        Self::open_file_internal(path.as_ref(), Some(key), now_ms)
    }

    fn open_file_internal(
        path: &Path,
        key: Option<&DbKey>,
        now_ms: i64,
    ) -> Result<Self, StorageError> {
        register_sqlite_vec().map_err(StorageError::SqliteVecInit)?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::OpLog(ef_oplog::OpLogError::InvalidValue(format!(
                    "failed to create database directory: {e}"
                )))
            })?;
        }

        let mut conn = Connection::open(path)?;
        configure_writer_connection(&mut conn, key, true, now_ms)?;
        let mut sync_conn = SyncWriteConn::from_connection(conn);
        let hlc = OpLog::init_hlc(&mut sync_conn, now_ms)?;

        let read_pool = Arc::new(ReadPool::open(path, key, READ_POOL_SIZE)?);
        let writer_tx = spawn_writer_thread(sync_conn, hlc)?;
        Ok(Self {
            writer_tx,
            read_pool: Some(read_pool),
        })
    }

    /// Opens an in-memory SQLCipher database for unit tests and startup smoke checks.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::open_in_memory_at(current_unix_ms())
    }

    /// Opens an in-memory database with an explicit initial clock timestamp.
    pub fn open_in_memory_at(now_ms: i64) -> Result<Self, StorageError> {
        register_sqlite_vec().map_err(StorageError::SqliteVecInit)?;
        let mut conn = Connection::open_in_memory()?;
        configure_writer_connection(&mut conn, None, false, now_ms)?;
        let mut sync_conn = SyncWriteConn::from_connection(conn);
        let hlc = OpLog::init_hlc(&mut sync_conn, now_ms)?;
        let writer_tx = spawn_writer_thread(sync_conn, hlc)?;
        Ok(Self {
            writer_tx,
            read_pool: None,
        })
    }

    /// Applies a local syncable mutation exclusively through `ef_oplog::OpLog::apply`.
    pub async fn apply_sync(&self, mutation: SyncMutation) -> Result<Vec<OpRecord>, StorageError> {
        self.apply_sync_at(mutation, current_unix_ms()).await
    }

    /// Applies a local syncable mutation with an explicit timestamp.
    pub async fn apply_sync_at(
        &self,
        mutation: SyncMutation,
        now_ms: i64,
    ) -> Result<Vec<OpRecord>, StorageError> {
        let (tx, rx) = oneshot::channel();
        self.writer_tx
            .send(WriterJob::ApplySync {
                mutation,
                now_ms,
                reply: tx,
            })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }

    /// Applies a remote/replayed operation idempotently through `ef_oplog::OpLog::apply_remote`.
    pub async fn apply_remote_op(&self, op: OpRecord) -> Result<ApplyOutcome, StorageError> {
        self.apply_remote_op_at(op, current_unix_ms()).await
    }

    /// Applies a remote/replayed operation with an explicit local clock timestamp.
    pub async fn apply_remote_op_at(
        &self,
        op: OpRecord,
        now_ms: i64,
    ) -> Result<ApplyOutcome, StorageError> {
        let (tx, rx) = oneshot::channel();
        self.writer_tx
            .send(WriterJob::ApplyRemote {
                op,
                now_ms,
                reply: tx,
            })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }

    /// Lists unsynced op-log records in sequence order.
    pub async fn list_unsynced_ops(&self) -> Result<Vec<OpRecord>, StorageError> {
        let (tx, rx) = oneshot::channel();
        self.writer_tx
            .send(WriterJob::ListUnsynced { reply: tx })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }

    /// Rebuilds the database file cleanly via `VACUUM` on the dedicated writer thread.
    pub async fn vacuum(&self) -> Result<(), StorageError> {
        let (tx, rx) = oneshot::channel();
        self.writer_tx
            .send(WriterJob::Vacuum { reply: tx })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }

    /// Executes a device-local write closure on the writer thread under `SyncWriteConn`'s
    /// SQLite authorizer (which blocks writes to any syncable table).
    pub async fn write_local<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let task = Box::new(move |conn: &mut Connection| {
            let _ = tx.send(f(conn));
        });
        self.writer_tx
            .send(WriterJob::WriteLocal { task })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }

    /// Executes a read-only query against the read pool (file-backed) or writer thread
    /// authorizer-enforced read-only view (in-memory).
    pub async fn read<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
    {
        if let Some(pool) = &self.read_pool {
            return pool.read(f).await;
        }
        let (tx, rx) = oneshot::channel();
        let task = Box::new(move |conn: &Connection| {
            let _ = tx.send(f(conn));
        });
        self.writer_tx
            .send(WriterJob::ReadOnly { task })
            .await
            .map_err(|_| StorageError::WriterClosed)?;
        rx.await.map_err(|_| StorageError::WriterClosed)?
    }
}

fn spawn_writer_thread(
    mut sync_conn: SyncWriteConn,
    mut hlc: ef_core::Hlc,
) -> Result<mpsc::Sender<WriterJob>, StorageError> {
    let (tx, mut rx) = mpsc::channel::<WriterJob>(WRITER_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("ef-sqlite-writer".to_string())
        .spawn(move || {
            while let Some(job) = rx.blocking_recv() {
                match job {
                    WriterJob::ApplySync {
                        mutation,
                        now_ms,
                        reply,
                    } => {
                        let res = OpLog::new(&mut sync_conn, &mut hlc)
                            .apply(mutation, now_ms)
                            .map_err(StorageError::from);
                        let _ = reply.send(res);
                    }
                    WriterJob::ApplyRemote { op, now_ms, reply } => {
                        let res = OpLog::new(&mut sync_conn, &mut hlc)
                            .apply_remote(&op, now_ms)
                            .map_err(StorageError::from);
                        let _ = reply.send(res);
                    }
                    WriterJob::ListUnsynced { reply } => {
                        let res =
                            OpLog::list_unsynced_ops(&mut sync_conn).map_err(StorageError::from);
                        let _ = reply.send(res);
                    }
                    WriterJob::Vacuum { reply } => {
                        let res = sync_conn.vacuum().map_err(StorageError::from);
                        let _ = reply.send(res);
                    }
                    WriterJob::WriteLocal { task } => {
                        let _ = sync_conn.with_device_local_conn(|conn| -> Result<(), ()> {
                            task(conn);
                            Ok(())
                        });
                    }
                    WriterJob::ReadOnly { task } => {
                        let _ = sync_conn.with_readonly(|conn| -> Result<(), ()> {
                            task(conn);
                            Ok(())
                        });
                    }
                }
            }
        })
        .map_err(|e| {
            StorageError::OpLog(ef_oplog::OpLogError::InvalidValue(format!(
                "failed to spawn sqlite writer thread: {e}"
            )))
        })?;
    Ok(tx)
}
