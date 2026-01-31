//! NetMic 客户端库导出。
//!
//! 目前仅用于复用采集/重采样管线（AudioPipeline）。

pub mod audio;

pub use audio::{
    list_input_devices, AudioPipeline, CaptureError, Pcm16Frame, TARGET_CHANNELS,
    TARGET_SAMPLE_RATE_HZ,
};
