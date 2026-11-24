//! Domain models.

use std::fmt;

use crate::id::{Aid, Bvid, Cid, Dmid};

// ---------------------------------------------------------------------------
// Video / parts
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub mid: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PageMeta {
    pub cid: Cid,
    /// 1-based part number.
    pub page: u32,
    pub part: String,
    pub duration_ms: u64,
    /// Total danmaku count declared by the server (from `dm/web/view`); may be absent.
    pub declared_count: Option<u32>,
}

impl PageMeta {
    /// One segment per six minutes, rounded up, never less than one.
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

    /// Folder name: prefer the BV id, fall back to `av{aid}`.
    pub fn folder_name(&self) -> String {
        match &self.bvid {
            Some(b) => b.to_string(),
            None => format!("av{}", self.aid.get()),
        }
    }
}

// ---------------------------------------------------------------------------
// Danmaku
// ---------------------------------------------------------------------------

/// A single danmaku.
///
/// `progress_ms` is an integer number of milliseconds rather than a float number of seconds:
/// sorting is stable, comparisons are exact, and round-trip export stays controllable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Danmaku {
    pub dmid: Dmid,
    /// Millisecond offset from the start of the video.
    pub progress_ms: i64,
    /// 1/2/3 scrolling, 4 bottom, 5 top, 6 reverse, 7 advanced, 8 code, 9 BAS.
    pub mode: u8,
    pub fontsize: u8,
    pub color: u32,
    /// Hash of the sender's UID (not the plaintext UID).
    pub mid_hash: String,
    pub content: String,
    /// Send time, unix seconds.
    pub ctime: i64,
    pub weight: u32,
    /// 0/1 normal pool, 2 subtitle pool.
    pub pool: u8,
    pub attr: u32,
    /// The 9th and later fields of the original `p` attribute, so that re-exporting loses nothing.
    pub extras: Option<String>,
}

impl Danmaku {
    /// Stable sort key for export.
    pub fn sort_key(&self) -> (i64, u64) {
        (self.progress_ms, self.dmid.get())
    }

    /// Seconds representation with 5 decimal places (matching the Bilibili XML).
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
// Segments
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentReq {
    pub cid: Cid,
    pub aid: Aid,
    /// 1-based segment number; one segment covers 6 minutes.
    pub index: u32,
}

/// Result of fetching one segment: the raw bytes (for verify/cache) plus the parsed items.
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
    /// `x/v2/dm/web/seg.so` (protobuf)
    Protobuf,
    /// `x/v1/dm/list.so` (deflate + xml)
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

/// The state of a segment in the database; this is what resume is based on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentState {
    pub index: u32,
    pub sha256: [u8; 32],
    pub byte_len: u64,
    pub n_items: u32,
    pub source: SegmentSource,
    pub fetched_at: i64,
}
