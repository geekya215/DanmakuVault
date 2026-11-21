//! `dv sync` —— 抓弹幕。
//!
//! 两种模式，由「这个分 P 是否曾经完整同步过」决定：
//!
//! * **断点续传**（从未完整同步）：跳过库里已有的段，补齐剩下的。崩溃重跑走这条。
//! * **刷新**（已完整同步过）：全量重抓，比对段哈希，并在全段成功后标记墓碑。
//!
//! 之所以要分开，是因为墓碑（"这条弹幕服务端没了"）只有在「本轮真的把每一段都看过」
//! 时才成立。用缓存段拼出来的视图不能用来判定消失。

use std::sync::Arc;

use anyhow::Result;
use dv_bili::BiliClient;
use dv_core::{
    now_unix, DanmakuSource, PageMeta, Repository, SegmentReq, SegmentSource, SegmentState,
    VideoMeta, VideoRef,
};
use dv_store::Vault;
use serde::Serialize;
use tracing::info;

use crate::cli::SyncArgs;
use crate::commands::{
    blocking, build_client, open_vault, print_json, write_page_artifacts, write_video_meta,
};
use crate::config::{Config, Context};

#[derive(Debug, Default, Serialize)]
struct Totals {
    videos: u32,
    videos_failed: u32,
    pages: u32,
    pages_skipped: u32,
    pages_failed: u32,
    segments: u32,
    segments_cached: u32,
    segments_empty: u32,
    danmaku_added: u64,
    danmaku_seen: u64,
    marked_missing: u64,
}

pub async fn run(ctx: &Context, args: &SyncArgs) -> Result<()> {
    let vault = open_vault(ctx)?;
    let client = build_client(&ctx.config)?;
    let targets = resolve_targets(&vault, &client, args).await?;

    if targets.is_empty() {
        println!("没有可同步的视频：先运行 `dv add <BV号>`");
        return Ok(());
    }

    if args.dry_run {
        for meta in &targets {
            println!(
                "将同步 {:<14} 《{}》 {} 个分P",
                meta.folder_name(),
                meta.title,
                meta.pages.len()
            );
            for page in &meta.pages {
                println!(
                    "  p{:<3} cid={:<12} 约 {} 段",
                    page.page,
                    page.cid.get(),
                    page.nominal_segments()
                );
            }
        }
        return Ok(());
    }

    let mut totals = Totals::default();

    for meta in &targets {
        match sync_video(&vault, &client, meta, args.force, &ctx.config, &mut totals).await {
            Ok(()) => {
                totals.videos += 1;
                println!("✓ {:<14} 《{}》", meta.folder_name(), meta.title);
            }
            Err(err) => {
                totals.videos_failed += 1;
                eprintln!("✗ {:<14} 《{}》: {err}", meta.folder_name(), meta.title);
            }
        }
    }

    if ctx.json {
        print_json(&totals)?;
    } else {
        println!(
            "\n视频 {}（失败 {}） · 分P {}（跳过 {} 失败 {}） · 分段 {}{}",
            totals.videos,
            totals.videos_failed,
            totals.pages,
            totals.pages_skipped,
            totals.pages_failed,
            totals.segments,
            if totals.segments_cached > 0 {
                format!("（复用 {}）", totals.segments_cached)
            } else {
                String::new()
            }
        );
        if totals.segments_empty > 0 {
            println!(
                "其中 {} 个空分段（该时间段没有弹幕）",
                totals.segments_empty
            );
        }
        println!(
            "弹幕 新增 {} · 覆盖 {} · 标记消失 {}",
            totals.danmaku_added, totals.danmaku_seen, totals.marked_missing
        );
    }

    if totals.videos_failed > 0 || totals.pages_failed > 0 {
        anyhow::bail!("部分视频同步失败，已成功的数据保留；重跑会从断点继续");
    }
    Ok(())
}

async fn resolve_targets(
    vault: &Arc<Vault>,
    client: &BiliClient,
    args: &SyncArgs,
) -> Result<Vec<VideoMeta>> {
    let mut targets = Vec::new();

    if args.ids.is_empty() {
        let worker = vault.clone();
        let listed = blocking(move || worker.list_videos()).await?;
        for row in listed {
            let worker = vault.clone();
            let aid = row.aid;
            if let Some(meta) = blocking(move || worker.video_by_aid(aid)).await? {
                targets.push(meta);
            }
        }
        return Ok(targets);
    }

    for raw in &args.ids {
        let vref = VideoRef::parse_input(raw)?;
        let meta = client.video(&vref).await?;
        let worker = vault.clone();
        let to_store = meta.clone();
        blocking(move || worker.upsert_video(&to_store)).await?;
        targets.push(meta);
    }
    Ok(targets)
}

