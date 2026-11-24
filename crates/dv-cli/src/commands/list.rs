//! `dv list` — list backed-up videos.

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
        println!("No videos backed up yet: dv add <ID>");
        return Ok(());
    }

    println!(
        "{:<14} {:>5} {:>10} {:>10}  last sync",
        "video", "part", "danmaku", "tombstones"
    );
    for row in &rows {
        let id = match &row.bvid {
            Some(bvid) => bvid.to_string(),
            None => format!("av{}", row.aid.get()),
        };
        let synced = row
            .last_synced_at
            .map(ts_unix)
            .unwrap_or_else(|| "never".to_string());
        println!(
            "{:<14} {:>5} {:>10} {:>10}  {}",
            id, row.page_count, row.danmaku_count, row.missing_count, synced
        );
    }

    let total: u64 = rows.iter().map(|r| r.danmaku_count).sum();
    println!(
        "\n{} {} · {} danmaku",
        rows.len(),
        if rows.len() == 1 { "video" } else { "videos" },
        total
    );
    Ok(())
}
