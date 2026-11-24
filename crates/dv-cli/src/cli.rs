//! Command-line argument definitions. The contract lives in `docs/design.md` §9.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "dv",
    version,
    about = "DanmakuVault — Bilibili danmaku backup tool",
    long_about = "Back up Bilibili video danmaku to a local vault completely and verifiably, and export it in formats players can use.",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Override the vault root directory
    #[arg(long, global = true, env = "DV_VAULT", value_name = "DIR")]
    pub vault: Option<PathBuf>,

    /// Override the config file path
    #[arg(long, global = true, env = "DV_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Machine-readable output (only results on stdout, logs on stderr)
    #[arg(long, global = true)]
    pub json: bool,

    /// Verbose logging: -v info, -vv debug, -vvv trace
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// initialize the vault: create directories, the database, and the default config
    Init(InitArgs),

    /// register a video: resolve metadata and parts, then store them (does not fetch danmaku)
    Add(AddArgs),

    /// fetch danmaku; syncs all registered videos when no ID is given
    Sync(SyncArgs),

    /// list backed-up videos
    List,

    /// show the sync status of a single video
    Show(ShowArgs),

    /// export danmaku (XML / JSONL)
    Export(ExportArgs),

    /// verify backup integrity and file consistency
    Verify(VerifyArgs),

    /// purge danmaku that have been missing for a long time
    Gc(GcArgs),

    /// rebuild the index from the files under videos/
    Reindex,

    /// run an environment self-check: database, directories, config, cookie
    Doctor,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// vault root directory (defaults to the config value, then the platform data directory)
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// Overwrite an existing config file
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// One or more BV IDs, av IDs, or video page URLs
    #[arg(required = true, value_name = "ID")]
    pub ids: Vec<String>,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Sync only these videos (BV ID / av ID / URL)
    #[arg(value_name = "ID")]
    pub ids: Vec<String>,

    /// Ignore the freshness check and re-fetch every segment
    #[arg(long)]
    pub force: bool,

    /// Print the plan only and send no requests
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// BV ID / av ID / URL
    #[arg(value_name = "ID")]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// BV ID / av ID / URL
    #[arg(value_name = "ID")]
    pub id: String,

    /// Export only this part; `all` exports every part
    #[arg(long, default_value = "all", value_name = "N|all")]
    pub page: String,

    /// Output format
    #[arg(long, default_value = "xml", value_parser = ["xml", "jsonl"])]
    pub format: String,

    /// Output directory (defaults to videos/ inside the vault)
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Exclude danmaku marked as missing (already deleted server-side)
    #[arg(long)]
    pub exclude_missing: bool,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// Verify only this video (BV ID / av ID / URL)
    #[arg(value_name = "ID")]
    pub id: Option<String>,

    /// Re-fetch every segment and compare sha256 (requires network access)
    #[arg(long)]
    pub deep: bool,
}

#[derive(Args, Debug)]
pub struct GcArgs {
    /// Only delete danmaku marked missing for more than N days
    #[arg(long, default_value_t = 30, value_name = "DAYS")]
    pub older_than_days: u32,

    /// Report only, delete nothing
    #[arg(long)]
    pub dry_run: bool,

    /// Confirm the deletion (irreversible)
    #[arg(long)]
    pub yes: bool,
}
