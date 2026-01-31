//! UDP datagram 最小分流骨架（控制面 vs 数据面）。
//!
//! 设计目标：
//! - 以“首字节 kind + 剩余 payload”的极简 framing 支撑 M1 的接收骨架。
//! - 先解决“可区分消息类型”的问题，后续再冻结 wire format。

use crate::protocol::AudioFrameHeader;
use serde_json;
use thiserror::Error;

/// 控制面 datagram（JSON payload，占位实现）。
pub const DATAGRAM_KIND_CONTROL_JSON: u8 = 0;
/// 数据面 datagram（PCM16 payload，占位实现）。
pub const DATAGRAM_KIND_AUDIO_PCM16: u8 = 1;
/// 音频帧头长度字段字节数（u32 小端）。
pub const AUDIO_HEADER_LEN_BYTES: usize = 4;

#[derive(Debug, Error)]
pub enum AudioFrameError {
    #[error("audio payload too short")]
    PayloadTooShort,
    #[error("audio header length out of bounds: {0}")]
    HeaderLengthOutOfBounds(usize),
    #[error("audio header json decode failed: {0}")]
    HeaderDecodeFailed(#[from] serde_json::Error),
}

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

/// 打包数据面 PCM16 datagram（含音频帧头）。
pub fn wrap_audio_pcm16_with_header(
    header: &AudioFrameHeader,
    pcm_payload: &[u8],
) -> Result<Vec<u8>, AudioFrameError> {
    let header_json = serde_json::to_vec(header)?;
    let header_len = header_json.len();
    let mut payload = Vec::with_capacity(AUDIO_HEADER_LEN_BYTES + header_len + pcm_payload.len());
    let len_u32 = u32::try_from(header_len).unwrap_or(0);
    payload.extend_from_slice(&len_u32.to_le_bytes());
    payload.extend_from_slice(&header_json);
    payload.extend_from_slice(pcm_payload);
    Ok(wrap_audio_pcm16(&payload))
}

/// 解码数据面 PCM16 payload（含音频帧头）。
pub fn split_audio_pcm16_with_header(
    payload: &[u8],
) -> Result<(AudioFrameHeader, &[u8]), AudioFrameError> {
    if payload.len() < AUDIO_HEADER_LEN_BYTES {
        return Err(AudioFrameError::PayloadTooShort);
    }
    let len_bytes = [payload[0], payload[1], payload[2], payload[3]];
    let header_len = u32::from_le_bytes(len_bytes) as usize;
    let header_end = AUDIO_HEADER_LEN_BYTES + header_len;
    if payload.len() < header_end {
        return Err(AudioFrameError::HeaderLengthOutOfBounds(header_len));
    }
    let header_json = &payload[AUDIO_HEADER_LEN_BYTES..header_end];
    let header: AudioFrameHeader = serde_json::from_slice(header_json)?;
    let pcm = &payload[header_end..];
    Ok((header, pcm))
}

#[cfg(test)]
mod tests {
    use super::{
        split_audio_pcm16_with_header, split_datagram, wrap_audio_pcm16,
        wrap_audio_pcm16_with_header, wrap_control_json, wrap_datagram, DatagramKind,
        DATAGRAM_KIND_AUDIO_PCM16, DATAGRAM_KIND_CONTROL_JSON,
    };
    use crate::protocol::AudioFrameHeader;

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

    #[test]
    fn wrap_and_split_audio_with_header() {
        let header = AudioFrameHeader {
            session_id: "s-1".to_string(),
            seq: 42,
            timestamp_ms: 1234,
            frame_samples: 960,
        };
        let pcm = vec![0x11, 0x22, 0x33];
        let buf = wrap_audio_pcm16_with_header(&header, &pcm).expect("wrap");
        let (kind, payload) = split_datagram(&buf).expect("split datagram");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        let (decoded, decoded_pcm) = split_audio_pcm16_with_header(payload).expect("split header");
        assert_eq!(decoded.session_id, header.session_id);
        assert_eq!(decoded.seq, header.seq);
        assert_eq!(decoded.timestamp_ms, header.timestamp_ms);
        assert_eq!(decoded.frame_samples, header.frame_samples);
        assert_eq!(decoded_pcm, pcm.as_slice());
    }
}
