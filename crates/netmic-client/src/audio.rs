//! 客户端采集/重采样骨架（占位实现）。
//!
//! 目标：
//! - 形成“采集 → 重采样 → 统一 PCM16 mono 48k 帧”的接口形态；
//! - 当前用静音帧占位，后续替换为 cpal/rubato 实现。

use std::cmp::max;
use std::time::Duration;

use netmic_proto::protocol::SessionParams;

/// 内部标准采样率（MVP 约定）。
pub const TARGET_SAMPLE_RATE_HZ: u32 = 48_000;
/// 内部标准声道数（MVP 固定 mono）。
pub const TARGET_CHANNELS: u16 = 1;

/// PCM16 音频帧（小端序样本）。
#[derive(Debug, Clone)]
pub struct Pcm16Frame {
    pub samples: Vec<i16>,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl Pcm16Frame {
    pub fn new(samples: Vec<i16>, sample_rate_hz: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate_hz,
            channels,
        }
    }

    /// 转成 PCM16 小端序字节，用于发送/封包。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.samples.len() * 2);
        for sample in &self.samples {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
        buf
    }
}

/// 采集/重采样阶段的最小错误类型（占位）。
#[derive(Debug)]
#[allow(dead_code)]
pub enum CaptureError {
    DeviceUnavailable(String),
    ResampleFailed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::DeviceUnavailable(msg) => write!(f, "device unavailable: {msg}"),
            CaptureError::ResampleFailed(msg) => write!(f, "resample failed: {msg}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// 采集 → 重采样 → 统一输出 的骨架管线。
pub struct AudioPipeline {
    capture: SilenceCapture,
    resampler: StubResampler,
    target_sample_rate_hz: u32,
    target_channels: u16,
    chunk_ms: u32,
}

impl AudioPipeline {
    pub fn new(params: &SessionParams) -> Self {
        Self {
            capture: SilenceCapture::new(params.sample_rate_hz, params.channels, params.chunk_ms),
            resampler: StubResampler::new(),
            target_sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
            target_channels: TARGET_CHANNELS,
            chunk_ms: params.chunk_ms,
        }
    }

    /// 产出下一帧 PCM16 mono 48k 数据（占位）。
    pub fn next_frame(&mut self) -> Result<Pcm16Frame, CaptureError> {
        let input = self.capture.next_frame()?;
        self.resampler.resample(
            input,
            self.target_sample_rate_hz,
            self.target_channels,
            self.chunk_ms,
        )
    }

    /// 发送循环建议的帧间隔。
    pub fn frame_interval(&self) -> Duration {
        Duration::from_millis(self.chunk_ms.max(1) as u64)
    }
}

/// 静音采集器（占位：后续替换为真实麦克风采集）。
struct SilenceCapture {
    source_sample_rate_hz: u32,
    channels: u16,
    chunk_ms: u32,
}

impl SilenceCapture {
    fn new(source_sample_rate_hz: u32, channels: u16, chunk_ms: u32) -> Self {
        Self {
            source_sample_rate_hz,
            channels,
            chunk_ms,
        }
    }

    fn next_frame(&mut self) -> Result<Pcm16Frame, CaptureError> {
        let samples_per_channel =
            max(1, (self.source_sample_rate_hz as u64 * self.chunk_ms as u64 / 1000) as usize);
        let total_samples = samples_per_channel * self.channels as usize;
        Ok(Pcm16Frame::new(
            vec![0; total_samples],
            self.source_sample_rate_hz,
            self.channels,
        ))
    }
}

/// 重采样占位实现：若输入非目标格式，则回退为目标格式静音帧。
struct StubResampler;

impl StubResampler {
    fn new() -> Self {
        Self
    }

    fn resample(
        &mut self,
        frame: Pcm16Frame,
        target_sample_rate_hz: u32,
        target_channels: u16,
        chunk_ms: u32,
    ) -> Result<Pcm16Frame, CaptureError> {
        if frame.sample_rate_hz == target_sample_rate_hz && frame.channels == target_channels {
            return Ok(frame);
        }

        let samples_per_channel =
            max(1, (target_sample_rate_hz as u64 * chunk_ms as u64 / 1000) as usize);
        let total_samples = samples_per_channel * target_channels as usize;
        Ok(Pcm16Frame::new(
            vec![0; total_samples],
            target_sample_rate_hz,
            target_channels,
        ))
    }
}
