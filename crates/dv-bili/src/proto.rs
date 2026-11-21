//! 弹幕分段的 protobuf 结构。
//!
//! 手写 `prost` tag 而不是用 `build.rs` 编译 `.proto`：字段少且稳定，
//! 换取「CI 和用户机器都不需要 protoc」。
//!
//! 字段号来自 bilibili-API-collect 的 `danmaku/danmaku_proto.md`。
//! 服务端新增字段时，`prost` 会忽略未知 tag，不需要跟着改。

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct DanmakuElem {
    /// 弹幕 id（dmid）
    #[prost(int64, tag = "1")]
    pub id: i64,
    /// 相对视频起点的毫秒偏移
    #[prost(int32, tag = "2")]
    pub progress: i32,
    /// 1/2/3 滚动，4 底部，5 顶部，6 逆向，7 高级，8 代码，9 BAS
    #[prost(int32, tag = "3")]
    pub mode: i32,
    #[prost(int32, tag = "4")]
    pub fontsize: i32,
    /// 十进制 RGB
    #[prost(uint32, tag = "5")]
    pub color: u32,
    /// 发送者 UID 的哈希
    #[prost(string, tag = "6")]
    pub mid_hash: String,
    #[prost(string, tag = "7")]
    pub content: String,
    /// 发送时间（unix 秒）
    #[prost(int64, tag = "8")]
    pub ctime: i64,
    #[prost(int32, tag = "9")]
    pub weight: i32,
    #[prost(string, tag = "10")]
    pub action: String,
    /// 0/1 普通池，2 字幕池
    #[prost(int32, tag = "11")]
    pub pool: i32,
    /// dmid 的字符串形式
    #[prost(string, tag = "12")]
    pub id_str: String,
    #[prost(int32, tag = "13")]
    pub attr: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct DmSegMobileReply {
    #[prost(message, repeated, tag = "1")]
    pub elems: Vec<DanmakuElem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_handcrafted_segment() {
        // elems[0] = { id: 42, progress: 1234, mode: 1, fontsize: 25, color: 16777215,
        //              mid_hash: "abc", content: "你好", ctime: 1700000000, weight: 7,
        //              pool: 1, id_str: "42", attr: 0 }
        let elem = DanmakuElem {
            id: 42,
            progress: 1234,
            mode: 1,
            fontsize: 25,
            color: 16_777_215,
            mid_hash: "abc".into(),
            content: "你好".into(),
            ctime: 1_700_000_000,
            weight: 7,
            action: String::new(),
            pool: 1,
            id_str: "42".into(),
            attr: 0,
        };
        let reply = DmSegMobileReply {
            elems: vec![elem.clone()],
        };
        let bytes = reply.encode_to_vec();

        let back = DmSegMobileReply::decode(&bytes[..]).unwrap();
        assert_eq!(back.elems.len(), 1);
        assert_eq!(back.elems[0], elem);
    }

    #[test]
    fn ignores_unknown_fields() {
        let elem = DanmakuElem {
            id: 7,
            content: "x".into(),
            ..Default::default()
        };
        let mut bytes = DmSegMobileReply { elems: vec![elem] }.encode_to_vec();
        // 追加一个未知的字段 99（varint），模拟服务端加字段
        bytes.extend_from_slice(&[0x98, 0x06, 0x01]);
        let back = DmSegMobileReply::decode(&bytes[..]).unwrap();
        assert_eq!(back.elems.len(), 1);
        assert_eq!(back.elems[0].id, 7);
    }
}
