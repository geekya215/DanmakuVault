//! Bilibili HTTP client.
//!
//! It does exactly three things: assemble URLs, map HTTP/API errors into `VaultError`, and turn
//! responses into domain models.
//! Rate limiting, retries, and decompression are independent, so each can be tested on its own.

use std::time::Duration;

use async_trait::async_trait;
use prost::Message;
use serde::Deserialize;

use dv_core::{
    sha256, Aid, Bvid, Cid, Danmaku, DanmakuSource, Dmid, PageMeta, Result, Segment, SegmentReq,
    VaultError, VideoMeta, VideoRef,
};

use crate::inflate::inflate_auto;
use crate::limiter::RateLimiter;
use crate::proto::{DanmakuElem, DmSegMobileReply};
use crate::retry::RetryPolicy;

const API: &str = "https://api.bilibili.com";
const REFERER: &str = "https://www.bilibili.com/";

#[derive(Clone, Debug)]
pub struct BiliConfig {
    pub qps: f32,
    pub burst: u32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub user_agent: String,
    pub proxy: Option<String>,
    pub cookie: Option<String>,
}

impl Default for BiliConfig {
    fn default() -> Self {
        Self {
            qps: 2.0,
            burst: 4,
            timeout_secs: 20,
            max_retries: 5,
            user_agent: concat!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ",
                "(KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
            )
            .to_string(),
            proxy: None,
            cookie: None,
        }
    }
}

pub struct BiliClient {
    http: reqwest::Client,
    limiter: RateLimiter,
    retry: RetryPolicy,
    cookie: Option<String>,
}

impl BiliClient {
    pub fn new(cfg: BiliConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .user_agent(&cfg.user_agent)
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .connect_timeout(Duration::from_secs(5))
            .gzip(true)
            .redirect(reqwest::redirect::Policy::limited(3));

        if let Some(proxy) = cfg.proxy.as_deref().filter(|p| !p.is_empty()) {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(|e| VaultError::Network(format!("invalid proxy configuration: {e}")))?;
            builder = builder.proxy(proxy);
        }

        let http = builder
            .build()
            .map_err(|e| VaultError::Network(format!("failed to create the HTTP client: {e}")))?;

        Ok(Self {
            http,
            limiter: RateLimiter::new(cfg.qps, cfg.burst),
            retry: RetryPolicy::default().with_max_attempts(cfg.max_retries),
            cookie: cfg.cookie.filter(|c| !c.trim().is_empty()),
        })
    }

    // -- Infrastructure --------------------------------------------------

    /// A single request without retries; the rate limit is applied here.
    async fn http_once(&self, url: &str) -> Result<Vec<u8>> {
        self.limiter.acquire().await;

        let mut req = self
            .http
            .get(url)
            .header(reqwest::header::REFERER, REFERER)
            .header(reqwest::header::ACCEPT, "*/*");
        if let Some(cookie) = &self.cookie {
            req = req.header(reqwest::header::COOKIE, cookie);
        }

        let resp = req.send().await.map_err(net_err)?;
        let status = resp.status();
        let http_code = status.as_u16();

        // 304 / 204: Bilibili replies 304 (not an empty 200) for a segment that does not exist;
        // treat it as empty content.
        if matches!(http_code, 204 | 304) {
            return Ok(Vec::new());
        }
        if http_code == 412 || http_code == 429 {
            return Err(VaultError::RateLimited);
        }
        if status.is_server_error() {
            return Err(VaultError::Network(format!("HTTP {http_code}")));
        }
        if !status.is_success() {
            // 4xx almost always means "no permission / not found", so retrying is pointless.
            return Err(VaultError::Api {
                code: -(http_code as i32),
                msg: format!("HTTP {http_code}"),
            });
        }

        let body = resp.bytes().await.map_err(net_err)?;
        Ok(body.to_vec())
    }

