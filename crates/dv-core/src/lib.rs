//! DanmakuVault 领域层：模型、ID、端口 trait、错误。
//!
//! 这里不做任何 IO，也不认识 HTTP 与 SQLite —— 便于单测与替换实现。

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

/// 当前 unix 时间戳（秒）。
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// 当前 unix 时间戳（毫秒）。
pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
