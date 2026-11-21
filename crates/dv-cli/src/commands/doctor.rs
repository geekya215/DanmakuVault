//! `dv doctor` —— 环境自检。
//!
//! 只报告「能不能干活」，不打印任何凭据内容。

use std::fs;

use anyhow::Result;
use dv_store::Vault;
use serde::Serialize;

use crate::commands::print_json;
use crate::config;

#[derive(Serialize)]
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl Check {
    fn new(name: &'static str, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok,
            detail: detail.into(),
        }
    }
}

#[derive(Serialize)]
struct DoctorReport {
    ok: bool,
    checks: Vec<Check>,
}

pub fn run(ctx: &config::Context) -> Result<()> {
    let mut checks = vec![Check::new(
        "程序版本",
        true,
        format!("dv {}", env!("CARGO_PKG_VERSION")),
    )];

    // vault 目录
    let root_exists = ctx.vault_root.is_dir();
    checks.push(Check::new(
        "vault 目录",
        root_exists,
        ctx.vault_root.display().to_string(),
    ));

    // 写权限探针：真的写一个小文件再删掉
    let probe = ctx.vault_root.join(".doctor-probe");
    let writable = fs::create_dir_all(&ctx.vault_root)
        .and_then(|_| fs::write(&probe, b"ok"))
        .is_ok();
    let _ = fs::remove_file(&probe);
    checks.push(Check::new(
        "目录可写",
        writable,
        if writable {
            "可写".to_string()
        } else {
            "不可写（检查权限）".to_string()
        },
    ));

    // 数据库
    let mut db_ok = false;
    match Vault::open(&ctx.vault_root) {
        Ok(vault) => {
            db_ok = true;
            match vault.schema_version() {
                Ok(version) => {
                    checks.push(Check::new("数据库 schema", true, format!("v{version}")))
                }
                Err(err) => checks.push(Check::new("数据库 schema", false, err.to_string())),
            }
            match vault.stats() {
                Ok(stats) => checks.push(Check::new(
                    "数据规模",
                    true,
                    format!(
                        "视频 {} · 分P {} · 弹幕 {}（墓碑 {}） · 分段 {}",
                        stats.videos, stats.pages, stats.danmaku, stats.missing, stats.segments
                    ),
                )),
                Err(err) => checks.push(Check::new("数据规模", false, err.to_string())),
            }
        }
        Err(err) => checks.push(Check::new("数据库", false, err.to_string())),
    }

    // 配置缺失不算失败：内置默认值就能跑，只是少了可调项。
    checks.push(Check::new(
        "配置文件",
        true,
        if ctx.config_path.exists() {
            ctx.config_path.display().to_string()
        } else {
            format!("{}（不存在，使用默认值）", ctx.config_path.display())
        },
    ));

    // 认证：M1 起生效
    let cookie = std::env::var_os("DV_COOKIE").is_some();
    checks.push(Check::new(
        "登录凭据",
        true,
        if cookie {
            "已通过 DV_COOKIE 提供".to_string()
        } else {
            "未提供（匿名模式，限速更严）".to_string()
        },
    ));

    checks.push(Check::new(
        "运行参数",
        true,
        format!(
            "qps {:.1} · 并发 {} · 超时 {}s",
            ctx.config.net.qps, ctx.config.sync.concurrency, ctx.config.net.timeout_secs
        ),
    ));

    let ok = db_ok && writable && root_exists;

    if ctx.json {
        print_json(&DoctorReport { ok, checks })?;
    } else {
        for check in &checks {
            println!(
                "{} {:<12} {}",
                if check.ok { "✓" } else { "✗" },
                check.name,
                check.detail
            );
        }
        println!();
        println!(
            "{}",
            if ok {
                "自检通过"
            } else {
                "自检未通过"
            }
        );
    }

    if !ok {
        anyhow::bail!("自检未通过，请先运行 `dv init` 或修复上面标记 ✗ 的项目");
    }
    Ok(())
}
