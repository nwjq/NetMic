//! 客户端采集/重采样骨架（MVP：真实采集版）。
//!
//! 目标：
//! - 用 cpal 采集系统默认输入设备；
//! - 统一为 PCM16 / mono / 48k 的内部标准格式；
//! - 为后续网络发送提供稳定帧长（chunk_ms）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};
use netmic_proto::protocol::SessionParams;
use rubato::{FftFixedInOut, Resampler};
use tracing::{info, warn};

/// 内部标准采样率（MVP 约定）。
pub const TARGET_SAMPLE_RATE_HZ: u32 = 48_000;
/// 内部标准声道数（MVP 固定 mono）。
pub const TARGET_CHANNELS: u16 = 1;

/// 采集缓冲默认保留时长（秒）。
const CAPTURE_BUFFER_SECS: u32 = 2;
/// 等待采集缓冲的最大时长（毫秒）。
const BUFFER_WAIT_TIMEOUT_MS: u64 = 2_000;
/// 轮询缓冲的睡眠间隔（毫秒）。
const BUFFER_POLL_MS: u64 = 5;

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

    /// 从 f32 单声道样本构造 PCM16 帧。
    pub fn from_f32_mono(samples: Vec<f32>, sample_rate_hz: u32) -> Self {
        let pcm = samples.into_iter().map(f32_to_i16).collect();
        Self {
            samples: pcm,
            sample_rate_hz,
            channels: TARGET_CHANNELS,
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

/// 采集/重采样阶段的最小错误类型。
#[derive(Debug)]
pub enum CaptureError {
    DeviceUnavailable(String),
    StreamConfigUnavailable(String),
    StreamBuildFailed(String),
    StreamPlayFailed(String),
    BufferTimeout { wanted: usize, available: usize },
    ResampleFailed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::DeviceUnavailable(msg) => write!(f, "device unavailable: {msg}"),
            CaptureError::StreamConfigUnavailable(msg) => write!(f, "stream config failed: {msg}"),
            CaptureError::StreamBuildFailed(msg) => write!(f, "stream build failed: {msg}"),
            CaptureError::StreamPlayFailed(msg) => write!(f, "stream play failed: {msg}"),
            CaptureError::BufferTimeout { wanted, available } => write!(
                f,
                "capture buffer timeout: wanted {wanted} frames, available {available}"
            ),
            CaptureError::ResampleFailed(msg) => write!(f, "resample failed: {msg}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// 采集 → 重采样 → 统一输出 的骨架管线。
pub struct AudioPipeline {
    capture: CpalCapture,
    resampler: MonoResampler,
    target_sample_rate_hz: u32,
    target_channels: u16,
    chunk_ms: u32,
}

impl AudioPipeline {
    pub fn new(params: &SessionParams) -> Result<Self, CaptureError> {
        let capture = CpalCapture::new(params)?;
        let resampler = MonoResampler::new(
            capture.input_sample_rate_hz,
            TARGET_SAMPLE_RATE_HZ,
            params.chunk_ms,
        )?;

        Ok(Self {
            capture,
            resampler,
            target_sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
            target_channels: TARGET_CHANNELS,
            chunk_ms: params.chunk_ms,
        })
    }

    /// 产出下一帧 PCM16 mono 48k 数据。
    pub fn next_frame(&mut self) -> Result<Pcm16Frame, CaptureError> {
        let input_frames = self.resampler.required_input_frames();
        let input = self.capture.read_mono_samples(input_frames)?;
        let output = self.resampler.resample(input)?;
        Ok(Pcm16Frame::from_f32_mono(
            output,
            self.target_sample_rate_hz,
        ))
    }

    /// 发送循环建议的帧间隔。
    pub fn frame_interval(&self) -> Duration {
        Duration::from_millis(self.chunk_ms.max(1) as u64)
    }
}

/// cpal 采集器：把输入样本下混为 mono f32，写入环形缓冲。
struct CpalCapture {
    _stream: Stream,
    buffer: Arc<Mutex<SampleBuffer>>,
    input_sample_rate_hz: u32,
    input_channels: u16,
}

impl CpalCapture {
    fn new(params: &SessionParams) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| {
            CaptureError::DeviceUnavailable("no default input device".to_string())
        })?;
        let supported_config = device
            .default_input_config()
            .map_err(|err| CaptureError::StreamConfigUnavailable(err.to_string()))?;
        let sample_format = supported_config.sample_format();
        let config: StreamConfig = supported_config.into();

        let sample_rate_hz = config.sample_rate.0;
        let channels = config.channels.max(1) as u16;
        let capacity = (sample_rate_hz.saturating_mul(CAPTURE_BUFFER_SECS)) as usize;
        let buffer = Arc::new(Mutex::new(SampleBuffer::new(capacity)));

        info!(
            device = %device.name().unwrap_or_else(|_| "unknown".to_string()),
            input_sample_rate_hz = sample_rate_hz,
            input_channels = channels,
            sample_format = %sample_format,
            requested_sample_rate_hz = params.sample_rate_hz,
            "cpal input device ready"
        );

        let buffer_clone = Arc::clone(&buffer);
        let err_fn = |err| {
            warn!(%err, "cpal input stream error");
        };

        let stream = match sample_format {
            SampleFormat::F32 => {
                build_stream::<f32>(&device, &config, channels, buffer_clone, err_fn)?
            }
            SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, channels, buffer_clone, err_fn)?
            }
            SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, channels, buffer_clone, err_fn)?
            }
            other => {
                return Err(CaptureError::StreamConfigUnavailable(format!(
                    "unsupported sample format: {other}"
                )))
            }
        };

        stream
            .play()
            .map_err(|err| CaptureError::StreamPlayFailed(err.to_string()))?;

        Ok(Self {
            _stream: stream,
            buffer,
            input_sample_rate_hz: sample_rate_hz,
            input_channels: channels,
        })
    }

    fn read_mono_samples(&self, frames: usize) -> Result<Vec<f32>, CaptureError> {
        let deadline = Instant::now() + Duration::from_millis(BUFFER_WAIT_TIMEOUT_MS);
        loop {
            let mut guard = match self.buffer.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let available = guard.available();
            if available >= frames {
                return Ok(guard.pop_samples(frames));
            }
            drop(guard);
            if Instant::now() >= deadline {
                return Err(CaptureError::BufferTimeout {
                    wanted: frames,
                    available,
                });
            }
            std::thread::sleep(Duration::from_millis(BUFFER_POLL_MS));
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: u16,
    buffer: Arc<Mutex<SampleBuffer>>,
    err_fn: impl Fn(cpal::StreamError) + Send + 'static,
) -> Result<Stream, CaptureError>
where
    T: Sample,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                push_interleaved_to_mono(data, channels, &buffer);
            },
            err_fn,
            None,
        )
        .map_err(|err| CaptureError::StreamBuildFailed(err.to_string()))
}