async fn sync_video(
    vault: &Arc<Vault>,
    client: &BiliClient,
    meta: &VideoMeta,
    force: bool,
    cfg: &Config,
    totals: &mut Totals,
) -> Result<()> {
    let run_start = now_unix();
    let mut failed_pages: Vec<u32> = Vec::new();

    write_video_meta(vault, meta).await?;

    for page in &meta.pages {
        let worker = vault.clone();
        let cid = page.cid;
        let synced_at = blocking(move || worker.page_synced_at(cid)).await?;

        // 新鲜度：刚同步过的跳过，避免用户手抖连点造成无意义请求。
        if !force && cfg.sync.refresh_interval_secs > 0 {
            if let Some(ts) = synced_at {
                if run_start.saturating_sub(ts) < cfg.sync.refresh_interval_secs as i64 {
                    totals.pages_skipped += 1;
                    info!(page = page.page, "分P 未到刷新间隔，跳过");
                    continue;
                }
            }
        }

        let resume = synced_at.is_none();
        match sync_page(vault, client, meta, page, run_start, resume, cfg, totals).await {
            Ok(()) => totals.pages += 1,
            Err(err) => {
                totals.pages_failed += 1;
                failed_pages.push(page.page);
                eprintln!("  p{} 失败: {err}", page.page);
            }
        }
    }

    if !failed_pages.is_empty() {
        anyhow::bail!("分P {failed_pages:?} 未完成");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn sync_page(
    vault: &Arc<Vault>,
    client: &BiliClient,
    meta: &VideoMeta,
    page: &PageMeta,
    run_start: i64,
    resume: bool,
    cfg: &Config,
    totals: &mut Totals,
) -> Result<()> {
    let nominal = page.nominal_segments();
    // 实际段数可能多于「时长 / 6min」，留几段的探测余量。
    let hard_limit = nominal + if cfg.sync.probe_extra_segment { 5 } else { 0 };

    let mut index = 1u32;
    let mut last_nonempty = true;
    let mut had_cached = false;

    while index <= hard_limit {
        // 超出名义段数后，只有上一段非空才值得继续探测。
        if index > nominal && !(cfg.sync.probe_extra_segment && last_nonempty) {
            break;
        }

        if resume && index <= nominal {
            let worker = vault.clone();
            let cid = page.cid;
            let idx = index;
            if blocking(move || worker.segment_state(cid, idx))
                .await?
                .is_some()
            {
                totals.segments_cached += 1;
                had_cached = true;
                index += 1;
                continue;
            }
        }

        let req = SegmentReq {
            cid: page.cid,
            aid: meta.aid,
            index,
        };
        let segment = client.segment(&req).await?;
        totals.segments += 1;
        last_nonempty = !segment.items.is_empty();
        if !last_nonempty {
            totals.segments_empty += 1;
        }

        let state = SegmentState {
            index,
            sha256: segment.sha256,
            byte_len: segment.raw.len() as u64,
            n_items: segment.items.len() as u32,
            source: SegmentSource::Protobuf,
            fetched_at: now_unix(),
        };

        let worker = vault.clone();
        let cid = page.cid;
        let items = segment.items;
        // 空分段不落 segment 记录：它没有任何数据，留着只会让「段数」统计失真。
        let keep_state = !items.is_empty();
        let stats = blocking(move || {
            let stats = worker.upsert_danmaku(cid, &items, run_start)?;
            if keep_state {
                worker.put_segment_state(cid, &state)?;
            }
            Ok(stats)
        })
        .await?;

        // 立刻计入总计：万一段 3 失败，前两段的成果也该出现在报告里。
        totals.danmaku_added += stats.inserted;
        totals.danmaku_seen += stats.total();
        if index <= nominal {
            info!(page = page.page, index, items = stats.total(), "分段完成");
        }
        index += 1;
    }

    let worker = vault.clone();
    let cid = page.cid;
    let folder = meta.folder_name();
    let page_no = page.page;
    let write_jsonl = cfg.export.write_jsonl;
    let marked = blocking(move || {
        // 只有「每段都是本轮新抓的」才能判定哪些弹幕消失了。
        let marked = if had_cached {
            0
        } else {
            worker.mark_missing(cid, run_start)?
        };
        worker.set_page_synced(cid, run_start)?;
        write_page_artifacts(&worker, cid, &folder, page_no, write_jsonl)?;
        Ok(marked)
    })
    .await?;

    totals.marked_missing += marked;
    Ok(())
}
