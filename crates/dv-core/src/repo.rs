//! 存储端口（port）。
//!
//! 同步实现：SQLite 的写入在 `dv-store` 里由一把锁串行化，
//! 异步侧需要批量写时用 `spawn_blocking` 包一层。

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

/// `list` 命令用的一行摘要。
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

/// `show` 用的单个分 P 统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageStats {
    pub danmaku: u64,
    pub missing: u64,
    pub segments: u32,
}

pub trait Repository {
    /// 写入/更新稿件与分 P 元信息。
    fn upsert_video(&self, meta: &VideoMeta) -> Result<()>;

    /// 批量写入弹幕，`(cid, dmid)` 冲突时只刷新 `last_seen_at` 与墓碑。
    fn upsert_danmaku(&self, cid: Cid, items: &[Danmaku], seen_at: i64) -> Result<UpsertStats>;

    fn segment_state(&self, cid: Cid, index: u32) -> Result<Option<SegmentState>>;

    fn put_segment_state(&self, cid: Cid, state: &SegmentState) -> Result<()>;

    /// 把本轮全量同步中没再出现的弹幕标为墓碑，返回受影响行数。
    fn mark_missing(&self, cid: Cid, before: i64) -> Result<u64>;

    /// 读取某分 P 的弹幕，按时间排序。
    ///
    /// `include_missing = false` 时排除墓碑（服务端已消失的弹幕）。
    fn load_danmaku(&self, cid: Cid, include_missing: bool) -> Result<Vec<Danmaku>>;

    fn set_page_synced(&self, cid: Cid, at: i64) -> Result<()>;

    /// 该分 P 上次「完整同步成功」的时间；从未完整同步过返回 `None`。
    ///
    /// 用它区分两种模式：`None` 走断点续传（跳过已有段），`Some` 走刷新（全量重抓 + 比对）。
    fn page_synced_at(&self, cid: Cid) -> Result<Option<i64>>;

    fn page_stats(&self, cid: Cid) -> Result<PageStats>;

    fn video_by_bvid(&self, bvid: &Bvid) -> Result<Option<VideoMeta>>;

    fn video_by_aid(&self, aid: Aid) -> Result<Option<VideoMeta>>;

    fn list_videos(&self) -> Result<Vec<VideoSummary>>;
}
