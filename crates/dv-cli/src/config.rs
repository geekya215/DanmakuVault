//! 配置：内置默认 → 配置文件 → 环境变量 → 命令行。
//!
//! 只加载需要的字段；写回时保持结构稳定，方便用户手工编辑。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::cli::Cli;

pub const APP_NAME: &str = "danmaku-vault";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub vault: VaultConfig,
    pub net: NetConfig,
    pub sync: SyncConfig,
    pub export: ExportConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultConfig {
    /// 留空表示用平台数据目录。
    pub root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    /// 全局限速（每秒请求数）。
    pub qps: f32,
    pub burst: u32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub user_agent: String,
    /// 例如 `http://127.0.0.1:7890`。
    pub proxy: Option<String>,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            qps: 2.0,
            burst: 4,
            timeout_secs: 20,
            max_retries: 5,
            user_agent: concat!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ",
                "(KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36"
            )
            .to_string(),
            proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// 同时在途的视频数。
    pub concurrency: usize,
    /// 保留原始分段字节（便于复核，占空间）。
    pub save_raw_segments: bool,
    /// 全量同步后没再看到的弹幕保留为墓碑，不物理删除。
    pub retain_missing: bool,
    /// 段数按视频时长算完之后再多探一段。
    pub probe_extra_segment: bool,
    /// 已完整同步过的分 P 在这个间隔内跳过（秒）；0 表示每次都重新检查。
    pub refresh_interval_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            save_raw_segments: false,
            retain_missing: true,
            probe_extra_segment: true,
            refresh_interval_secs: 6 * 3600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportConfig {
    pub default_format: String,
    /// 导出 XML 时同时写一份无损 JSONL。
    pub write_jsonl: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            default_format: "xml".into(),
            write_jsonl: true,
        }
    }
}

/// 一次命令运行的上下文：已解析的路径与配置。
#[derive(Debug, Clone)]
pub struct Context {
    pub vault_root: PathBuf,
    pub config: Config,
    pub config_path: PathBuf,
    pub json: bool,
}

impl Context {
    pub fn load(cli: &Cli) -> Result<Self> {
        let config_path = cli
            .config
            .clone()
            .or_else(|| std::env::var_os("DV_CONFIG").map(PathBuf::from))
            .unwrap_or_else(default_config_path);

        let config = load_config(&config_path)?;

        let vault_root = cli
            .vault
            .clone()
            .or_else(|| std::env::var_os("DV_VAULT").map(PathBuf::from))
            .or_else(|| config.vault.root.clone())
            .unwrap_or_else(default_vault_root);

        Ok(Self {
            vault_root,
            config,
            config_path,
            json: cli.json,
        })
    }
}

pub fn default_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.config_dir().join(APP_NAME).join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("dv-config.toml"))
}

pub fn default_vault_root() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.data_local_dir().join(APP_NAME).join("vault"))
        .unwrap_or_else(|| PathBuf::from("dv-vault"))
}

/// 配置文件不存在时返回默认值，而不是报错。
pub fn load_config(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            toml::from_str(&raw).with_context(|| format!("配置文件解析失败: {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(err).with_context(|| format!("读取配置失败: {}", path.display())),
    }
}

/// 写入默认配置；`force` 为假且文件已存在时跳过。
///
/// 返回是否真的写了。
pub fn write_default_config(path: &Path, vault_root: &Path, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    let mut config = Config::default();
    config.vault.root = Some(vault_root.to_path_buf());
    let text = toml::to_string_pretty(&config).context("序列化默认配置失败")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
    }
    dv_store::write_atomic(path, text.as_bytes())
        .with_context(|| format!("写配置失败: {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_toml_and_roundtrips() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.net.qps, cfg.net.qps);
        assert_eq!(back.sync.concurrency, cfg.sync.concurrency);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = std::env::temp_dir().join("dv-no-such-config-xyz.toml");
        let _ = std::fs::remove_file(&path);
        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.export.default_format, "xml");
    }
}
