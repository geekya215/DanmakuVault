pub mod doctor;
pub mod init;

use anyhow::Result;

/// 按 `--json` 输出结构化结果。
pub fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// unix 秒 → RFC3339。
pub fn ts_unix(ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(ts)
        .ok()
        .and_then(|dt| dt.format(&time::format_description::well_known::Rfc3339).ok())
        .unwrap_or_else(|| ts.to_string())
}
