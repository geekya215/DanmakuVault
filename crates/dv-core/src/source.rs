//! 数据源端口（port）。
//!
//! 具体实现在 `dv-bili`；测试里用内存替身。

use async_trait::async_trait;

use crate::error::Result;
use crate::id::{Cid, VideoRef};
use crate::model::{PageMeta, Segment, SegmentReq, VideoMeta};

#[async_trait]
pub trait DanmakuSource: Send + Sync {
    /// 拉取稿件元信息与全部分 P。
    async fn video(&self, video: &VideoRef) -> Result<VideoMeta>;

    /// 拉取一个 6 分钟分段。
    async fn segment(&self, req: &SegmentReq) -> Result<Segment>;

    /// 拉取整包 XML（兼容导出 / protobuf 通道异常时的兜底）。
    async fn legacy_xml(&self, cid: Cid) -> Result<Vec<u8>>;

    /// 服务端声明的弹幕总数，用不上就返回 `Ok(None)`。
    async fn declared_count(&self, _cid: Cid) -> Result<Option<u32>> {
        Ok(None)
    }

    /// 只保留分 P 元信息（`declared_count` 之类），默认原样返回。
    fn enrich_pages(&self, pages: &mut [PageMeta]) {
        let _ = pages;
    }
}
