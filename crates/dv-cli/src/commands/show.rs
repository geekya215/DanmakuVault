//! `dv show` —— 单个视频的同步状态。

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
        anyhow::bail!("库里没有 {}（先运行 `dv add {}`）", args.id, args.id);
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
            .map(|o| format!("{}（mid {}）", o.name, o.mid)),
        duration_ms: meta.duration_ms,
        pages: rows,
    };

    if ctx.json {
        return print_json(&report);
    }

    println!("{}  《{}》", report.id, report.title);
    if let Some(owner) = &report.owner {
        println!("UP        {owner}");
    }
    println!("时长      {}s", report.duration_ms / 1000);
    println!();
    println!(
        "{:<4} {:>13} {:>8} {:>6} {:>4}  最近同步",
        "分P", "cid", "弹幕", "墓碑", "分段"
    );
    for row in &report.pages {
        println!(
            "{:<4} {:>13} {:>8} {:>6} {:>4}  {}",
            row.page,
            row.cid,
            row.danmaku,
            row.missing,
            row.segments,
            row.synced_at.clone().unwrap_or_else(|| "从未".to_string())
        );
    }
    Ok(())
}
