pub mod add;
pub mod doctor;
pub mod export;
pub mod init;
pub mod list;
pub mod show;
pub mod sync;
pub mod verify;

use std::sync::Arc;

use anyhow::Result;
use dv_bili::{BiliClient, BiliConfig};
use dv_core::{Cid, Repository, VaultError, VideoMeta};
use dv_store::Vault;

use crate::config;

/// 按 `--json` 输出结构化结果。
pub fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// unix 秒 → RFC3339。
pub fn ts_unix(ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(ts)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| ts.to_string())
}

/// 打开 vault；不存在时给出可操作的提示。
pub fn open_vault(ctx: &config::Context) -> Result<Arc<Vault>> {
    Ok(Arc::new(Vault::open(&ctx.vault_root).map_err(|err| {
        anyhow::anyhow!(
            "{err}（提示：先运行 `dv init --root {}`）",
            ctx.vault_root.display()
        )
    })?))
}

/// 构造 B 站客户端。cookie 只从环境变量 / 配置文件读，绝不落到日志里。
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

/// 把同步的 SQLite 调用挪到阻塞线程池，别占住 tokio 的 worker。
pub async fn blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> std::result::Result<T, VaultError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| anyhow::anyhow!("后台任务异常: {err}"))?
        .map_err(Into::into)
}

/// 写 `videos/<id>/meta.json`：脱离数据库也能读懂这份备份。
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

/// 把某个分 P 的弹幕落成文件（XML 始终写，JSONL 按配置）。
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
