//! `dv verify` — check that the backup is complete and that files match the database.
//!
//! Three layers of checks:
//!
//! 1. **Database consistency**: recorded segment count vs the nominal count derived from duration, and whether any danmaku exist;
//! 2. **File consistency**: re-render the database as XML and compare it byte for byte with the file on disk (this catches manual edits,
//!    interrupted writes, and disk corruption);
//! 3. **Upstream consistency** (`--deep`): refetch each segment and compare sha256, reporting which segments changed.
//!
//! Check 3 does not judge right or wrong: upstream changing is normal, and it only signals that the backup is stale.

use anyhow::Result;
use dv_bili::BiliClient;
use dv_core::{DanmakuSource, Repository, SegmentReq, VideoMeta, VideoRef};
use dv_store::Vault;
use serde::Serialize;
use std::sync::Arc;

use crate::cli::VerifyArgs;
use crate::commands::{blocking, build_client, open_vault, plural, print_json};
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
    /// Segment numbers whose content changed during this deep check.
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
    /// Number of blocking problems (missing or corrupt files, database mismatches).
    corrupt: usize,
    /// Number of segments changed upstream (normal; informational only).
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
        println!("No videos to verify: run `dv add <BV id>` and `dv sync` first");
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
        return Err(dv_core::VaultError::Corrupt(format!(
            "{} parts failed verification",
            report.corrupt
        ))
        .into());
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
            None => Err(anyhow::anyhow!(
                "no {raw} in the database (run `dv add {raw}` first)"
            )),
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
        problems.push("no danmaku in the database".to_string());
    }

    // "Whether a full sync ever succeeded" is more reliable than "are there enough segments": empty segments are never recorded, so a count comparison gives false alarms.
    let worker = vault.clone();
    let cid = page.cid;
    let synced_at = blocking(move || worker.page_synced_at(cid)).await?;
    if synced_at.is_none() {
        problems.push(
            "this part never completed a full sync (the last fetch was interrupted)".to_string(),
        );
    }

    // File consistency: re-rendering and comparing is far more meaningful than just "does the file exist".
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
            problems.push(format!(
                "XML does not match the database: {}",
                xml_path.display()
            ));
            XmlState::Drift
        }
        Err(_) => {
            problems.push(format!("XML missing: {}", xml_path.display()));
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
                    // Empty segments are never recorded anyway, so skip the comparison.
                    if segment.items.is_empty() {
                        continue;
                    }
                    let worker = vault.clone();
                    let cid = page.cid;
                    let recorded = blocking(move || worker.segment_state(cid, index)).await?;
                    match recorded {
                        Some(state) if state.sha256 == segment.sha256 => {}
                        Some(_) => changed_segments.push(index),
                        None => problems.push(format!(
                            "segment {index} was fetched but has no record in the database"
                        )),
                    }
                }
                Err(err) if err.is_permanent_skip() => {
                    changed_segments.push(index);
                }
                Err(err) => problems.push(format!("segment {index} refetch failed: {err}")),
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
        "{mark} {video_id} p{} · {} danmaku · {} {}",
        check.page,
        check.danmaku,
        check.segments_recorded,
        plural(check.segments_recorded as usize, "segment", "segments")
    );
    for problem in &check.problems {
        println!("    - {problem}");
    }
    if !check.changed_segments.is_empty() {
        println!(
            "    · segments changed upstream: {:?}",
            check.changed_segments
        );
    }
}

fn print_summary(report: &VerifyReport, deep: bool) {
    let pages: usize = report.videos.iter().map(|v| v.pages.len()).sum();
    println!();
    if report.corrupt == 0 {
        println!(
            "Verification passed: checked {pages} {}; database and files agree",
            plural(pages, "part", "parts")
        );
    } else {
        println!(
            "Found {} problematic {} (of {pages})",
            report.corrupt,
            plural(report.corrupt, "part", "parts")
        );
    }
    if deep {
        if report.changed == 0 {
            println!("Deep verify: all segments match the last fetch");
        } else {
            println!(
                "Deep verify: {} segments changed upstream (the backup can be updated on the next sync)",
                report.changed
            );
        }
    }
}
