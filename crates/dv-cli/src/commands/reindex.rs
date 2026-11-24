//! `dv reindex` — rebuild the index from files on disk when the database is lost or corrupt.
//!
//! Recoverable: video metadata (`meta.json`) and danmaku (`danmaku.jsonl`).
//! Not recoverable: segment sha256 values and the "fully synced" flag, so the first `dv sync` after a rebuild
//! takes the resume path and refetches every segment (which is exactly the cross-check we want).

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
        println!("videos directory does not exist: {}", videos_dir.display());
        return Ok(());
    }

    let mut report = ReindexReport::default();

    for entry in std::fs::read_dir(&videos_dir)
        .with_context(|| format!("read failed: {}", videos_dir.display()))?
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
                eprintln!(
                    "Skipping {}: meta.json is missing or malformed",
                    video_dir.display()
                );
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
                .with_context(|| format!("parse failed: {}", jsonl_path.display()))?;
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
        "Rebuild complete: videos {} · parts {} · danmaku {} (skipped {})",
        report.videos, report.pages, report.danmaku, report.videos_skipped
    );
    println!("Consider running `dv verify` and `dv sync` next: segment state cannot be recovered from files");
    Ok(())
}
