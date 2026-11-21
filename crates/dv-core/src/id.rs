//! 领域标识符。
//!
//! 全部用 newtype 包一层，避免把 `aid` 当 `cid` 传这类难以排查的错误。

use std::fmt;
use std::str::FromStr;

use crate::error::{Result, VaultError};

macro_rules! numeric_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Copy,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
            serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self(value)
            }
        }

        impl FromStr for $name {
            type Err = VaultError;

            fn from_str(s: &str) -> Result<Self> {
                s.trim().parse::<u64>().map(Self).map_err(|_| {
                    VaultError::InvalidId(format!("{} 不是合法的 {}: {s}", s, stringify!($name)))
                })
            }
        }
    };
}

numeric_id!(Aid, "稿件 id（av 号）");
numeric_id!(Cid, "分 P 的 cid");
numeric_id!(Dmid, "单条弹幕 id");

/// BV 号，形如 `BV1GJ411x7h7`（大小写敏感，`BV` 前缀固定大写）。
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Bvid(String);

impl Bvid {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let body = s.strip_prefix("BV").unwrap_or("");
        if s.len() == 12 && body.len() == 10 && body.chars().all(|c| c.is_ascii_alphanumeric()) {
            Ok(Self(s.to_string()))
        } else {
            Err(VaultError::InvalidId(format!("不是合法的 BV 号: {s}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Bvid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Bvid {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// 用户给出的视频定位：至少知道 BV 或 av 之一。
///
/// 绝大多数接口两者都要（`seg.so` 要 `oid=cid&pid=aid`），所以解析后通常还要调一次 `view` 补齐。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoRef {
    bvid: Option<Bvid>,
    aid: Option<Aid>,
}

impl VideoRef {
    pub fn from_bvid(bvid: Bvid) -> Self {
        Self {
            bvid: Some(bvid),
            aid: None,
        }
    }

    pub fn from_aid(aid: Aid) -> Self {
        Self {
            bvid: None,
            aid: Some(aid),
        }
    }

    pub fn bvid(&self) -> Option<&Bvid> {
        self.bvid.as_ref()
    }

    pub fn aid(&self) -> Option<Aid> {
        self.aid
    }

    /// 解析用户输入：裸 BV 号、`av` 号、纯数字 aid，或视频页 URL。
    ///
    /// 短链接（`b23.tv`）需要联网展开，这里明确报错而不是猜。
    pub fn parse_input(input: &str) -> Result<Self> {
        let raw = input.trim();
        if raw.is_empty() {
            return Err(VaultError::InvalidId("输入为空".into()));
        }

        if raw.contains("b23.tv") || raw.contains("bili2233.cn") {
            return Err(VaultError::InvalidId(
                "短链接无法离线解析，请先在浏览器里打开并复制完整 URL 或 BV 号".into(),
            ));
        }

        // 从 URL 中取出 /video/ 之后的那一段
        let candidate = if raw.contains("://") || raw.starts_with("www.") {
            let after = raw
                .split("/video/")
                .nth(1)
                .ok_or_else(|| VaultError::InvalidId(format!("URL 里没找到 /video/ 段: {raw}")))?;
            after
        } else {
            raw
        };

        // 去掉查询串与锚点
        let candidate = candidate
            .split(['?', '#'])
            .next()
            .unwrap_or(candidate)
            .trim_end_matches('/');

        let lowered = candidate.to_ascii_lowercase();
        if lowered.starts_with("bv") {
            let mut fixed = String::with_capacity(candidate.len());
            fixed.push_str("BV");
            fixed.push_str(&candidate[2..]);
            return Bvid::parse(&fixed).map(Self::from_bvid);
        }

        if let Some(rest) = lowered.strip_prefix("av") {
            let rest = rest.trim();
            let digits = rest
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>();
            if digits.is_empty() {
                return Err(VaultError::InvalidId(format!("av 号缺少数字部分: {raw}")));
            }
            return digits
                .parse::<u64>()
                .map(|n| Self::from_aid(Aid(n)))
                .map_err(|_| VaultError::InvalidId(format!("av 号超出范围: {raw}")));
        }

        if candidate.chars().all(|c| c.is_ascii_digit()) && !candidate.is_empty() {
            return candidate
                .parse::<u64>()
                .map(|n| Self::from_aid(Aid(n)))
                .map_err(|_| VaultError::InvalidId(format!("aid 超出范围: {raw}")));
        }

        Err(VaultError::InvalidId(format!("无法识别的视频标识: {raw}")))
    }
}

impl fmt::Display for VideoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.bvid, self.aid) {
            (Some(b), _) => write!(f, "{b}"),
            (None, Some(a)) => write!(f, "av{a}"),
            (None, None) => f.write_str("<empty>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bv(input: &str) -> String {
        VideoRef::parse_input(input)
            .unwrap()
            .bvid()
            .unwrap()
            .to_string()
    }

    #[test]
    fn parses_bare_bvid() {
        assert_eq!(bv("BV1GJ411x7h7"), "BV1GJ411x7h7");
        assert_eq!(bv("  BV1GJ411x7h7  "), "BV1GJ411x7h7");
        assert_eq!(bv("bv1GJ411x7h7"), "BV1GJ411x7h7");
    }

    #[test]
    fn parses_url_with_query() {
        assert_eq!(
            bv("https://www.bilibili.com/video/BV1GJ411x7h7/?spm_id_from=333.999&p=2"),
            "BV1GJ411x7h7"
        );
        assert_eq!(
            bv("https://www.bilibili.com/video/BV1GJ411x7h7"),
            "BV1GJ411x7h7"
        );
    }

    #[test]
    fn parses_av_and_aid() {
        assert_eq!(
            VideoRef::parse_input("av80433022")
                .unwrap()
                .aid()
                .unwrap()
                .get(),
            80_433_022
        );
        assert_eq!(
            VideoRef::parse_input("AV80433022")
                .unwrap()
                .aid()
                .unwrap()
                .get(),
            80_433_022
        );
        assert_eq!(
            VideoRef::parse_input("80433022")
                .unwrap()
                .aid()
                .unwrap()
                .get(),
            80_433_022
        );
    }

    #[test]
    fn rejects_short_link_and_garbage() {
        assert!(VideoRef::parse_input("https://b23.tv/abcd123").is_err());
        assert!(VideoRef::parse_input("BV1GJ411x7h").is_err());
        assert!(VideoRef::parse_input("not-a-video").is_err());
        assert!(VideoRef::parse_input("").is_err());
    }

    #[test]
    fn display_round_trip() {
        let v = VideoRef::parse_input("BV1GJ411x7h7").unwrap();
        assert_eq!(v.to_string(), "BV1GJ411x7h7");
        let a = VideoRef::parse_input("av80433022").unwrap();
        assert_eq!(a.to_string(), "av80433022");
    }
}
