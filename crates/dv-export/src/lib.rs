//! Export: render the danmaku stored in a vault into files.
//!
//! Two output formats:
//!
//! * **XML** — compatible with Bilibili's official format, so players and DanmakuFactory can
//!   read it directly;
//! * **JSONL** — lossless internal format with one-to-one fields, convenient for further
//!   processing.
//!
//! Two hard requirements: **deterministic output** (exporting the same data twice yields the
//! same bytes, which makes diffing and verification easy) and **no dropped fields** (whatever
//! is in `extras` is preserved verbatim).

pub mod jsonl;
pub mod xml;

use dv_core::Danmaku;

/// Sort everything up front before export so the output stays deterministic.
pub fn sorted(items: &[Danmaku]) -> Vec<&Danmaku> {
    let mut out: Vec<&Danmaku> = items.iter().collect();
    out.sort();
    out
}
