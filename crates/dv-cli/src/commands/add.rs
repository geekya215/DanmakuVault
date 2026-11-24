//! `dv add` — resolve video metadata and register it in the database (danmaku is not fetched).

use anyhow::Result;
use dv_core::{DanmakuSource, Repository, VideoRef};
use serde::Serialize;

use crate::cli::AddArgs;
use crate::commands::{blocking, build_client, open_vault, plural, print_json, write_video_meta};
use crate::config::Context;

#[derive(Serialize)]
struct AddedVideo {
    id: String,
    aid: u64,
    title: String,
    pages: usize,
}

pub async fn run(ctx: &Context, args: &AddArgs) -> Result<()> {
    let vault = open_vault(ctx)?;
    let client = build_client(&ctx.config)?;
    let mut added = Vec::with_capacity(args.ids.len());

    for raw in &args.ids {
        let vref = VideoRef::parse_input(raw)?;
        let meta = client.video(&vref).await?;

        let worker = vault.clone();
        let to_store = meta.clone();
        blocking(move || worker.upsert_video(&to_store)).await?;
        write_video_meta(&vault, &meta).await?;

        if !ctx.json {
            println!(
                "✓ {:<14} \"{}\" {} parts",
                meta.folder_name(),
                meta.title,
                meta.pages.len()
            );
        }

        added.push(AddedVideo {
            id: meta.folder_name(),
            aid: meta.aid.get(),
            title: meta.title.clone(),
            pages: meta.pages.len(),
        });
    }

    if ctx.json {
        print_json(&added)?;
    } else {
        println!(
            "\nRegistered {} {}; run `dv sync` to fetch danmaku",
            added.len(),
            plural(added.len(), "video", "videos")
        );
    }
    Ok(())
}
