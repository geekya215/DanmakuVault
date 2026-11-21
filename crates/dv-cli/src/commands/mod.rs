pub mod add;
pub mod doctor;
pub mod init;
pub mod list;
pub mod show;
pub mod sync;

use std::sync::Arc;

use anyhow::Result;
use dv_bili::{BiliClient, BiliConfig};
use dv_core::VaultError;
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
