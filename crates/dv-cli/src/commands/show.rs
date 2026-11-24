//! `dv show` — sync state of a single video.

use anyhow::Result;
use dv_core::{PageStats, Repository, VideoRef};
use serde::Serialize;

use crate::cli::ShowArgs;
use crate::commands::{blocking, open_vault, print_json, ts_unix};
use crate::config::Context;

#[derive(Serialize)]
struct PageRow {
    page: u32,
    cid: u64,
    part: String,
    danmaku: u64,
    missing: u64,
    segments: u32,
    synced_at: Option<String>,
}

#[derive(Serialize)]
struct ShowReport {
    id: String,
    aid: u64,
    title: String,
    owner: Option<String>,
    duration_ms: u64,
    pages: Vec<PageRow>,
}

pub async fn run(ctx: &Context, args: &ShowArgs) -> Result<()> {
    let vault = open_vault(ctx)?;
    let vref = VideoRef::parse_input(&args.id)?;
    let bvid = vref.bvid().cloned();
    let aid = vref.aid();

    let worker = vault.clone();
    let meta = blocking(move || match (&bvid, aid) {
        (Some(bvid), _) => worker.video_by_bvid(bvid),
        (None, Some(aid)) => worker.video_by_aid(aid),
        (None, None) => Ok(None),
    })
    .await?;

    let Some(meta) = meta else {
        anyhow::bail!(
            "no {} in the database (run `dv add {}` first)",
            args.id,
            args.id
        );
    };

    let pages = meta.pages.clone();
    let worker = vault.clone();
    let rows = blocking(move || {
        let mut out = Vec::with_capacity(pages.len());
        for page in &pages {
            let stats: PageStats = worker.page_stats(page.cid)?;
            out.push(PageRow {
                page: page.page,
                cid: page.cid.get(),
                part: page.part.clone(),
                danmaku: stats.danmaku,
                missing: stats.missing,
                segments: stats.segments,
                synced_at: worker.page_synced_at(page.cid)?.map(ts_unix),
            });
        }
        Ok(out)
    })
    .await?;

    let report = ShowReport {
        id: meta.folder_name(),
        aid: meta.aid.get(),
        title: meta.title.clone(),
        owner: meta
            .owner
            .as_ref()
            .map(|o| format!("{} (mid {})", o.name, o.mid)),
        duration_ms: meta.duration_ms,
        pages: rows,
    };

    if ctx.json {
        return print_json(&report);
    }

    println!("{}  \"{}\"", report.id, report.title);
    if let Some(owner) = &report.owner {
        println!("UP        {owner}");
    }
    println!("duration  {}s", report.duration_ms / 1000);
    println!();
    println!(
        "{:<4} {:>13} {:>10} {:>10} {:>8}  last sync",
        "part", "cid", "danmaku", "tombstones", "segments"
    );
    for row in &report.pages {
        println!(
            "{:<4} {:>13} {:>10} {:>10} {:>8}  {}",
            row.page,
            row.cid,
            row.danmaku,
            row.missing,
            row.segments,
            row.synced_at.clone().unwrap_or_else(|| "never".to_string())
        );
    }
    Ok(())
}
