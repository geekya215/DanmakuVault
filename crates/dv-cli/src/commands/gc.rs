//! `dv gc` —— 清理长期处于「已消失」状态的弹幕。
//!
//! 删除是不可逆的，所以默认只预演：必须显式 `--yes` 才会真删。

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

    // 没见过墓碑就别动数据库，也别让用户以为发生了什么。
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
            "没有需要清理的弹幕（标记消失超过 {} 天的为 0 条）",
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

    // 删了行就得重画文件，否则 verify 会立刻报「文件与库不一致」。
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
            "已删除 {deleted} 条标记消失超过 {} 天的弹幕",
            args.older_than_days
        );
    } else {
        println!(
            "将删除 {candidates} 条标记消失超过 {} 天的弹幕",
            args.older_than_days
        );
        println!("这是预演，数据没有改动。确认执行请加 --yes");
    }
    Ok(())
}
