//! `dv verify` —— 校验备份是否完整、文件与库是否一致。
//!
//! 三层检查：
//!
//! 1. **库内一致性**：分段记录数 vs 时长推算的名义段数、是否有弹幕；
//! 2. **文件一致性**：把库重新渲染成 XML，与磁盘上的文件逐字节比较（能发现手工改动、
//!    写入中断、磁盘损坏）；
//! 3. **上游一致性**（`--deep`）：逐段重新拉取并比对 sha256，报告「有哪些段变了」。
//!
//! 第 3 项不判断对错：上游变了是正常现象，只提示备份已过期。

use anyhow::Result;
use dv_bili::BiliClient;
use dv_core::{DanmakuSource, Repository, SegmentReq, VideoMeta, VideoRef};
use dv_store::Vault;
use serde::Serialize;
use std::sync::Arc;

use crate::cli::VerifyArgs;
use crate::commands::{blocking, build_client, open_vault, print_json};
use crate::config::Context;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum XmlState {
    Ok,
    Missing,
    Drift,
}

#[derive(Debug, Serialize)]
struct PageCheck {
    page: u32,
    cid: u64,
    danmaku: u64,
    segments_recorded: u32,
    segments_expected: u32,
    xml: XmlState,
    /// 本次深度校验里内容发生变化的段号。
    changed_segments: Vec<u32>,
    problems: Vec<String>,
}

impl PageCheck {
    fn is_clean(&self) -> bool {
        self.problems.is_empty() && self.xml == XmlState::Ok
    }
}

#[derive(Debug, Serialize)]
struct VideoCheck {
    id: String,
    title: String,
    pages: Vec<PageCheck>,
}

#[derive(Debug, Serialize)]
struct VerifyReport {
    deep: bool,
    /// 阻断性问题（文件缺失/损坏/库不一致）的数量。
    corrupt: usize,
    /// 上游已变化的段数（正常现象，仅提示）。
    changed: usize,
    videos: Vec<VideoCheck>,
}

pub async fn run(ctx: &Context, args: &VerifyArgs) -> Result<()> {
    let vault = open_vault(ctx)?;
    let client = if args.deep {
        Some(build_client(&ctx.config)?)
    } else {
        None
    };

    let targets = load_targets(&vault, args.id.as_deref()).await?;
    if targets.is_empty() {
        println!("没有可校验的视频：先运行 `dv add <BV号>` 与 `dv sync`");
        return Ok(());
    }

    let mut report = VerifyReport {
        deep: args.deep,
        corrupt: 0,
        changed: 0,
        videos: Vec::new(),
    };

    for meta in &targets {
        let mut video = VideoCheck {
            id: meta.folder_name(),
            title: meta.title.clone(),
            pages: Vec::new(),
        };

        for page in &meta.pages {
            let check = check_page(&vault, client.as_ref(), meta, page).await?;
            if !check.is_clean() {
                report.corrupt += 1;
            }
            report.changed += check.changed_segments.len();
            if !ctx.json {
                print_page(&video.id, &check);
            }
            video.pages.push(check);
        }

        report.videos.push(video);
    }

    if ctx.json {
        print_json(&report)?;
    } else {
        print_summary(&report, args.deep);
    }

    if report.corrupt > 0 {
        return Err(
            dv_core::VaultError::Corrupt(format!("{} 个分P 校验未通过", report.corrupt)).into(),
        );
    }
    Ok(())
}

async fn load_targets(vault: &Arc<Vault>, id: Option<&str>) -> Result<Vec<VideoMeta>> {
    if let Some(raw) = id {
        let vref = VideoRef::parse_input(raw)?;
        let bvid = vref.bvid().cloned();
        let aid = vref.aid();
        let worker = vault.clone();
        let found = blocking(move || match (&bvid, aid) {
            (Some(bvid), _) => worker.video_by_bvid(bvid),
            (None, Some(aid)) => worker.video_by_aid(aid),
            (None, None) => Ok(None),
        })
        .await?;
        return match found {
            Some(meta) => Ok(vec![meta]),
            None => Err(anyhow::anyhow!("库里没有 {raw}（先运行 `dv add {raw}`）")),
        };
    }

    let worker = vault.clone();
    let listed = blocking(move || worker.list_videos()).await?;
    let mut out = Vec::with_capacity(listed.len());
    for row in listed {
        let worker = vault.clone();
        let aid = row.aid;
        if let Some(meta) = blocking(move || worker.video_by_aid(aid)).await? {
            out.push(meta);
        }
    }
    Ok(out)
}

