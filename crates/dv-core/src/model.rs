//! 领域模型。

use std::fmt;

use crate::id::{Aid, Bvid, Cid, Dmid};

// ---------------------------------------------------------------------------
// 视频 / 分 P
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub mid: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PageMeta {
    pub cid: Cid,
    /// 1-based 分 P 序号。
    pub page: u32,
    pub part: String,
    pub duration_ms: u64,
    /// 服务端声明的弹幕总数（来自 `dm/web/view`），可能缺失。
    pub declared_count: Option<u32>,
}

impl PageMeta {
    /// 6 分钟一段，向下取整至少 1 段。
    pub fn nominal_segments(&self) -> u32 {
        let seg = self.duration_ms.div_ceil(360_000).max(1);
        u32::try_from(seg).unwrap_or(u32::MAX)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VideoMeta {
    pub aid: Aid,
    pub bvid: Option<Bvid>,
    pub title: String,
    pub owner: Option<Owner>,
    pub duration_ms: u64,
    pub pubdate: Option<i64>,
    pub cover_url: Option<String>,
    pub pages: Vec<PageMeta>,
}

impl VideoMeta {
    pub fn page(&self, cid: Cid) -> Option<&PageMeta> {
        self.pages.iter().find(|p| p.cid == cid)
    }

    /// 目录名：优先 BV 号，退化成 `av{aid}`。
    pub fn folder_name(&self) -> String {
        match &self.bvid {
            Some(b) => b.to_string(),
            None => format!("av{}", self.aid.get()),
        }
    }
}

// ---------------------------------------------------------------------------
// 弹幕
// ---------------------------------------------------------------------------

/// 一条弹幕。
///
/// `progress_ms` 用整数毫秒而不是秒的浮点：排序稳定、比较无误差、往返导出可控。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Danmaku {
    pub dmid: Dmid,
    /// 相对视频起点的毫秒偏移。
    pub progress_ms: i64,
    /// 1/2/3 滚动，4 底部，5 顶部，6 逆向，7 高级，8 代码，9 BAS。
    pub mode: u8,
    pub fontsize: u8,
    pub color: u32,
    /// 发送者 UID 的哈希（不是明文 UID）。
    pub mid_hash: String,
    pub content: String,
    /// 发送时间，unix 秒。
    pub ctime: i64,
    pub weight: u32,
    /// 0/1 普通池，2 字幕池。
    pub pool: u8,
    pub attr: u32,
    /// 原始 `p` 属性里第 9 个及之后的字段，保证再导出不丢信息。
    pub extras: Option<String>,
}

impl Danmaku {
    /// 导出时的稳定排序键。
    pub fn sort_key(&self) -> (i64, u64) {
        (self.progress_ms, self.dmid.get())
    }

    /// 秒的表示，保留 5 位小数（与 B 站 XML 一致）。
    pub fn progress_secs(&self) -> String {
        format!("{:.5}", self.progress_ms as f64 / 1000.0)
    }
}

impl PartialOrd for Danmaku {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Danmaku {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

// ---------------------------------------------------------------------------
// 分段
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentReq {
    pub cid: Cid,
    pub aid: Aid,
    /// 1-based 分段号，一段 6 分钟。
    pub index: u32,
}

/// 一次分段抓取的结果：原始字节（用于校验/缓存）+ 解析后的条目。
#[derive(Clone, Debug)]
pub struct Segment {
    pub index: u32,
    pub raw: Vec<u8>,
    pub sha256: [u8; 32],
    pub items: Vec<Danmaku>,
}

impl Segment {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentSource {
    /// `x/v2/dm/web/seg.so`（protobuf）
    Protobuf,
    /// `x/v1/dm/list.so`（deflate + xml）
    LegacyXml,
}

impl SegmentSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SegmentSource::Protobuf => "pb",
            SegmentSource::LegacyXml => "xml",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pb" => Some(SegmentSource::Protobuf),
            "xml" => Some(SegmentSource::LegacyXml),
            _ => None,
        }
    }
}

impl fmt::Display for SegmentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 分段在库里的状态，断点续传的依据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentState {
    pub index: u32,
    pub sha256: [u8; 32],
    pub byte_len: u64,
    pub n_items: u32,
    pub source: SegmentSource,
    pub fetched_at: i64,
}
