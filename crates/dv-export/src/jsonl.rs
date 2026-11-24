//! Lossless JSONL: one danmaku per line.
//!
//! Field names map one-to-one to database columns, and the field order is fixed (`serde`
//! emits fields in declaration order), so exporting the same data repeatedly produces
//! byte-identical output.

use dv_core::Danmaku;
use serde::{Deserialize, Serialize};

use crate::sorted;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonlDanmaku {
    pub dmid: u64,
    pub progress_ms: i64,
    pub mode: u8,
    pub fontsize: u8,
    pub color: u32,
    pub mid_hash: String,
    pub content: String,
    pub ctime: i64,
    pub weight: u32,
    pub pool: u8,
    pub attr: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extras: Option<String>,
}

impl From<&Danmaku> for JsonlDanmaku {
    fn from(d: &Danmaku) -> Self {
        Self {
            dmid: d.dmid.get(),
            progress_ms: d.progress_ms,
            mode: d.mode,
            fontsize: d.fontsize,
            color: d.color,
            mid_hash: d.mid_hash.clone(),
            content: d.content.clone(),
            ctime: d.ctime,
            weight: d.weight,
            pool: d.pool,
            attr: d.attr,
            extras: d.extras.clone(),
        }
    }
}

impl From<JsonlDanmaku> for Danmaku {
    fn from(j: JsonlDanmaku) -> Self {
        Self {
            dmid: dv_core::Dmid(j.dmid),
            progress_ms: j.progress_ms,
            mode: j.mode,
            fontsize: j.fontsize,
            color: j.color,
            mid_hash: j.mid_hash,
            content: j.content,
            ctime: j.ctime,
            weight: j.weight,
            pool: j.pool,
            attr: j.attr,
            extras: j.extras,
        }
    }
}

/// Render as JSONL text (one record per line, each line newline-terminated).
pub fn render(items: &[Danmaku]) -> String {
    let ordered = sorted(items);
    let mut out = String::with_capacity(ordered.len() * 160);
    for d in ordered {
        match serde_json::to_string(&JsonlDanmaku::from(d)) {
            Ok(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            Err(_) => continue,
        }
    }
    out
}

/// Parse JSONL (used for verification and re-import).
pub fn parse(text: &str) -> Result<Vec<Danmaku>, serde_json::Error> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: JsonlDanmaku = serde_json::from_str(line)?;
        out.push(record.into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(dmid: u64, ms: i64, content: &str) -> Danmaku {
        Danmaku {
            dmid: dv_core::Dmid(dmid),
            progress_ms: ms,
            mode: 1,
            fontsize: 25,
            color: 16_777_215,
            mid_hash: "abc".into(),
            content: content.into(),
            ctime: 1_700_000_000,
            weight: 5,
            pool: 0,
            attr: 0,
            extras: None,
        }
    }

    #[test]
    fn roundtrips_losslessly() {
        let items = vec![
            dm(2, 2_000, "with \"quotes\" and emoji 😂"),
            dm(1, 1_000, "first"),
        ];
        let text = render(&items);
        assert_eq!(text.lines().count(), 2);

        let back = parse(&text).unwrap();
        assert_eq!(back.len(), 2);
        // Everything but the lossy `extras` field must match exactly, and the output is sorted
        assert_eq!(back[0].dmid.get(), 1);
        assert_eq!(back[1].content, "with \"quotes\" and emoji 😂");
    }

    #[test]
    fn output_is_deterministic() {
        let items = vec![dm(3, 300, "c"), dm(1, 100, "a"), dm(2, 200, "b")];
        assert_eq!(render(&items), render(&items));
    }

    #[test]
    fn keeps_extras() {
        let mut d = dm(1, 0, "x");
        d.extras = Some("9,custom".into());
        let text = render(&[d]);
        assert!(text.contains("\"extras\":\"9,custom\""));
        let back = parse(&text).unwrap();
        assert_eq!(back[0].extras.as_deref(), Some("9,custom"));
    }
}
