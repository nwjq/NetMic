//! 参数安全范围、默认值与回退策略（以文档为准）。
//!
//! 参考：
//! - `MVP.md` 第 3 节（可调参数与安全范围）
//! - `MVP.md` 第 4 节（格式归一化与回退策略）

use crate::protocol::SessionParams;

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

/// MVP 允许的采样率档位（`MVP.md` 3.2）。
pub const SUPPORTED_SAMPLE_RATES_HZ: [u32; 5] = [16_000, 24_000, 32_000, 44_100, 48_000];
/// MVP 允许的 chunk 档位（`MVP.md` 3.4）。
pub const SUPPORTED_CHUNK_MS: [u32; 4] = [10, 20, 40, 60];

/// Opus 比特率安全范围（kbps，`MVP.md` 3.5）。
pub const OPUS_BITRATE_MIN_KBPS: u32 = 16;
pub const OPUS_BITRATE_MAX_KBPS: u32 = 128;

/// jitter buffer 安全范围（ms，`MVP.md` 3.6）。
pub const JITTER_BUFFER_MIN_MS: u32 = 40;
pub const JITTER_BUFFER_MAX_MS: u32 = 400;

/// MVP 默认参数（与 `SessionParams::mvp_default` 对齐）。
pub const DEFAULT_CODEC: CodecKind = CodecKind::Opus;
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = INTERNAL_SAMPLE_RATE_HZ;
pub const DEFAULT_CHUNK_MS: u32 = 20;
pub const DEFAULT_OPUS_BITRATE_KBPS: u32 = 48;
pub const DEFAULT_JITTER_BUFFER_MS: u32 = 100;

/// 回退事件（用于日志与 UI 提示“已回退到 X”）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackEvent {
    /// 发生回退的字段名。
    pub field: &'static str,
    /// 请求值（字符串化，便于直接输出到日志/JSON）。
    pub requested: String,
    /// 生效值（字符串化）。
    pub applied: String,
    /// 回退原因（面向开发者/日志）。
    pub reason: &'static str,
}

/// 参数归一化结果：包含生效参数与回退记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizeResult {
    pub effective: SessionParams,
    pub fallbacks: Vec<FallbackEvent>,
}

impl NormalizeResult {
    /// 是否发生过任意回退。
    pub fn had_fallbacks(&self) -> bool {
        !self.fallbacks.is_empty()
    }
}