    /// Fetch raw bytes with retries.
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        self.retry.run(|| async { self.http_once(url).await }).await
    }

    /// Fetch the JSON `data` field; `code != 0` counts as an API error and takes part in the retry
    /// decision.
    async fn fetch_data(&self, url: &str) -> Result<serde_json::Value> {
        self.retry
            .run(|| async {
                let bytes = self.http_once(url).await?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| VaultError::Parse(format!("response is not valid JSON: {e}")))?;
                check_api(&value)?;
                Ok(value
                    .get("data")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null))
            })
            .await
    }

    // -- Endpoints -------------------------------------------------------

    /// Video metadata plus every part.
    pub async fn fetch_video(&self, video: &VideoRef) -> Result<VideoMeta> {
        let url = match (video.bvid(), video.aid()) {
            (Some(bvid), _) => format!("{API}/x/web-interface/view?bvid={}", bvid.as_str()),
            (None, Some(aid)) => format!("{API}/x/web-interface/view?aid={}", aid.get()),
            (None, None) => return Err(VaultError::InvalidId("video reference is empty".into())),
        };

        let data = self.fetch_data(&url).await?;
        let view: ViewData = serde_json::from_value(data)
            .map_err(|e| VaultError::Parse(format!("unexpected view response shape: {e}")))?;
        view.into_meta()
    }

    /// One 6-minute segment.
    pub async fn fetch_segment(&self, req: &SegmentReq) -> Result<Segment> {
        let url = format!(
            "{API}/x/v2/dm/web/seg.so?type=1&oid={}&pid={}&segment_index={}",
            req.cid.get(),
            req.aid.get(),
            req.index
        );

        let bytes = self
            .retry
            .run(|| async {
                let body = self.http_once(&url).await?;
                // On failure the server returns JSON instead of protobuf
                if let Some(err) = json_error(&body) {
                    return Err(err);
                }
                Ok(body)
            })
            .await?;

        parse_segment(req.index, &bytes)
    }

    /// The full XML package (already decompressed).
    pub async fn fetch_legacy_xml(&self, cid: Cid) -> Result<Vec<u8>> {
        let url = format!("{API}/x/v1/dm/list.so?oid={}", cid.get());
        let bytes = self.fetch_bytes(&url).await?;
        if let Some(err) = json_error(&bytes) {
            return Err(err);
        }
        inflate_auto(&bytes)
    }
}

#[async_trait]
impl DanmakuSource for BiliClient {
    async fn video(&self, video: &VideoRef) -> Result<VideoMeta> {
        self.fetch_video(video).await
    }

    async fn segment(&self, req: &SegmentReq) -> Result<Segment> {
        self.fetch_segment(req).await
    }

    async fn legacy_xml(&self, cid: Cid) -> Result<Vec<u8>> {
        self.fetch_legacy_xml(cid).await
    }
}

// ---------------------------------------------------------------------------
// Response → domain model
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ViewData {
    aid: u64,
    #[serde(default)]
    bvid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pic: String,
    #[serde(default)]
    pubdate: i64,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    cid: u64,
    #[serde(default)]
    owner: Option<OwnerData>,
    #[serde(default)]
    pages: Vec<PageData>,
}

