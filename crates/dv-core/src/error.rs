//! Unified error type.
//!
//! It deliberately does not depend on `reqwest` / `rusqlite`: the lower layers convert their own
//! errors into the variants here, so `dv-core` keeps zero IO dependencies and test doubles do not
//! have to pull in an HTTP stack.

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("network error: {0}")]
    Network(String),

    /// Bilibili business error (`code != 0`). Whether it is `retryable` is decided by the caller
    /// from the error code.
    #[error("API error {code}: {msg}")]
    Api { code: i32, msg: String },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Db(String),

    #[error("login required: cookie missing or expired")]
    Auth,

    #[error("rate limited or blocked by anti-bot (-412)")]
    RateLimited,

    #[error("invalid ID: {0}")]
    InvalidId(String),

    #[error("corrupt data: {0}")]
    Corrupt(String),

    #[error("{0}")]
    Other(String),
}

impl VaultError {
    /// Whether it is worth retrying: network blips, 5xx, rate limiting.
    pub fn is_retryable(&self) -> bool {
        match self {
            VaultError::Network(_) | VaultError::RateLimited => true,
            VaultError::Api { code, .. } => matches!(code, -412 | -509 | -500 | -503 | -504),
            _ => false,
        }
    }

    /// Whether this falls into "this video/part is beyond saving, skip it and stop retrying".
    pub fn is_permanent_skip(&self) -> bool {
        match self {
            VaultError::Api { code, .. } => matches!(code, -404 | 62002 | 62004 | 62012),
            _ => false,
        }
    }
}

impl From<std::fmt::Error> for VaultError {
    fn from(value: fmt::Error) -> Self {
        VaultError::Other(value.to_string())
    }
}

pub type Result<T, E = VaultError> = std::result::Result<T, E>;
