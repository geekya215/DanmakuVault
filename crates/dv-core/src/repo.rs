//! Storage port.
//!
//! Synchronous implementation: in `dv-store` the SQLite writes are serialized by a single lock,
//! and the async side wraps batch writes in `spawn_blocking`.

use crate::error::Result;
use crate::id::{Aid, Bvid, Cid};
use crate::model::{Danmaku, SegmentState, VideoMeta};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct UpsertStats {
    pub inserted: u64,
    pub updated: u64,
}

impl UpsertStats {
    pub fn total(&self) -> u64 {
        self.inserted + self.updated
    }

    pub fn merge(&mut self, other: UpsertStats) {
        self.inserted += other.inserted;
        self.updated += other.updated;
    }
}

/// One-row summary used by the `list` command.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct VideoSummary {
    pub aid: Aid,
    pub bvid: Option<Bvid>,
    pub title: String,
    pub page_count: u32,
    pub danmaku_count: u64,
    pub missing_count: u64,
    pub last_synced_at: Option<i64>,
}

/// Per-part statistics used by `show`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageStats {
    pub danmaku: u64,
    pub missing: u64,
    pub segments: u32,
}

/// Result of `gc`: how many rows were deleted and which parts were affected.
///
/// Every affected part has to be written back to disk, otherwise `verify` immediately reports
/// "file does not match database".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub deleted: u64,
    pub affected: Vec<Cid>,
}

pub trait Repository {
    /// Insert/update video and part metadata.
    fn upsert_video(&self, meta: &VideoMeta) -> Result<()>;

    /// Batch-insert danmaku; on a `(cid, dmid)` conflict only `last_seen_at` and the tombstone are
    /// refreshed.
    fn upsert_danmaku(&self, cid: Cid, items: &[Danmaku], seen_at: i64) -> Result<UpsertStats>;

    fn segment_state(&self, cid: Cid, index: u32) -> Result<Option<SegmentState>>;

    fn put_segment_state(&self, cid: Cid, state: &SegmentState) -> Result<()>;

    /// Mark as tombstone every danmaku that no longer appeared in this full sync, returning the
    /// number of affected rows.
    fn mark_missing(&self, cid: Cid, before: i64) -> Result<u64>;

    /// Number of rows whose tombstone time is earlier than `before` (the dry run for `gc`).
    fn count_missing_before(&self, before: i64) -> Result<u64>;

    /// Physically delete danmaku whose tombstone time is earlier than `before`.
    fn purge_missing_before(&self, before: i64) -> Result<PurgeOutcome>;

    /// On-disk location of a part: `(folder name, part number)`.
    fn page_location(&self, cid: Cid) -> Result<Option<(String, u32)>>;

    /// Load the danmaku of one part, sorted by time.
    ///
    /// With `include_missing = false` the tombstones (danmaku that have disappeared from the
    /// server) are excluded.
    fn load_danmaku(&self, cid: Cid, include_missing: bool) -> Result<Vec<Danmaku>>;

    fn set_page_synced(&self, cid: Cid, at: i64) -> Result<()>;

    /// When this part was last "fully synced successfully"; `None` if it has never been fully
    /// synced.
    ///
    /// This is what distinguishes the two modes: `None` goes through resume (skipping segments
    /// already present), `Some` goes through refresh (refetch everything and compare).
    fn page_synced_at(&self, cid: Cid) -> Result<Option<i64>>;

    fn page_stats(&self, cid: Cid) -> Result<PageStats>;

    fn video_by_bvid(&self, bvid: &Bvid) -> Result<Option<VideoMeta>>;

    fn video_by_aid(&self, aid: Aid) -> Result<Option<VideoMeta>>;

    fn list_videos(&self) -> Result<Vec<VideoSummary>>;
}
