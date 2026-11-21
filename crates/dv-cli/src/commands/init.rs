//! `dv init` —— 建目录、建库、写默认配置。幂等。

use anyhow::Result;
use dv_store::Vault;
use serde::Serialize;

use crate::cli::InitArgs;
use crate::commands::{print_json, ts_unix};
use crate::config;

#[derive(Serialize)]
struct InitReport {
    vault: String,
    created: bool,
    schema_version: i32,
    config_path: String,
    config_written: bool,
    videos: u64,
    pages: u64,
    danmaku: u64,
    segments: u64,
    created_at: Option<String>,
}

pub fn run(ctx: &config::Context, args: &InitArgs) -> Result<()> {
    let root = args.root.clone().unwrap_or_else(|| ctx.vault_root.clone());

    // 目录/文件已存在只影响提示，不影响结果：init 必须可重复执行。
    let created = !root.join("index.sqlite").exists();

    let vault = Vault::create(&root)?;
    let stats = vault.stats()?;
    let schema_version = vault.schema_version()?;
    let marker = vault.read_marker();

    // 配置写不进去不该让整个 init 失败：库本身已经可用，默认值也能跑。
    let config_written = match config::write_default_config(&ctx.config_path, &root, args.force) {
        Ok(written) => written,
        Err(err) => {
            eprintln!(
                "警告: 配置未写入（{}），本次使用内置默认值",
                err.root_cause()
            );
            false
        }
    };

    if ctx.json {
        return print_json(&InitReport {
            vault: root.display().to_string(),
            created,
            schema_version,
            config_path: ctx.config_path.display().to_string(),
            config_written,
            videos: stats.videos,
            pages: stats.pages,
            danmaku: stats.danmaku,
            segments: stats.segments,
            created_at: marker.map(|m| ts_unix(m.created_at)),
        });
    }

    println!("vault    {}", root.display());
    println!(
        "状态     {}",
        if created {
            "新建"
        } else {
            "已存在（未改动数据）"
        }
    );
    println!("schema   v{schema_version}");
    println!(
        "已有     视频 {} · 分P {} · 弹幕 {} · 分段 {}",
        stats.videos, stats.pages, stats.danmaku, stats.segments
    );
    println!(
        "配置     {} {}",
        ctx.config_path.display(),
        if config_written {
            "（已写入）"
        } else {
            "（已存在，未改动）"
        }
    );
    if let Some(m) = marker {
        println!("创建于   {}", ts_unix(m.created_at));
    }
    println!("\n下一步：dv add <BV号> && dv sync");
    Ok(())
}
