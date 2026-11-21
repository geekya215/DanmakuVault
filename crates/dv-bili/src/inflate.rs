//! 弹幕接口的响应解压。
//!
//! `list.so` 返回的是 **raw deflate**（没有 zlib 头）；少数情况是 gzip 或明文。
//! 这里统一嗅探，避免调用方到处写分支。

use std::io::Read;

use dv_core::{Result, VaultError};
use flate2::read::{DeflateDecoder, GzDecoder};

/// 猜测并解压。
pub fn inflate_auto(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return gunzip(bytes);
    }

    if let Ok(out) = inflate_raw(bytes) {
        if looks_like_xml(&out) {
            return Ok(out);
        }
    }

    if looks_like_xml(bytes) {
        return Ok(bytes.to_vec());
    }

    Err(VaultError::Parse(
        "无法识别弹幕响应格式（既不是 deflate/gzip，也不是 XML）".into(),
    ))
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| VaultError::Parse(format!("gzip 解压失败: {e}")))?;
    Ok(out)
}

fn inflate_raw(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    DeflateDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| VaultError::Parse(format!("deflate 解压失败: {e}")))?;
    Ok(out)
}

fn looks_like_xml(bytes: &[u8]) -> bool {
    let head: &[u8] = &bytes[..bytes.len().min(256)];
    let trimmed = head
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .copied()
        .collect::<Vec<u8>>();
    matches!(trimmed.first(), Some(b'<'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, GzEncoder};
    use flate2::Compression;
    use std::io::Write;

    const XML: &str = r#"<?xml version="1.0"?><i><d p="1.0,1,25,16777215,0,0,ab,1">hi</d></i>"#;

    #[test]
    fn decodes_raw_deflate() {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(XML.as_bytes()).unwrap();
        let packed = enc.finish().unwrap();
        assert_ne!(packed, XML.as_bytes());
        assert_eq!(inflate_auto(&packed).unwrap(), XML.as_bytes());
    }

    #[test]
    fn decodes_gzip() {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(XML.as_bytes()).unwrap();
        let packed = enc.finish().unwrap();
        assert_eq!(inflate_auto(&packed).unwrap(), XML.as_bytes());
    }

    #[test]
    fn falls_back_to_plain_xml() {
        assert_eq!(inflate_auto(XML.as_bytes()).unwrap(), XML.as_bytes());
    }

    #[test]
    fn rejects_garbage() {
        assert!(inflate_auto(b"\x00\x01\x02 not xml").is_err());
    }
}
