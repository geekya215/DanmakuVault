//! `dv gc` — purge danmaku that have long been marked "missing".
//!
//! Deletion is irreversible, so the default is a dry run: nothing is actually deleted unless `--yes` is given explicitly.

use anyhow::Result;
use dv_core::{now_unix, Repository};
use serde::Serialize;

use crate::cli::GcArgs;
use crate::commands::{blocking, open_vault, print_json, write_page_artifacts};
use crate::config::Context;

#[derive(Serialize)]
struct GcReport {
    older_than_days: u32,
    cutoff: i64,
    candidates: u64,
    deleted: u64,
    dry_run: bool,
}

pub async fn run(ctx: &Context, args: &GcArgs) -> Result<()> {
    let vault = open_vault(ctx)?;
    let cutoff = now_unix() - i64::from(args.older_than_days) * 86_400;

    let worker = vault.clone();
    let candidates = blocking(move || worker.count_missing_before(cutoff)).await?;

    // If no tombstones were found, leave the database alone and do not let the user think something happened.
    if candidates == 0 {
        if ctx.json {
            return print_json(&GcReport {
                older_than_days: args.older_than_days,
                cutoff,
                candidates,
                deleted: 0,
                dry_run: !args.yes,
            });
        }
        println!(
            "No danmaku to purge (0 marked missing for more than {} days)",
            args.older_than_days
        );
        return Ok(());
    }

    let confirmed = args.yes && !args.dry_run;
    let (deleted, affected) = if confirmed {
        let worker = vault.clone();
        let outcome = blocking(move || worker.purge_missing_before(cutoff)).await?;
        (outcome.deleted, outcome.affected)
    } else {
        (0, Vec::new())
    };

    // Deleting rows means the files must be re-rendered, otherwise verify immediately reports "files do not match the database".
    if !affected.is_empty() {
        let worker = vault.clone();
        let write_jsonl = ctx.config.export.write_jsonl;
        let repainted = affected.len();
        blocking(move || {
            for cid in affected {
                if let Some((folder, page)) = worker.page_location(cid)? {
                    write_page_artifacts(&worker, cid, &folder, page, write_jsonl)?;
                }
            }
            Ok(repainted)
        })
        .await?;
    }

    let report = GcReport {
        older_than_days: args.older_than_days,
        cutoff,
        candidates,
        deleted,
        dry_run: !confirmed,
    };

    if ctx.json {
        return print_json(&report);
    }

    if confirmed {
        println!(
            "Deleted {deleted} danmaku marked missing for more than {} days",
            args.older_than_days
        );
    } else {
        println!(
            "Would delete {candidates} danmaku marked missing for more than {} days",
            args.older_than_days
        );
        println!("This is a dry run; no data was changed. Add --yes to confirm");
    }
    Ok(())
}