fn push_interleaved_to_mono<T: Sample>(
    data: &[T],
    channels: u16,
    buffer: &Arc<Mutex<SampleBuffer>>,
) {
    let ch = channels.max(1) as usize;
    let mut mono = Vec::with_capacity(data.len() / ch);
    if ch == 1 {
        for sample in data {
            mono.push(sample.to_sample::<f32>());
        }
    } else {
        for frame in data.chunks_exact(ch) {
            let mut sum = 0.0_f32;
            for sample in frame {
                sum += sample.to_sample::<f32>();
            }
            mono.push(sum / ch as f32);
        }
    }

    if let Ok(mut guard) = buffer.try_lock() {
        guard.push_samples(&mono);
    }
}

/// 简单的单声道重采样包装（rubato FftFixedInOut）。
struct MonoResampler {
    inner: ResampleMode,
    target_frames: usize,
}

enum ResampleMode {
    Passthrough,
    Rubato(FftFixedInOut<f32>),
}

impl MonoResampler {
    fn new(input_rate_hz: u32, target_rate_hz: u32, chunk_ms: u32) -> Result<Self, CaptureError> {
        let target_frames = frames_for_chunk(target_rate_hz, chunk_ms);
        if input_rate_hz == target_rate_hz {
            return Ok(Self {
                inner: ResampleMode::Passthrough,
                target_frames,
            });
        }

        let desired_input_frames = frames_for_chunk(input_rate_hz, chunk_ms);
        let resampler = FftFixedInOut::<f32>::new(
            input_rate_hz as usize,
            target_rate_hz as usize,
            desired_input_frames,
            1,
        )
        .map_err(|err| CaptureError::ResampleFailed(err.to_string()))?;

        Ok(Self {
            inner: ResampleMode::Rubato(resampler),
            target_frames,
        })
    }