async fn check_page(
    vault: &Arc<Vault>,
    client: Option<&BiliClient>,
    meta: &VideoMeta,
    page: &dv_core::PageMeta,
) -> Result<PageCheck> {
    let worker = vault.clone();
    let cid = page.cid;
    let stats = blocking(move || worker.page_stats(cid)).await?;

    let expected = page.nominal_segments();
    let mut problems = Vec::new();
    if stats.danmaku == 0 {
        problems.push("库里没有任何弹幕".to_string());
    }

    // 「是否完整同步过」比「段数够不够」可靠：空分段不会入库，按条数比会误报。
    let worker = vault.clone();
    let cid = page.cid;
    let synced_at = blocking(move || worker.page_synced_at(cid)).await?;
    if synced_at.is_none() {
        problems.push("该分P 从未完整同步成功（上次抓取中断）".to_string());
    }

    // 文件一致性：重新渲染再比较，比「文件是否存在」有意义得多。
    let worker = vault.clone();
    let cid = page.cid;
    let items = blocking(move || worker.load_danmaku(cid, true)).await?;
    let rendered = dv_export::xml::render(page.cid.get(), &items);
    let xml_path = vault
        .paths()
        .page_dir(&meta.folder_name(), page.page)
        .join("danmaku.xml");

    let xml = match std::fs::read_to_string(&xml_path) {
        Ok(on_disk) if on_disk == rendered => XmlState::Ok,
        Ok(_) => {
            problems.push(format!("XML 与库不一致：{}", xml_path.display()));
            XmlState::Drift
        }
        Err(_) => {
            problems.push(format!("XML 缺失：{}", xml_path.display()));
            XmlState::Missing
        }
    };

    let mut changed_segments = Vec::new();
    if let Some(client) = client {
        for index in 1..=stats.segments.max(1) {
            let req = SegmentReq {
                cid: page.cid,
                aid: meta.aid,
                index,
            };
            match client.segment(&req).await {
                Ok(segment) => {
                    // 空分段本来就不入库，跳过比对。
                    if segment.items.is_empty() {
                        continue;
                    }
                    let worker = vault.clone();
                    let cid = page.cid;
                    let recorded = blocking(move || worker.segment_state(cid, index)).await?;
                    match recorded {
                        Some(state) if state.sha256 == segment.sha256 => {}
                        Some(_) => changed_segments.push(index),
                        None => problems.push(format!("段 {index} 抓到了但库里没有记录")),
                    }
                }
                Err(err) if err.is_permanent_skip() => {
                    changed_segments.push(index);
                }
                Err(err) => problems.push(format!("段 {index} 重新抓取失败: {err}")),
            }
        }
    }

    Ok(PageCheck {
        page: page.page,
        cid: page.cid.get(),
        danmaku: stats.danmaku,
        segments_recorded: stats.segments,
        segments_expected: expected,
        xml,
        changed_segments,
        problems,
    })
}

fn print_page(video_id: &str, check: &PageCheck) {
    let mark = if check.is_clean() { "✓" } else { "✗" };
    println!(
        "{mark} {video_id} p{} · {} 条 · 分段 {}",
        check.page, check.danmaku, check.segments_recorded
    );
    for problem in &check.problems {
        println!("    - {problem}");
    }
    if !check.changed_segments.is_empty() {
        println!("    · 上游已变化的分段: {:?}", check.changed_segments);
    }
}

fn print_summary(report: &VerifyReport, deep: bool) {
    let pages: usize = report.videos.iter().map(|v| v.pages.len()).sum();
    println!();
    if report.corrupt == 0 {
        println!("校验通过：{pages} 个分P 与库、文件一致");
    } else {
        println!("发现 {} 个分P 有问题（共 {pages} 个）", report.corrupt);
    }
    if deep {
        if report.changed == 0 {
            println!("深度校验：所有分段与上次抓取一致");
        } else {
            println!(
                "深度校验：{} 个分段在上游已变化（备份可在下次 sync 时更新）",
                report.changed
            );
        }
    }
}
