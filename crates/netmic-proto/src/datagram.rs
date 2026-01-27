//! UDP datagram 最小分流骨架（控制面 vs 数据面）。
//!
//! 设计目标：
//! - 以“首字节 kind + 剩余 payload”的极简 framing 支撑 M1 的接收骨架。
//! - 先解决“可区分消息类型”的问题，后续再冻结 wire format。

/// 控制面 datagram（JSON payload，占位实现）。
pub const DATAGRAM_KIND_CONTROL_JSON: u8 = 0;
/// 数据面 datagram（PCM16 payload，占位实现）。
pub const DATAGRAM_KIND_AUDIO_PCM16: u8 = 1;

/// datagram 类型判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatagramKind {
    ControlJson,
    AudioPcm16,
    Unknown(u8),
}

impl DatagramKind {
    /// 从首字节判定 datagram 类型。
    pub fn from_byte(kind: u8) -> Self {
        match kind {
            DATAGRAM_KIND_CONTROL_JSON => Self::ControlJson,
            DATAGRAM_KIND_AUDIO_PCM16 => Self::AudioPcm16,
            other => Self::Unknown(other),
        }
    }

    /// 返回人类可读的类型名称（便于日志）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ControlJson => "control-json",
            Self::AudioPcm16 => "audio-pcm16",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// 按“首字节 kind + 剩余 payload”拆分 datagram。
///
/// - 空缓冲返回 `None`。
/// - 其余情况返回 `(DatagramKind, payload)`。
pub fn split_datagram(buf: &[u8]) -> Option<(DatagramKind, &[u8])> {
    let (first, payload) = buf.split_first()?;
    Some((DatagramKind::from_byte(*first), payload))
}

/// 按“首字节 kind + 剩余 payload”打包 datagram。
///
/// 该函数不做额外校验，仅负责 framing，便于 client/server 复用。
pub fn wrap_datagram(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + payload.len());
    buf.push(kind);
    buf.extend_from_slice(payload);
    buf
}

/// 打包控制面 JSON datagram（kind=0）。
pub fn wrap_control_json(payload: &[u8]) -> Vec<u8> {
    wrap_datagram(DATAGRAM_KIND_CONTROL_JSON, payload)
}

/// 打包数据面 PCM16 datagram（kind=1）。
pub fn wrap_audio_pcm16(payload: &[u8]) -> Vec<u8> {
    wrap_datagram(DATAGRAM_KIND_AUDIO_PCM16, payload)
}

#[cfg(test)]
mod tests {
    use super::{
        split_datagram, wrap_audio_pcm16, wrap_control_json, wrap_datagram, DatagramKind,
        DATAGRAM_KIND_AUDIO_PCM16, DATAGRAM_KIND_CONTROL_JSON,
    };

    #[test]
    fn split_empty_returns_none() {
        assert!(split_datagram(&[]).is_none());
    }

    #[test]
    fn split_control_json() {
        let buf = [0_u8, b'{', b'}'];
        let (kind, payload) = split_datagram(&buf).expect("control datagram");
        assert_eq!(kind, DatagramKind::ControlJson);
        assert_eq!(payload, b"{}");
    }

    #[test]
    fn split_audio_pcm16() {
        let buf = [1_u8, 0x34, 0x12];
        let (kind, payload) = split_datagram(&buf).expect("audio datagram");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        assert_eq!(payload, &[0x34, 0x12]);
    }

    #[test]
    fn split_unknown_kind() {
        let buf = [9_u8, 1, 2, 3];
        let (kind, payload) = split_datagram(&buf).expect("unknown datagram");
        assert_eq!(kind, DatagramKind::Unknown(9));
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn wrap_datagram_prefixes_kind() {
        let buf = wrap_datagram(7, &[1, 2, 3]);
        assert_eq!(buf, vec![7, 1, 2, 3]);
    }

    #[test]
    fn wrap_control_json_uses_constant_kind() {
        let buf = wrap_control_json(b"{}");
        assert_eq!(buf[0], DATAGRAM_KIND_CONTROL_JSON);
        assert_eq!(&buf[1..], b"{}");
    }

    #[test]
    fn wrap_audio_pcm16_uses_constant_kind() {
        let buf = wrap_audio_pcm16(&[0x34, 0x12]);
        assert_eq!(buf[0], DATAGRAM_KIND_AUDIO_PCM16);
        assert_eq!(&buf[1..], &[0x34, 0x12]);
    }
}
