//! Protobuf structures for danmaku segments.
//!
//! The `prost` tags are hand-written instead of compiling a `.proto` in `build.rs`: the fields
//! are few and stable, which buys "neither CI nor user machines need protoc".
//!
//! Field numbers come from `danmaku/danmaku_proto.md` in bilibili-API-collect.
//! When the server adds a field, `prost` ignores the unknown tag, so this file does not have
//! to follow along.

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct DanmakuElem {
    /// Danmaku id (dmid)
    #[prost(int64, tag = "1")]
    pub id: i64,
    /// Offset in milliseconds from the start of the video
    #[prost(int32, tag = "2")]
    pub progress: i32,
    /// 1/2/3: scrolling, 4: bottom, 5: top, 6: reverse, 7: advanced, 8: scripted, 9: BAS
    #[prost(int32, tag = "3")]
    pub mode: i32,
    #[prost(int32, tag = "4")]
    pub fontsize: i32,
    /// RGB as a decimal integer
    #[prost(uint32, tag = "5")]
    pub color: u32,
    /// Hash of the sender's UID
    #[prost(string, tag = "6")]
    pub mid_hash: String,
    #[prost(string, tag = "7")]
    pub content: String,
    /// Post time (unix seconds)
    #[prost(int64, tag = "8")]
    pub ctime: i64,
    #[prost(int32, tag = "9")]
    pub weight: i32,
    #[prost(string, tag = "10")]
    pub action: String,
    /// 0/1: normal pool, 2: subtitle pool
    #[prost(int32, tag = "11")]
    pub pool: i32,
    /// String form of dmid
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
        // NOTE: the "你好" content is intentionally CJK to exercise UTF-8 round-tripping.
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
        // Append an unknown field 99 (varint) to simulate the server adding a field
        bytes.extend_from_slice(&[0x98, 0x06, 0x01]);
        let back = DmSegMobileReply::decode(&bytes[..]).unwrap();
        assert_eq!(back.elems.len(), 1);
        assert_eq!(back.elems[0].id, 7);
    }
}
