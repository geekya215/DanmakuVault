//! Data source port.
//!
//! The concrete implementation lives in `dv-bili`; tests use an in-memory double.

use async_trait::async_trait;

use crate::error::Result;
use crate::id::{Cid, VideoRef};
use crate::model::{PageMeta, Segment, SegmentReq, VideoMeta};

#[async_trait]
pub trait DanmakuSource: Send + Sync {
    /// Fetch video metadata and all its parts.
    async fn video(&self, video: &VideoRef) -> Result<VideoMeta>;

    /// Fetch one 6-minute segment.
    async fn segment(&self, req: &SegmentReq) -> Result<Segment>;

    /// Fetch the whole XML bundle (compatibility export / fallback when the protobuf path
    /// misbehaves).
    async fn legacy_xml(&self, cid: Cid) -> Result<Vec<u8>>;

    /// Total danmaku count declared by the server; return `Ok(None)` if it is not available.
    async fn declared_count(&self, _cid: Cid) -> Result<Option<u32>> {
        Ok(None)
    }

    /// Fill in part metadata only (things like `declared_count`); the default returns them
    /// unchanged.
    fn enrich_pages(&self, pages: &mut [PageMeta]) {
        let _ = pages;
    }
}
