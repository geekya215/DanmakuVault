//! Config layering: built-in defaults → config file → environment variables → command line.
//!
//! Only the fields that are needed get loaded; writes keep the structure stable so the file
//! stays easy to edit by hand.

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
    /// Empty means the platform data directory is used.
    pub root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    /// Global rate limit (requests per second).
    pub qps: f32,
    pub burst: u32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub user_agent: String,
    /// For example, `http://127.0.0.1:7890`.
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
    /// Number of videos in flight at once.
    pub concurrency: usize,
    /// Keep the raw segment bytes (useful for re-checking, but uses disk space).
    pub save_raw_segments: bool,
    /// Danmaku no longer seen after a full sync are kept as tombstones, never physically deleted.
    pub retain_missing: bool,
    /// Probe one extra segment after counting segments from the video duration.
    pub probe_extra_segment: bool,
    /// Parts already fully synced are skipped for this interval (seconds); 0 re-checks every time.
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
    /// Also write a lossless JSONL copy when exporting XML.
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

/// Context for one command run: resolved paths and config.
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

/// Return defaults when the config file is missing instead of failing.
pub fn load_config(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw)
            .with_context(|| format!("failed to parse config file: {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(err).with_context(|| format!("failed to read config: {}", path.display())),
    }
}

/// Write the default config; skipped when `force` is false and the file already exists.
///
/// Returns whether anything was actually written.
pub fn write_default_config(path: &Path, vault_root: &Path, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    let mut config = Config::default();
    config.vault.root = Some(vault_root.to_path_buf());
    let text = toml::to_string_pretty(&config).context("failed to serialize the default config")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }
    dv_store::write_atomic(path, text.as_bytes())
        .with_context(|| format!("failed to write config: {}", path.display()))?;
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
