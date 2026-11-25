#![forbid(unsafe_code)]

pub mod conn;
pub mod engine;
pub mod mutation;

#[cfg(test)]
mod convergence_tests;

pub use conn::SyncWriteConn;
pub use engine::OpLog;
pub use mutation::{ApplyOutcome, EntityType, OpKind, OpLogError, OpRecord, SyncMutation};
