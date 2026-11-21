//! 导出：把库里的弹幕渲染成文件。
//!
//! 两种输出：
//!
//! * **XML** —— 与 B 站官方格式兼容，播放器 / DanmakuFactory 能直接读；
//! * **JSONL** —— 无损内部格式，字段一一对应，方便二次处理。
//!
//! 两条硬要求：**输出确定性**（同一份数据导两次字节相同，便于 diff 与校验）、
//! **不丢字段**（`extras` 里的原样保留）。

pub mod jsonl;
pub mod xml;

use dv_core::Danmaku;

/// 导出前统一排序，保证输出确定性。
pub fn sorted(items: &[Danmaku]) -> Vec<&Danmaku> {
    let mut out: Vec<&Danmaku> = items.iter().collect();
    out.sort();
    out
}
