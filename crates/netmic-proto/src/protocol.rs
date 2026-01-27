//! 最小协议结构（握手 / 心跳 / 统计 / 音频帧头）。
//!
//! 说明：
//! - 文档事实来源：`MVP.md` 与 `docs/PROTO.md`。
//! - 这里先提供“可编译、可序列化”的骨架，后续再接入真实网络与音频链路。

use serde::{Deserialize, Serialize};

/// 会话参数（客户端请求值 / 服务端生效值）。
///
/// 参数安全范围与回退规则以后续 `config` 模块为准；此处仅承载数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionParams {
    /// 编码类型：`"opus"` 或 `"pcm16"`。
    pub codec: String,
    /// 目标采样率（Hz），MVP 安全范围：16_000–48_000。
    pub sample_rate_hz: u32,
    /// 声道数（MVP 固定为 1 / mono）。
    pub channels: u16,
    /// 帧时长（ms），MVP 安全范围：10–60。
    pub chunk_ms: u32,
    /// Opus 比特率（kbps），仅在 codec=opus 时生效。
    pub opus_bitrate_kbps: Option<u32>,
    /// 接收端目标缓冲（ms），MVP 建议范围：40–400。
    pub jitter_buffer_ms: u32,
}

impl SessionParams {
    /// 生成与 MVP 文档默认值一致的参数集。
    pub fn mvp_default() -> Self {
        Self {
            codec: "opus".to_string(),
            sample_rate_hz: 48_000,
            channels: 1,
            chunk_ms: 20,
            opus_bitrate_kbps: Some(48),
            jitter_buffer_ms: 100,
        }
    }
}

/// 握手请求（Client → Server）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeRequest {
    /// 会话标识（建议客户端生成 UUID，服务端原样回显）。
    pub session_id: String,
    /// 客户端展示名（日志与诊断用途）。
    pub client_name: String,
    /// 客户端请求的会话参数。
    pub requested: SessionParams,
    /// 预留字段：鉴权 token（MVP 默认不启用）。
    pub token: Option<String>,
}

/// 握手响应（Server → Client）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeResponse {
    /// 会话标识（与请求一致）。
    pub session_id: String,
    /// 是否接受本次会话。
    pub accepted: bool,
    /// 拒绝或回退原因（可选）。
    pub reason: Option<String>,
    /// 服务端最终生效参数（含回退后的结果）。
    pub effective: SessionParams,
    /// 单客户端占用标记：busy=true 表示服务端已被占用。
    pub busy: bool,
}

/// 心跳报文（双向）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    /// 会话标识（来自握手阶段）。
    pub session_id: String,
    /// 单调递增序号（用于检测丢包 / 乱序）。
    pub seq: u64,
    /// 发送端时间戳（毫秒）。
    pub sent_at_ms: u64,
}

/// 统计快照（Server → Client 为主，Client → Server 可选）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatsSnapshot {
    /// 统计窗口内的接收包计数。
    pub packets_received: u64,
    /// 统计窗口内的丢包估计。
    pub packets_lost: u64,
    /// 接收端音频缓冲估算深度（帧数）。
    pub buffer_depth_frames: u64,
    /// 接收端音频缓冲估算深度（毫秒）。
    pub buffer_depth_ms: u64,
    /// 当前抖动缓冲深度（毫秒）。
    pub jitter_buffer_depth_ms: f32,
    /// 端到端估算延迟（毫秒）。
    pub estimated_e2e_latency_ms: f32,
    /// 音频 RMS 电平（占位，单位/算法后续冻结）。
    pub audio_rms: f32,
    /// 音频峰值电平（占位，通常为 PCM16 |sample| 的最大值）。
    pub audio_peak: u32,
}

/// 数据面音频帧头（不含 payload）。
///
/// 注意：实际 UDP 负载通常为“帧头 + 编码音频数据”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioFrameHeader {
    /// 会话标识。
    pub session_id: String,
    /// 音频帧序号（单调递增）。
    pub seq: u64,
    /// 帧起始时间戳（毫秒）。
    pub timestamp_ms: u64,
    /// 本帧音频采样数（解码后）。
    pub frame_samples: u32,
}
