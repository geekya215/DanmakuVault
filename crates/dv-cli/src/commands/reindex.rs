//! `dv reindex` —— 数据库丢失/损坏时，从磁盘文件重建索引。
//!
//! 能恢复：视频元信息（`meta.json`）、弹幕（`danmaku.jsonl`）。
//! 恢复不了：分段 sha256 与「完整同步过」的标记，所以重建后第一次 `dv sync`
//! 会走断点续传、把所有分段重新抓一遍（这也正是我们要的核对）。

use anyhow::{Context as _, Result};
use dv_core::{Repository, VideoMeta};
use serde::Serialize;

use crate::commands::{blocking, open_vault, print_json};
use crate::config::Context;

#[derive(Debug, Default, Serialize)]
struct ReindexReport {
    videos: u32,
    videos_skipped: u32,
    pages: u32,
    danmaku: u64,
}

pub async fn run(ctx: &Context) -> Result<()> {
    let vault = open_vault(ctx)?;
    let videos_dir = vault.paths().videos_dir();

    if !videos_dir.is_dir() {
        println!("videos 目录不存在：{}", videos_dir.display());
        return Ok(());
    }

    let mut report = ReindexReport::default();

    for entry in std::fs::read_dir(&videos_dir)
        .with_context(|| format!("读取失败: {}", videos_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let video_dir = entry.path();
        let meta_path = video_dir.join("meta.json");

        let meta: VideoMeta = match std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
        {
            Some(meta) => meta,
            None => {
                eprintln!("跳过 {}：缺 meta.json 或格式不对", video_dir.display());
                report.videos_skipped += 1;
                continue;
            }
        };

        let worker = vault.clone();
        let to_store = meta.clone();
        blocking(move || worker.upsert_video(&to_store)).await?;
        report.videos += 1;

        for page in &meta.pages {
            let jsonl_path = video_dir
                .join(format!("p{}", page.page))
                .join("danmaku.jsonl");
            let Ok(text) = std::fs::read_to_string(&jsonl_path) else {
                continue;
            };
            let items = dv_export::jsonl::parse(&text)
                .with_context(|| format!("解析失败: {}", jsonl_path.display()))?;
            if items.is_empty() {
                continue;
            }
            let count = items.len();

            let worker = vault.clone();
            let cid = page.cid;
            let seen_at = dv_core::now_unix();
            blocking(move || worker.upsert_danmaku(cid, &items, seen_at)).await?;

            report.pages += 1;
            report.danmaku += count as u64;
        }
    }

    if ctx.json {
        return print_json(&report);
    }
    println!(
        "重建完成：{} 个视频 · {} 个分P · {} 条弹幕（跳过 {}）",
        report.videos, report.pages, report.danmaku, report.videos_skipped
    );
    println!("建议接着跑一次 `dv verify` 与 `dv sync`：分段状态无法从文件恢复");
    Ok(())
}