/// 入口：将请求参数归一化到 MVP 安全范围，并记录回退。
///
/// 设计取向：
/// - 以“默认值为基线”，只在通过校验时覆盖；
/// - 所有越界/不支持的输入都会留下 `FallbackEvent` 便于诊断。
pub fn normalize_session_params(requested: &SessionParams) -> NormalizeResult {
    // 以文档默认值为基线，后续逐项覆盖。
    let mut effective = SessionParams::mvp_default();
    let mut fallbacks = Vec::new();

    // codec：仅支持 opus / pcm16（大小写不敏感）。
    match parse_codec(&requested.codec) {
        Some(CodecKind::Opus) => {
            effective.codec = "opus".to_string();
        }
        Some(CodecKind::Pcm16) => {
            effective.codec = "pcm16".to_string();
            // pcm16 不使用 opus bitrate。
            if requested.opus_bitrate_kbps.is_some() {
                fallbacks.push(FallbackEvent {
                    field: "opus_bitrate_kbps",
                    requested: format_option_u32(requested.opus_bitrate_kbps),
                    applied: "None".to_string(),
                    reason: "pcm16 codec does not use opus bitrate",
                });
            }
            effective.opus_bitrate_kbps = None;
        }
        None => {
            fallbacks.push(FallbackEvent {
                field: "codec",
                requested: requested.codec.clone(),
                applied: "opus".to_string(),
                reason: "unsupported codec, fallback to MVP default",
            });
            effective.codec = "opus".to_string();
        }
    }

    // channels：MVP 固定 mono（`MVP.md` 3.3）。
    if requested.channels == INTERNAL_CHANNELS {
        effective.channels = requested.channels;
    } else {
        fallbacks.push(FallbackEvent {
            field: "channels",
            requested: requested.channels.to_string(),
            applied: INTERNAL_CHANNELS.to_string(),
            reason: "MVP fixes channels to mono (1)",
        });
        effective.channels = INTERNAL_CHANNELS;
    }

    // sample_rate_hz：限制为文档列出的离散档位（`MVP.md` 3.2）。
    if is_supported_sample_rate(requested.sample_rate_hz) {
        effective.sample_rate_hz = requested.sample_rate_hz;
    } else {
        fallbacks.push(FallbackEvent {
            field: "sample_rate_hz",
            requested: requested.sample_rate_hz.to_string(),
            applied: DEFAULT_SAMPLE_RATE_HZ.to_string(),
            reason: "unsupported sample rate, fallback to MVP default",
        });
        effective.sample_rate_hz = DEFAULT_SAMPLE_RATE_HZ;
    }

    // chunk_ms：限制为文档列出的离散档位（`MVP.md` 3.4）。
    if is_supported_chunk_ms(requested.chunk_ms) {
        effective.chunk_ms = requested.chunk_ms;
    } else {
        fallbacks.push(FallbackEvent {
            field: "chunk_ms",
            requested: requested.chunk_ms.to_string(),
            applied: DEFAULT_CHUNK_MS.to_string(),
            reason: "unsupported chunk size, fallback to MVP default",
        });
        effective.chunk_ms = DEFAULT_CHUNK_MS;
    }

    // jitter_buffer_ms：允许安全范围内的任意值（`MVP.md` 3.6）。
    if (JITTER_BUFFER_MIN_MS..=JITTER_BUFFER_MAX_MS).contains(&requested.jitter_buffer_ms) {
        effective.jitter_buffer_ms = requested.jitter_buffer_ms;
    } else {
        fallbacks.push(FallbackEvent {
            field: "jitter_buffer_ms",
            requested: requested.jitter_buffer_ms.to_string(),
            applied: DEFAULT_JITTER_BUFFER_MS.to_string(),
            reason: "jitter buffer out of safe range, fallback to MVP default",
        });
        effective.jitter_buffer_ms = DEFAULT_JITTER_BUFFER_MS;
    }

    // opus_bitrate_kbps：仅在 codec=opus 时生效，并受安全范围限制（`MVP.md` 3.5）。
    if effective.codec == "opus" {
        match requested.opus_bitrate_kbps {
            Some(value) if (OPUS_BITRATE_MIN_KBPS..=OPUS_BITRATE_MAX_KBPS).contains(&value) => {
                effective.opus_bitrate_kbps = Some(value);
            }
            Some(value) => {
                fallbacks.push(FallbackEvent {
                    field: "opus_bitrate_kbps",
                    requested: value.to_string(),
                    applied: DEFAULT_OPUS_BITRATE_KBPS.to_string(),
                    reason: "opus bitrate out of safe range, fallback to MVP default",
                });
                effective.opus_bitrate_kbps = Some(DEFAULT_OPUS_BITRATE_KBPS);
            }
            None => {
                fallbacks.push(FallbackEvent {
                    field: "opus_bitrate_kbps",
                    requested: "None".to_string(),
                    applied: DEFAULT_OPUS_BITRATE_KBPS.to_string(),
                    reason: "opus codec requires bitrate, fallback to MVP default",
                });
                effective.opus_bitrate_kbps = Some(DEFAULT_OPUS_BITRATE_KBPS);
            }
        }
    } else {
        // codec=pcm16 时明确清空 opus_bitrate_kbps（即便请求了合法值）。
        effective.opus_bitrate_kbps = None;
    }

    NormalizeResult {
        effective,
        fallbacks,
    }
}

fn parse_codec(raw: &str) -> Option<CodecKind> {
    match raw.to_ascii_lowercase().as_str() {
        "opus" => Some(CodecKind::Opus),
        "pcm16" | "pcm" => Some(CodecKind::Pcm16),
        _ => None,
    }
}

fn is_supported_sample_rate(value: u32) -> bool {
    SUPPORTED_SAMPLE_RATES_HZ.contains(&value)
}

