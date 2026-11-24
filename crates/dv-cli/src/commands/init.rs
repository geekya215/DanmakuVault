//! `dv init` — create the directory, the database and the default config. Idempotent.

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

    // An existing directory/file only changes the message, never the outcome: init must be repeatable.
    let created = !root.join("index.sqlite").exists();

    let vault = Vault::create(&root)?;
    let stats = vault.stats()?;
    let schema_version = vault.schema_version()?;
    let marker = vault.read_marker();

    // A config that cannot be written must not fail the whole init: the database is already usable and the defaults work too.
    let config_written = match config::write_default_config(&ctx.config_path, &root, args.force) {
        Ok(written) => written,
        Err(err) => {
            eprintln!(
                "warning: config not written ({}); using the built-in defaults for this run",
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
        "status   {}",
        if created {
            "new"
        } else {
            "exists (data untouched)"
        }
    );
    println!("schema   v{schema_version}");
    println!(
        "data     videos {} · parts {} · danmaku {} · segments {}",
        stats.videos, stats.pages, stats.danmaku, stats.segments
    );
    println!(
        "config   {} {}",
        ctx.config_path.display(),
        if config_written {
            "(written)"
        } else {
            "(exists, unchanged)"
        }
    );
    if let Some(m) = marker {
        println!("created  {}", ts_unix(m.created_at));
    }
    println!("\nnext: dv add <ID> && dv sync");
    Ok(())
}
