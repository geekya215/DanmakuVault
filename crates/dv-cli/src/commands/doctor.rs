//! `dv doctor` — environment check.
//!
//! Only reports whether the tool can do its job; never prints the content of any credential.

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
        "version",
        true,
        format!("dv {}", env!("CARGO_PKG_VERSION")),
    )];

    // vault directory
    let root_exists = ctx.vault_root.is_dir();
    checks.push(Check::new(
        "vault dir",
        root_exists,
        ctx.vault_root.display().to_string(),
    ));

    // Write-permission probe: actually write a small file, then delete it
    let probe = ctx.vault_root.join(".doctor-probe");
    let writable = fs::create_dir_all(&ctx.vault_root)
        .and_then(|_| fs::write(&probe, b"ok"))
        .is_ok();
    let _ = fs::remove_file(&probe);
    checks.push(Check::new(
        "write access",
        writable,
        if writable {
            "ok".to_string()
        } else {
            "denied (check permissions)".to_string()
        },
    ));

    // Database
    let mut db_ok = false;
    match Vault::open(&ctx.vault_root) {
        Ok(vault) => {
            db_ok = true;
            match vault.schema_version() {
                Ok(version) => checks.push(Check::new("db schema", true, format!("v{version}"))),
                Err(err) => checks.push(Check::new("db schema", false, err.to_string())),
            }
            match vault.stats() {
                Ok(stats) => checks.push(Check::new(
                    "data size",
                    true,
                    format!(
                        "videos {} · parts {} · danmaku {} (tombstones {}) · segments {}",
                        stats.videos, stats.pages, stats.danmaku, stats.missing, stats.segments
                    ),
                )),
                Err(err) => checks.push(Check::new("data size", false, err.to_string())),
            }
        }
        Err(err) => checks.push(Check::new("database", false, err.to_string())),
    }

    // A missing config is not a failure: the built-in defaults are enough to run, only the tunables are gone.
    checks.push(Check::new(
        "config file",
        true,
        if ctx.config_path.exists() {
            ctx.config_path.display().to_string()
        } else {
            format!("{} (missing, using defaults)", ctx.config_path.display())
        },
    ));

    // Authentication: in effect from M1 on
    let cookie = std::env::var_os("DV_COOKIE").is_some();
    checks.push(Check::new(
        "credentials",
        true,
        if cookie {
            "provided via DV_COOKIE".to_string()
        } else {
            "not provided (anonymous mode, stricter rate limit)".to_string()
        },
    ));

    checks.push(Check::new(
        "parameters",
        true,
        format!(
            "qps {:.1} · concurrency {} · timeout {}s",
            ctx.config.net.qps, ctx.config.sync.concurrency, ctx.config.net.timeout_secs
        ),
    ));

    let ok = db_ok && writable && root_exists;

    if ctx.json {
        print_json(&DoctorReport { ok, checks })?;
    } else {
        for check in &checks {
            println!(
                "{} {:<13} {}",
                if check.ok { "✓" } else { "✗" },
                check.name,
                check.detail
            );
        }
        println!();
        println!(
            "{}",
            if ok {
                "environment check passed"
            } else {
                "environment check failed"
            }
        );
    }

    if !ok {
        anyhow::bail!(
            "environment check failed; run `dv init` first or fix the items marked ✗ above"
        );
    }
    Ok(())
}