fn is_supported_chunk_ms(value: u32) -> bool {
    SUPPORTED_CHUNK_MS.contains(&value)
}

fn format_option_u32(value: Option<u32>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "None".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_session_params, CodecKind, NormalizeResult, DEFAULT_CHUNK_MS,
        DEFAULT_JITTER_BUFFER_MS, DEFAULT_OPUS_BITRATE_KBPS, DEFAULT_SAMPLE_RATE_HZ,
        INTERNAL_CHANNELS,
    };
    use crate::protocol::SessionParams;

    fn assert_no_fallbacks(result: &NormalizeResult) {
        assert!(
            result.fallbacks.is_empty(),
            "expected no fallbacks, got: {:?}",
            result.fallbacks
        );
        assert!(!result.had_fallbacks());
    }

    #[test]
    fn normalize_keeps_valid_opus_params() {
        let requested = SessionParams {
            codec: "opus".to_string(),
            sample_rate_hz: 48_000,
            channels: INTERNAL_CHANNELS,
            chunk_ms: 20,
            opus_bitrate_kbps: Some(64),
            jitter_buffer_ms: 120,
        };

        let result = normalize_session_params(&requested);
        assert_no_fallbacks(&result);
        assert_eq!(result.effective, requested);
    }

    #[test]
    fn normalize_falls_back_on_invalid_sample_rate_and_chunk() {
        let requested = SessionParams {
            codec: "opus".to_string(),
            sample_rate_hz: 96_000,
            channels: 2,
            chunk_ms: 15,
            opus_bitrate_kbps: Some(48),
            jitter_buffer_ms: 100,
        };

        let result = normalize_session_params(&requested);
        assert!(result.had_fallbacks());
        assert_eq!(result.effective.sample_rate_hz, DEFAULT_SAMPLE_RATE_HZ);
        assert_eq!(result.effective.chunk_ms, DEFAULT_CHUNK_MS);
        assert_eq!(result.effective.channels, INTERNAL_CHANNELS);
        assert!(
            result
                .fallbacks
                .iter()
                .any(|event| event.field == "sample_rate_hz")
        );
        assert!(
            result
                .fallbacks
                .iter()
                .any(|event| event.field == "chunk_ms")
        );
        assert!(
            result
                .fallbacks
                .iter()
                .any(|event| event.field == "channels")
        );
    }

    #[test]
    fn normalize_pcm16_clears_opus_bitrate() {
        let requested = SessionParams {
            codec: "pcm16".to_string(),
            sample_rate_hz: 48_000,
            channels: INTERNAL_CHANNELS,
            chunk_ms: 20,
            opus_bitrate_kbps: Some(64),
            jitter_buffer_ms: 100,
        };

        let result = normalize_session_params(&requested);
        assert!(result.had_fallbacks());
        assert_eq!(result.effective.codec, "pcm16");
        assert_eq!(result.effective.opus_bitrate_kbps, None);
        assert!(
            result
                .fallbacks
                .iter()
                .any(|event| event.field == "opus_bitrate_kbps")
        );
    }

    #[test]
    fn normalize_opus_bitrate_out_of_range_uses_default() {
        let requested = SessionParams {
            codec: "opus".to_string(),
            sample_rate_hz: 48_000,
            channels: INTERNAL_CHANNELS,
            chunk_ms: 20,
            opus_bitrate_kbps: Some(512),
            jitter_buffer_ms: 1_000,
        };

        let result = normalize_session_params(&requested);
        assert!(result.had_fallbacks());
        assert_eq!(
            result.effective.opus_bitrate_kbps,
            Some(DEFAULT_OPUS_BITRATE_KBPS)
        );
        assert_eq!(result.effective.jitter_buffer_ms, DEFAULT_JITTER_BUFFER_MS);
    }

    #[test]
    fn codec_kind_is_still_exposed_for_callers() {
        // 该测试主要用于避免 `CodecKind` 被误删（对外 API 的占位保证）。
        assert_eq!(CodecKind::Opus, CodecKind::Opus);
        assert_eq!(CodecKind::Pcm16, CodecKind::Pcm16);
    }
}
