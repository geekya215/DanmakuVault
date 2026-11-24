//! Bilibili API client: video metadata, danmaku segments, and legacy XML.

pub mod client;
pub mod inflate;
pub mod limiter;
pub mod proto;
pub mod retry;

pub use client::{BiliClient, BiliConfig};
pub use proto::{DanmakuElem, DmSegMobileReply};
