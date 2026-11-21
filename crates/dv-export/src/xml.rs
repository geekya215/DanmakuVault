//! B 站兼容的弹幕 XML。
//!
//! ```xml
//! <i>
//!   <chatserver>chat.bilibili.com</chatserver>
//!   ...
//!   <d p="29.35200,1,25,16777215,1565103107,0,f7a4e1a1,1565103107899,7">前方高能</d>
//! </i>
//! ```
//!
//! `p` 现在的官方格式是 **9 段**（第 9 段是权重），与线上 `list.so` 一致；
//! 老播放器按前 8 段解析，多出来的一段会被忽略。

use std::borrow::Cow;
use std::fmt::Write as _;

use dv_core::Danmaku;

use crate::sorted;

/// 渲染整份弹幕 XML。
pub fn render(cid: u64, items: &[Danmaku]) -> String {
    let ordered = sorted(items);
    let mut out = String::with_capacity(ordered.len() * 96 + 256);

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i>\n");
    out.push_str("  <chatserver>chat.bilibili.com</chatserver>\n");
    let _ = writeln!(out, "  <chatid>{cid}</chatid>");
    out.push_str("  <mission>0</mission>\n");
    // 0 表示不限制，避免播放器再截断一次。
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

/// 构造 `<d p="...">` 里的 9 段属性。
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

/// XML 文本转义。只转 5 个必须转的字符，不产出数字实体（部分老播放器不认）。
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

/// 反转义，供解析旧版 XML 时使用。
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
                // 数字实体：不认识就原样保留，不要吞掉内容
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
        let items = vec![dm(2, 5_000, "后"), dm(1, 1_000, "前")];
        let xml = render(137_649_199, &items);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i>\n"));
        assert!(xml.contains("<chatid>137649199</chatid>"));
        assert!(xml.contains("<maxlimit>0</maxlimit>"));
        let first = xml.find("前").unwrap();
        let second = xml.find("后").unwrap();
        assert!(first < second, "应按时间排序");
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
