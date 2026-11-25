use thiserror::Error;

/// Errors returned by native Win32/WinRT host operations.
#[derive(Debug, Error)]
pub enum Win32Error {
    #[error("Win32 API call `{op}` failed: {message}")]
    Api { op: &'static str, message: String },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("internal channel closed")]
    ChannelClosed,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl Win32Error {
    #[must_use]
    pub fn api(op: &'static str, message: impl Into<String>) -> Self {
        Self::Api {
            op,
            message: message.into(),
        }
    }
}
