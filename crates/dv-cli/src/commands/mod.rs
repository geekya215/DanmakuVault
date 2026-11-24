pub mod add;
pub mod doctor;
pub mod export;
pub mod gc;
pub mod init;
pub mod list;
pub mod reindex;
pub mod show;
pub mod sync;
pub mod verify;

use std::sync::Arc;

use anyhow::Result;
use dv_bili::{BiliClient, BiliConfig};
use dv_core::{Cid, Repository, VaultError, VideoMeta};
use dv_store::Vault;

use crate::config;

/// Print a structured result when `--json` is given.
pub fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Unix seconds → RFC3339.
pub fn ts_unix(ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(ts)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| ts.to_string())
}

/// Open the vault; when it is missing, point the user at an actionable fix.
pub fn open_vault(ctx: &config::Context) -> Result<Arc<Vault>> {
    Ok(Arc::new(Vault::open(&ctx.vault_root).map_err(|err| {
        anyhow::anyhow!(
            "{err} (hint: run `dv init --root {}` first)",
            ctx.vault_root.display()
        )
    })?))
}

/// Build the Bilibili client. The cookie is read only from the environment / config file and never lands in logs.
pub fn build_client(cfg: &config::Config) -> Result<BiliClient> {
    let client = BiliClient::new(BiliConfig {
        qps: cfg.net.qps,
        burst: cfg.net.burst,
        timeout_secs: cfg.net.timeout_secs,
        max_retries: cfg.net.max_retries,
        user_agent: cfg.net.user_agent.clone(),
        proxy: cfg.net.proxy.clone(),
        cookie: cookie_from_env(),
    })?;
    Ok(client)
}

fn cookie_from_env() -> Option<String> {
    std::env::var("DV_COOKIE")
        .ok()
        .filter(|c| !c.trim().is_empty())
}

/// Move synchronous SQLite calls onto the blocking thread pool instead of tying up tokio workers.
pub async fn blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> std::result::Result<T, VaultError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| anyhow::anyhow!("background task failed: {err}"))?
        .map_err(Into::into)
}

/// Singular or plural form for `n`.
pub fn plural<'a>(n: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if n == 1 {
        singular
    } else {
        plural
    }
}

/// Write `videos/<id>/meta.json`: this backup stays readable without the database.
pub async fn write_video_meta(vault: &Arc<Vault>, meta: &VideoMeta) -> Result<()> {
    let worker = vault.clone();
    let folder = meta.folder_name();
    let snapshot = meta.clone();
    blocking(move || {
        let dir = worker.paths().video_dir(&folder);
        dv_store::write_json_atomic(&dir.join("meta.json"), &snapshot)?;
        Ok(())
    })
    .await
}

/// Materialize the danmaku of one part as files (XML is always written, JSONL per config).
///
/// Called both after a sync completes and after `gc`, so the files on disk always stay a faithful projection of the database.
pub fn write_page_artifacts(
    vault: &Vault,
    cid: Cid,
    folder: &str,
    page: u32,
    write_jsonl: bool,
) -> dv_core::Result<()> {
    let items = vault.load_danmaku(cid, true)?;
    let dir = vault.paths().page_dir(folder, page);

    let xml = dv_export::xml::render(cid.get(), &items);
    dv_store::write_atomic(&dir.join("danmaku.xml"), xml.as_bytes())?;

    if write_jsonl {
        let jsonl = dv_export::jsonl::render(&items);
        dv_store::write_atomic(&dir.join("danmaku.jsonl"), jsonl.as_bytes())?;
    }
    Ok(())
}
