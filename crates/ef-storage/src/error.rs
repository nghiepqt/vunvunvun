use thiserror::Error;

pub const EMBEDDING_DIM: usize = 384;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("op-log error: {0}")]
    OpLog(#[from] ef_oplog::OpLogError),
    #[error("domain enum parse error: {0}")]
    DomainEnum(#[from] ef_core::ParseDomainEnumError),
    #[error("position error: {0}")]
    Position(#[from] ef_core::PositionError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("credential key store error: {0}")]
    KeyStore(#[from] ef_win32::KeyStoreError),
    #[error("failed to register static sqlite-vec extension (sqlite error code {0})")]
    SqliteVecInit(i32),
    #[error("invalid embedding dimensionality: expected {expected}, got {actual}")]
    InvalidEmbeddingDim { expected: usize, actual: usize },
    #[error("database writer thread closed unexpectedly")]
    WriterClosed,
    #[error("invalid UUID in database row: {0}")]
    InvalidUuid(String),
    #[error("record not found: {0}")]
    NotFound(String),
}
