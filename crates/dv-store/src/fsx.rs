//! 崩溃安全的文件写入。
//!
//! 统一走「临时文件 → fsync → rename」，任何一步中断都不会留下半截文件。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn create_dir_all(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// 测试用临时目录，放在 `target/` 下。
///
/// 不用 `std::env::temp_dir()`：某些环境（沙箱、CI 容器）会对系统临时目录做额外限制。
pub fn scratch_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("dv-scratch")
        .join(name)
}

/// 原子写入：同目录临时文件 + `sync_all` + `rename` 覆盖。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let tmp = parent.join(format!(".{file_name}.tmp-{}-{stamp}", std::process::id()));

    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

pub fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec_pretty(value)?;
    buf.push(b'\n');
    write_atomic(path, &buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_file() {
        let dir = scratch_dir("fsx-replaces");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested/file.txt");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        // 不留临时文件
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "unexpected temp files: {leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