    fn required_input_frames(&self) -> usize {
        match &self.inner {
            ResampleMode::Passthrough => self.target_frames,
            ResampleMode::Rubato(resampler) => resampler.input_frames_next(),
        }
    }

    fn resample(&mut self, mut input: Vec<f32>) -> Result<Vec<f32>, CaptureError> {
        match &mut self.inner {
            ResampleMode::Passthrough => {
                normalize_length(&mut input, self.target_frames);
                Ok(input)
            }
            ResampleMode::Rubato(resampler) => {
                let input_frames = resampler.input_frames_next();
                if input.len() < input_frames {
                    return Err(CaptureError::ResampleFailed(format!(
                        "insufficient input frames: got {}, need {}",
                        input.len(),
                        input_frames
                    )));
                }
                input.truncate(input_frames);
                let wave_in = vec![input];
                let mut wave_out = resampler
                    .process(&wave_in, None)
                    .map_err(|err| CaptureError::ResampleFailed(err.to_string()))?;
                let mut out = wave_out.pop().unwrap_or_default();
                normalize_length(&mut out, self.target_frames);
                Ok(out)
            }
        }
    }
}

/// 采集缓冲：单声道 f32 样本。
struct SampleBuffer {
    samples: VecDeque<f32>,
    capacity: usize,
    dropped_samples: u64,
}

impl SampleBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
            dropped_samples: 0,
        }
    }

    fn available(&self) -> usize {
        self.samples.len()
    }

    fn push_samples(&mut self, input: &[f32]) {
        if self.capacity == 0 || input.is_empty() {
            return;
        }
        let incoming = input.len();
        let total = self.samples.len().saturating_add(incoming);
        if total > self.capacity {
            let overflow = total - self.capacity;
            for _ in 0..overflow {
                if self.samples.pop_front().is_some() {
                    self.dropped_samples = self.dropped_samples.saturating_add(1);
                }
            }
        }
        for sample in input {
            self.samples.push_back(*sample);
        }
    }

    fn pop_samples(&mut self, frames: usize) -> Vec<f32> {
        let take = frames.min(self.samples.len());
        let mut out = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(value) = self.samples.pop_front() {
                out.push(value);
            }
        }
        out
    }
}

fn frames_for_chunk(sample_rate_hz: u32, chunk_ms: u32) -> usize {
    let rate = sample_rate_hz as u64;
    let ms = chunk_ms.max(1) as u64;
    let frames = rate.saturating_mul(ms).saturating_div(1000);
    frames.max(1) as usize
}

fn normalize_length(samples: &mut Vec<f32>, target_frames: usize) {
    if samples.len() > target_frames {
        samples.truncate(target_frames);
    } else if samples.len() < target_frames {
        samples.resize(target_frames, 0.0);
    }
}

fn f32_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    let scaled = if clamped >= 0.0 {
        clamped * i16::MAX as f32
    } else {
        clamped * (i16::MAX as f32 + 1.0)
    };
    scaled as i16
}

#[cfg(test)]
mod tests {
    use super::{f32_to_i16, frames_for_chunk, normalize_length};

    #[test]
    fn frames_for_chunk_never_zero() {
        assert_eq!(frames_for_chunk(48_000, 0), 1);
        assert_eq!(frames_for_chunk(48_000, 20), 960);
    }

    #[test]
    fn normalize_length_truncates_or_pads() {
        let mut samples = vec![1.0_f32, 2.0, 3.0];
        normalize_length(&mut samples, 2);
        assert_eq!(samples.len(), 2);

        normalize_length(&mut samples, 5);
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[2], 0.0);
    }

    #[test]
    fn f32_to_i16_clamps() {
        assert_eq!(f32_to_i16(1.5), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), i16::MIN);
        assert_eq!(f32_to_i16(0.0), 0);
    }
}
