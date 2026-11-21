//! `dv list` —— 列出已备份视频。

use anyhow::Result;
use dv_core::Repository;

use crate::commands::{blocking, open_vault, print_json, ts_unix};
use crate::config::Context;

pub async fn run(ctx: &Context) -> Result<()> {
    let vault = open_vault(ctx)?;
    let worker = vault.clone();
    let rows = blocking(move || worker.list_videos()).await?;

    if ctx.json {
        return print_json(&rows);
    }

    if rows.is_empty() {
        println!("还没有备份任何视频：dv add <BV号>");
        return Ok(());
    }

    println!(
        "{:<14} {:>4} {:>10} {:>6}  最近同步",
        "视频", "分P", "弹幕", "墓碑"
    );
    for row in &rows {
        let id = match &row.bvid {
            Some(bvid) => bvid.to_string(),
            None => format!("av{}", row.aid.get()),
        };
        let synced = row
            .last_synced_at
            .map(ts_unix)
            .unwrap_or_else(|| "从未".to_string());
        println!(
            "{:<14} {:>4} {:>10} {:>6}  {}",
            id, row.page_count, row.danmaku_count, row.missing_count, synced
        );
    }

    let total: u64 = rows.iter().map(|r| r.danmaku_count).sum();
    println!("\n共 {} 个视频 · {} 条弹幕", rows.len(), total);
    Ok(())
}
