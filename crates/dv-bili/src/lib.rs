//! B 站接口客户端：视频元信息、弹幕分段、旧版 XML。

pub mod client;
pub mod inflate;
pub mod limiter;
pub mod proto;
pub mod retry;

pub use client::{BiliClient, BiliConfig};
pub use proto::{DanmakuElem, DmSegMobileReply};
