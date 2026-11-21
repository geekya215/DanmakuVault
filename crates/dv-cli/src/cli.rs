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
