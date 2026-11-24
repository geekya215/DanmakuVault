//! Domain identifiers.
//!
//! Each one is wrapped in a newtype so that a mistake like passing an `aid` where a `cid` is
//! expected does not turn into a bug that is hard to track down.

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
                    VaultError::InvalidId(format!(
                        "{} is not a valid {}: {s}",
                        s,
                        stringify!($name)
                    ))
                })
            }
        }
    };
}

numeric_id!(Aid, "video id (the av number)");
numeric_id!(Cid, "cid of a part");
numeric_id!(Dmid, "id of a single danmaku");

/// BV id, of the form `BV1GJ411x7h7` (case-sensitive; the `BV` prefix is always uppercase).
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
            Err(VaultError::InvalidId(format!("not a valid BV id: {s}")))
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

/// A video reference supplied by the user: at least one of BV or av is known.
///
/// Almost every endpoint needs both (`seg.so` wants `oid=cid&pid=aid`), so after parsing you
/// usually still have to call `view` once to fill in the missing one.
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

    /// Parse user input: a bare BV id, an `av` number, a plain numeric aid, or a video page URL.
    ///
    /// Short links (`b23.tv`) need network access to expand, so this reports an explicit error
    /// instead of guessing.
    pub fn parse_input(input: &str) -> Result<Self> {
        let raw = input.trim();
        if raw.is_empty() {
            return Err(VaultError::InvalidId("input is empty".into()));
        }

        if raw.contains("b23.tv") || raw.contains("bili2233.cn") {
            return Err(VaultError::InvalidId(
                "short links cannot be resolved offline; open the link in a browser first and copy the full URL or BV id"
                    .into(),
            ));
        }

        // Take the piece after /video/ out of the URL
        let candidate = if raw.contains("://") || raw.starts_with("www.") {
            let after = raw.split("/video/").nth(1).ok_or_else(|| {
                VaultError::InvalidId(format!("no /video/ segment found in URL: {raw}"))
            })?;
            after
        } else {
            raw
        };

        // Strip the query string and fragment
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
                return Err(VaultError::InvalidId(format!(
                    "av id is missing its numeric part: {raw}"
                )));
            }
            return digits
                .parse::<u64>()
                .map(|n| Self::from_aid(Aid(n)))
                .map_err(|_| VaultError::InvalidId(format!("av id is out of range: {raw}")));
        }

        if candidate.chars().all(|c| c.is_ascii_digit()) && !candidate.is_empty() {
            return candidate
                .parse::<u64>()
                .map(|n| Self::from_aid(Aid(n)))
                .map_err(|_| VaultError::InvalidId(format!("aid is out of range: {raw}")));
        }

        Err(VaultError::InvalidId(format!(
            "unrecognized video identifier: {raw}"
        )))
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
