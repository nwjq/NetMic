//! 参数安全范围与默认值（以文档为准）。
//!
//! 参考：`MVP.md` 第 3 节与第 4 节。

/// MVP 支持的编码类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    Opus,
    Pcm16,
}

/// 内部标准格式（文档建议值）。
pub const INTERNAL_SAMPLE_RATE_HZ: u32 = 48_000;
pub const INTERNAL_CHANNELS: u16 = 1;
pub const INTERNAL_BITS_PER_SAMPLE: u16 = 16;

/// 文档给出的默认参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultParams {
    pub codec: CodecKind,
    pub sample_rate_hz: u32,
    pub chunk_ms: u32,
    pub jitter_buffer_ms: u32,
}

impl Default for DefaultParams {
    fn default() -> Self {
        Self {
            codec: CodecKind::Opus,
            sample_rate_hz: INTERNAL_SAMPLE_RATE_HZ,
            chunk_ms: 20,
            jitter_buffer_ms: 100,
        }
    }
}
