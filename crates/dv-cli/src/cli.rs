//! 命令行参数定义。契约见 `docs/design.md` §9。

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "dv",
    version,
    about = "DanmakuVault — B 站弹幕备份工具",
    long_about = "把 B 站视频弹幕完整、可校验地备份到本地，并导出为播放器可用的格式。",
    arg_required_else_help = true
)]
pub struct Cli {
    /// 覆盖 vault 根目录
    #[arg(long, global = true, env = "DV_VAULT", value_name = "DIR")]
    pub vault: Option<PathBuf>,

    /// 覆盖配置文件路径
    #[arg(long, global = true, env = "DV_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// 机器可读输出（stdout 只留结果，日志走 stderr）
    #[arg(long, global = true)]
    pub json: bool,

    /// 详细日志：-v 信息，-vv 调试，-vvv 追踪
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 初始化 vault：建目录、建库、写默认配置
    Init(InitArgs),

    /// 登记视频：解析元信息与分 P 并落库（不抓弹幕）
    Add(AddArgs),

    /// 抓取弹幕；省略 ID 时同步库里全部已登记视频
    Sync(SyncArgs),

    /// 列出已备份的视频
    List,

    /// 查看单个视频的同步状态
    Show(ShowArgs),

    /// 导出弹幕（XML / JSONL）
    Export(ExportArgs),

    /// 校验备份完整性与文件一致性
    Verify(VerifyArgs),

    /// 清理长期处于「已消失」状态的弹幕
    Gc(GcArgs),

    /// 从 videos/ 下的文件重建索引
    Reindex,

    /// 环境自检：库、目录、配置、cookie
    Doctor,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// vault 根目录（默认取配置，其次平台数据目录）
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// 覆盖已存在的配置文件
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// BV 号 / av 号 / 视频页 URL，可给多个
    #[arg(required = true, value_name = "ID")]
    pub ids: Vec<String>,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// 只同步这些视频（BV 号 / av 号 / URL）
    #[arg(value_name = "ID")]
    pub ids: Vec<String>,

    /// 忽略新鲜度检查，强制重新抓取全部分段
    #[arg(long)]
    pub force: bool,

    /// 只打印计划，不发请求
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// BV 号 / av 号 / URL
    #[arg(value_name = "ID")]
    pub id: String,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// BV 号 / av 号 / URL
    #[arg(value_name = "ID")]
    pub id: String,

    /// 只导出指定分P；`all` 表示全部
    #[arg(long, default_value = "all", value_name = "N|all")]
    pub page: String,

    /// 输出格式
    #[arg(long, default_value = "xml", value_parser = ["xml", "jsonl"])]
    pub format: String,

    /// 输出目录（默认写回 vault 内的 videos/）
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// 排除已标记为消失（服务端已删除）的弹幕
    #[arg(long)]
    pub exclude_missing: bool,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// 只校验这个视频（BV 号 / av 号 / URL）
    #[arg(value_name = "ID")]
    pub id: Option<String>,

    /// 逐段重新拉取并比对 sha256（需要联网）
    #[arg(long)]
    pub deep: bool,
}

#[derive(Args, Debug)]
pub struct GcArgs {
    /// 只删除标记为消失超过 N 天的弹幕
    #[arg(long, default_value_t = 30, value_name = "DAYS")]
    pub older_than_days: u32,

    /// 只统计，不删除
    #[arg(long)]
    pub dry_run: bool,

    /// 确认执行删除（不可逆）
    #[arg(long)]
    pub yes: bool,
}
