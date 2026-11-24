//! Bilibili-compatible danmaku XML.
//!
//! ```xml
//! <i>
//!   <chatserver>chat.bilibili.com</chatserver>
//!   ...
//!   <d p="29.35200,1,25,16777215,1565103107,0,f7a4e1a1,1565103107899,7">high energy ahead</d>
//! </i>
//! ```
//!
//! The current official `p` format has **9 segments** (the 9th is the weight), matching the
//! online `list.so`; older players parse the first 8 segments and ignore the extra one.

use std::borrow::Cow;
use std::fmt::Write as _;

use dv_core::Danmaku;

use crate::sorted;

/// Render a complete danmaku XML document.
pub fn render(cid: u64, items: &[Danmaku]) -> String {
    let ordered = sorted(items);
    let mut out = String::with_capacity(ordered.len() * 96 + 256);

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i>\n");
    out.push_str("  <chatserver>chat.bilibili.com</chatserver>\n");
    let _ = writeln!(out, "  <chatid>{cid}</chatid>");
    out.push_str("  <mission>0</mission>\n");
    // 0 means unlimited, so players do not truncate the list a second time.
    out.push_str("  <maxlimit>0</maxlimit>\n");
    out.push_str("  <state>0</state>\n");
    out.push_str("  <real_name>0</real_name>\n");
    out.push_str("  <source>k-v</source>\n");

    for d in ordered {
        out.push_str("  <d p=\"");
        out.push_str(&p_attr(d));
        out.push_str("\">");
        out.push_str(&escape(&d.content));
        out.push_str("</d>\n");
    }

    out.push_str("</i>\n");
    out
}

/// Build the 9-segment attribute inside `<d p="...">`.
pub fn p_attr(d: &Danmaku) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{}",
        d.progress_secs(),
        d.mode,
        d.fontsize,
        d.color,
        d.ctime,
        d.pool,
        d.mid_hash,
        d.dmid.get(),
        d.weight
    )
}

/// Escape XML text. Only the 5 characters that must be escaped are replaced, and no numeric
/// entities are produced (some older players do not recognize them).
pub fn escape(text: &str) -> Cow<'_, str> {
    if !text.contains(['&', '<', '>', '"', '\'']) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

/// Unescape text, used when parsing legacy XML.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let (entity, after) = match rest.find(';') {
            Some(end) => (&rest[..=end], &rest[end + 1..]),
            None => {
                out.push_str(rest);
                return out;
            }
        };
        match entity {
            "&amp;" => out.push('&'),
            "&lt;" => out.push('<'),
            "&gt;" => out.push('>'),
            "&quot;" => out.push('"'),
            "&apos;" => out.push('\''),
            "&#39;" => out.push('\''),
            "&#10;" => out.push('\n'),
            other if other.starts_with("&#") => {
                // Numeric entity: keep it verbatim when unrecognized, do not swallow its content
                let digits = &other[2..other.len().saturating_sub(1)];
                match digits.parse::<u32>().ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(other),
                }
            }
            other => out.push_str(other),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::Dmid;

    fn dm(dmid: u64, ms: i64, content: &str) -> Danmaku {
        Danmaku {
            dmid: Dmid(dmid),
            progress_ms: ms,
            mode: 1,
            fontsize: 25,
            color: 16_777_215,
            mid_hash: "f7a4e1a1".into(),
            content: content.into(),
            ctime: 1_565_103_107,
            weight: 7,
            pool: 0,
            attr: 0,
            extras: None,
        }
    }

    #[test]
    fn renders_header_and_sorted_items() {
        let items = vec![dm(2, 5_000, "later"), dm(1, 1_000, "earlier")];
        let xml = render(137_649_199, &items);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i>\n"));
        assert!(xml.contains("<chatid>137649199</chatid>"));
        assert!(xml.contains("<maxlimit>0</maxlimit>"));
        let first = xml.find("earlier").unwrap();
        let second = xml.find("later").unwrap();
        assert!(first < second, "items should be sorted by time");
        assert!(xml.ends_with("</i>\n"));
    }

    #[test]
    fn p_attr_has_nine_fields() {
        let attr = p_attr(&dm(123, 29_352, "x"));
        let parts: Vec<&str> = attr.split(',').collect();
        assert_eq!(parts.len(), 9);
        assert_eq!(parts[0], "29.35200");
        assert_eq!(parts[1], "1");
        assert_eq!(parts[3], "16777215");
        assert_eq!(parts[7], "123");
        assert_eq!(parts[8], "7");
    }

    #[test]
    fn escapes_and_unescapes() {
        let raw = "a&b<c>d\"e'f";
        let escaped = escape(raw);
        assert_eq!(escaped, "a&amp;b&lt;c&gt;d&quot;e&apos;f");
        assert_eq!(unescape(&escaped), raw);
    }

    #[test]
    fn keeps_emoji_and_chinese_untouched() {
        // CJK text and emoji must pass through escaping unchanged.
        let raw = "前方高能 [doge] 😂";
        assert_eq!(escape(raw), raw);
        assert_eq!(unescape(raw), raw);
    }

    #[test]
    fn unescape_keeps_unknown_entities() {
        assert_eq!(unescape("&unknown;"), "&unknown;");
        assert_eq!(unescape("&#65;"), "A");
    }
}
