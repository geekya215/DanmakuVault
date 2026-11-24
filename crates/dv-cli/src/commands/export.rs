//! `dv export` — export the danmaku in the database to files.
//!
//! By default it writes to `videos/<id>/p<N>/` inside the vault; `--out` exports to any directory instead.

use anyhow::Result;
use dv_core::{PageMeta, Repository, VideoRef};
use dv_export::{jsonl, xml};
use serde::Serialize;

use crate::cli::ExportArgs;
use crate::commands::{blocking, open_vault, plural, print_json};
use crate::config::Context;

#[derive(Serialize)]
struct ExportedPage {
    page: u32,
    cid: u64,
    path: String,
    danmaku: usize,
    bytes: usize,
}

pub async fn run(ctx: &Context, args: &ExportArgs) -> Result<()> {
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
            "No {} in the database (run `dv add {}` first, then `dv sync`)",
            args.id,
            args.id
        );
    };

    let selected = select_pages(&meta.pages, &args.page)?;
    let out_root = args
        .out
        .clone()
        .unwrap_or_else(|| vault.paths().videos_dir());
    let folder = meta.folder_name();

    let mut exported = Vec::with_capacity(selected.len());
    for page in &selected {
        let worker = vault.clone();
        let cid = page.cid;
        let include_missing = !args.exclude_missing;
        let items = blocking(move || worker.load_danmaku(cid, include_missing)).await?;

        if items.is_empty() {
            eprintln!("  p{} has no danmaku, skipping", page.page);
            continue;
        }

        let dir = out_root.join(&folder).join(format!("p{}", page.page));
        let (path, body) = if args.format == "jsonl" {
            (dir.join("danmaku.jsonl"), jsonl::render(&items))
        } else {
            (dir.join("danmaku.xml"), xml::render(page.cid.get(), &items))
        };

        dv_store::write_atomic(&path, body.as_bytes())?;

        if !ctx.json {
            println!(
                "  p{:<3} {:>7} items → {}",
                page.page,
                items.len(),
                path.display()
            );
        }
        exported.push(ExportedPage {
            page: page.page,
            cid: page.cid.get(),
            path: path.display().to_string(),
            danmaku: items.len(),
            bytes: body.len(),
        });
    }

    if ctx.json {
        return print_json(&exported);
    }

    let total: usize = exported.iter().map(|e| e.danmaku).sum();
    println!(
        "\nExported {} {} · {} danmaku · {}",
        exported.len(),
        plural(exported.len(), "part", "parts"),
        total,
        args.format
    );
    Ok(())
}

/// `--page all | N`
fn select_pages(pages: &[PageMeta], spec: &str) -> Result<Vec<PageMeta>> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok(pages.to_vec());
    }
    let want: u32 = spec
        .parse()
        .map_err(|_| anyhow::anyhow!("--page accepts only a part number (e.g. 2) or all"))?;
    let selected: Vec<PageMeta> = pages.iter().filter(|p| p.page == want).cloned().collect();
    if selected.is_empty() {
        anyhow::bail!("no part {want} (this video has {} parts)", pages.len());
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{Cid, PageMeta};

    fn page(n: u32) -> PageMeta {
        PageMeta {
            cid: Cid(n as u64 * 100),
            page: n,
            part: format!("P{n}"),
            duration_ms: 1000,
            declared_count: None,
        }
    }

    #[test]
    fn selects_all_or_one() {
        let pages = vec![page(1), page(2)];
        assert_eq!(select_pages(&pages, "all").unwrap().len(), 2);
        assert_eq!(select_pages(&pages, "").unwrap().len(), 2);
        assert_eq!(select_pages(&pages, "2").unwrap()[0].page, 2);
    }

    #[test]
    fn rejects_bad_selector() {
        let pages = vec![page(1)];
        assert!(select_pages(&pages, "3").is_err());
        assert!(select_pages(&pages, "abc").is_err());
    }
}