#[derive(Debug, Deserialize)]
struct OwnerData {
    #[serde(default)]
    mid: u64,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct PageData {
    cid: u64,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    part: String,
    #[serde(default)]
    duration: u64,
}

impl ViewData {
    fn into_meta(self) -> Result<VideoMeta> {
        if self.aid == 0 {
            return Err(VaultError::Parse("view response is missing aid".into()));
        }

        let mut pages: Vec<PageMeta> = Vec::with_capacity(self.pages.len().max(1));
        for (idx, p) in self.pages.into_iter().enumerate() {
            if p.cid == 0 {
                continue;
            }
            let seconds = if p.duration > 0 {
                p.duration
            } else {
                self.duration
            };
            pages.push(PageMeta {
                cid: Cid(p.cid),
                page: if p.page > 0 { p.page } else { idx as u32 + 1 },
                part: p.part,
                duration_ms: seconds.saturating_mul(1000),
                declared_count: None,
            });
        }

        // Fallback: a handful of responses have no pages array, so synthesize one part from the
        // top-level cid.
        if pages.is_empty() && self.cid > 0 {
            pages.push(PageMeta {
                cid: Cid(self.cid),
                page: 1,
                part: self.title.clone(),
                duration_ms: self.duration.saturating_mul(1000),
                declared_count: None,
            });
        }

        if pages.is_empty() {
            return Err(VaultError::Parse("view response contains no parts".into()));
        }

        let total_ms = pages
            .iter()
            .map(|p| p.duration_ms)
            .max()
            .unwrap_or_else(|| self.duration.saturating_mul(1000));

        Ok(VideoMeta {
            aid: Aid(self.aid),
            bvid: Bvid::parse(&self.bvid).ok(),
            title: self.title,
            owner: self.owner.map(|o| dv_core::Owner {
                mid: o.mid,
                name: o.name,
            }),
            duration_ms: total_ms,
            pubdate: (self.pubdate > 0).then_some(self.pubdate),
            cover_url: (!self.pic.is_empty()).then_some(self.pic),
            pages,
        })
    }
}

pub fn parse_segment(index: u32, bytes: &[u8]) -> Result<Segment> {
    let reply = DmSegMobileReply::decode(bytes)
        .map_err(|e| VaultError::Parse(format!("segment {index}: protobuf parse failed: {e}")))?;

    let items: Vec<Danmaku> = reply
        .elems
        .into_iter()
        .filter_map(elem_to_danmaku)
        .collect();

    Ok(Segment {
        index,
        raw: bytes.to_vec(),
        sha256: sha256(bytes),
        items,
    })
}

/// `dmid` is exposed both as a number and as a string; prefer the string (avoids large-integer
/// precision loss).
fn elem_to_danmaku(elem: DanmakuElem) -> Option<Danmaku> {
    let dmid = elem
        .id_str
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .or_else(|| u64::try_from(elem.id).ok().filter(|v| *v > 0))?;

    Some(Danmaku {
        dmid: Dmid(dmid),
        progress_ms: elem.progress.max(0) as i64,
        mode: elem.mode.clamp(0, u8::MAX as i32) as u8,
        fontsize: elem.fontsize.clamp(0, u8::MAX as i32) as u8,
        color: elem.color & 0x00FF_FFFF,
        mid_hash: if elem.mid_hash.is_empty() {
            elem.action
        } else {
            elem.mid_hash
        },
        content: elem.content,
        ctime: elem.ctime,
        weight: elem.weight.max(0) as u32,
        pool: elem.pool.clamp(0, u8::MAX as i32) as u8,
        attr: elem.attr.max(0) as u32,
        extras: None,
    })
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn net_err(err: reqwest::Error) -> VaultError {
    VaultError::Network(err.to_string())
}

fn check_api(value: &serde_json::Value) -> Result<()> {
    let code = value.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) as i32;
    if code == 0 {
        return Ok(());
    }
    let msg = value
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error")
        .to_string();
    Err(map_api_code(code, msg))
}

fn map_api_code(code: i32, msg: String) -> VaultError {
    match code {
        -412 => VaultError::RateLimited,
        -101 => VaultError::Auth,
        other => VaultError::Api { code: other, msg },
    }
}

/// Extract the error when the response body is a JSON error object (binary endpoints return
/// that on failure).
fn json_error(bytes: &[u8]) -> Option<VaultError> {
    let first = bytes.iter().find(|b| !b.is_ascii_whitespace())?;
    if *first != b'{' {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let code = value.get("code").and_then(|c| c.as_i64())? as i32;
    let msg = value
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error")
        .to_string();
    Some(map_api_code(code, msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_view_payload() {
        let raw = r#"{
            "aid": 80433022,
            "bvid": "BV1GJ411x7h7",
            "cid": 137649199,
            "title": "Title",
            "pic": "http://i0.hdslb.com/x.jpg",
            "pubdate": 1565103107,
            "duration": 213,
            "owner": { "mid": 123, "name": "UP" },
            "pages": [ { "cid": 137649199, "page": 1, "part": "P1", "duration": 213 } ]
        }"#;
        let view: ViewData = serde_json::from_str(raw).unwrap();
        let meta = view.into_meta().unwrap();
        assert_eq!(meta.aid.get(), 80_433_022);
        assert_eq!(meta.bvid.unwrap().as_str(), "BV1GJ411x7h7");
        assert_eq!(meta.pages.len(), 1);
        assert_eq!(meta.pages[0].duration_ms, 213_000);
        assert_eq!(meta.pages[0].nominal_segments(), 1);
    }

    #[test]
    fn synthesizes_page_when_missing() {
        let raw = r#"{"aid":1,"cid":99,"title":"t","duration":600}"#;
        let view: ViewData = serde_json::from_str(raw).unwrap();
        let meta = view.into_meta().unwrap();
        assert_eq!(meta.pages.len(), 1);
        assert_eq!(meta.pages[0].cid.get(), 99);
        assert_eq!(meta.pages[0].nominal_segments(), 2);
    }

    #[test]
    fn maps_api_errors() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"code":-404,"message":"nothing found","ttl":1}"#).unwrap();
        let err = check_api(&v).unwrap_err();
        assert!(matches!(err, VaultError::Api { code: -404, .. }));
        assert!(err.is_permanent_skip());

        let v: serde_json::Value =
            serde_json::from_str(r#"{"code":-412,"message":"intercepted"}"#).unwrap();
        assert!(matches!(
            check_api(&v).unwrap_err(),
            VaultError::RateLimited
        ));
    }

    #[test]
    fn detects_json_error_body_for_binary_endpoints() {
        assert!(json_error(b"  {\"code\":-500,\"message\":\"err\"}").is_some());
        // Real protobuf starts with 0x0a, so it must not be misdetected.
        assert!(json_error(&[0x0a, 0x03, 0x08, 0x01]).is_none());
    }

    #[test]
    fn skips_elems_without_id() {
        let elem = DanmakuElem {
            id: 0,
            id_str: String::new(),
            ..Default::default()
        };
        assert!(elem_to_danmaku(elem).is_none());
    }
}
