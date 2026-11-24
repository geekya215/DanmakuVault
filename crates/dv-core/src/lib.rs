//! DanmakuVault domain layer: models, IDs, port traits, errors.
//!
//! Nothing here does any IO, and nothing here knows about HTTP or SQLite — that keeps unit testing easy and lets implementations be swapped out.

pub mod error;
pub mod hash;
pub mod id;
pub mod model;
pub mod repo;
pub mod source;

pub use error::{Result, VaultError};
pub use hash::{hex_lower, sha256, sha256_hex};
pub use id::{Aid, Bvid, Cid, Dmid, VideoRef};
pub use model::{
    Danmaku, Owner, PageMeta, Segment, SegmentReq, SegmentSource, SegmentState, VideoMeta,
};
pub use repo::{PageStats, PurgeOutcome, Repository, UpsertStats, VideoSummary};
pub use source::DanmakuSource;

use std::time::{SystemTime, UNIX_EPOCH};

/// Current unix timestamp (seconds).
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Current unix timestamp (milliseconds).
pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
