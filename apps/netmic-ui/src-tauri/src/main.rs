#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use netmic_client::{list_input_devices, AudioPipeline, CaptureError};
use netmic_proto::config::normalize_session_params;
use netmic_proto::protocol::SessionParams;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

const EVENT_SNAPSHOT: &str = "netmic://snapshot";
const EVENT_WAVEFORM: &str = "netmic://waveform";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiConfig {
    server_addr: String,
    server_port: u16,
    listen_port: u16,
    input_device: String,
    codec: String,
    sample_rate_hz: u32,
    channels: u16,
    chunk_ms: u32,
    opus_bitrate_kbps: u32,
    jitter_buffer_ms: u32,
    auto_reconnect: bool,
    force_takeover: bool,
    pairing_token: String,
    virtual_mic_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiMetrics {
    rtt_ms: f32,
    packet_loss_pct: f32,
    buffer_depth_ms: f32,
    jitter_buffer_depth_ms: f32,
    estimated_e2e_latency_ms: f32,
    audio_rms: f32,
    audio_peak: u32,
    uplink_kbps: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiRuntime {
    peer_addr: Option<String>,
    connected_seconds: u64,
    reconnect_attempts: u32,
    mic_permission: String,
    virtual_mic_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiDevices {
    input: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiFallbackEvent {
    field: String,
    requested: String,
    applied: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiLogEntry {
    ts_ms: u64,
    level: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiWaveform {
    ts_ms: u64,
    points: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiSnapshot {
    mode: String,
    status: String,
    status_note: String,
    config: UiConfig,
    effective: SessionParams,
    fallbacks: Vec<UiFallbackEvent>,
    metrics: UiMetrics,
    runtime: UiRuntime,
    devices: UiDevices,
    logs: Vec<UiLogEntry>,
}

struct AppState {
    snapshot: UiSnapshot,
    streaming: bool,
    metrics_loop: bool,
    capture_loop: bool,
    connected_since: Option<Instant>,
    latest_audio_rms: f32,
    latest_audio_peak: u32,
}

type SharedState = Arc<Mutex<AppState>>;

impl Default for AppState {
    fn default() -> Self {
        let default_params = SessionParams::mvp_default();
        let snapshot = UiSnapshot {
            mode: "client".to_string(),
            status: "idle".to_string(),
            status_note: "准备就绪".to_string(),
            config: UiConfig {
                server_addr: "127.0.0.1".to_string(),
                server_port: 43000,
                listen_port: 43000,
                input_device: "系统默认".to_string(),
                codec: default_params.codec.clone(),
                sample_rate_hz: default_params.sample_rate_hz,
                channels: default_params.channels,
                chunk_ms: default_params.chunk_ms,
                opus_bitrate_kbps: default_params.opus_bitrate_kbps.unwrap_or(48),
                jitter_buffer_ms: default_params.jitter_buffer_ms,
                auto_reconnect: true,
                force_takeover: false,
                pairing_token: "".to_string(),
                virtual_mic_enabled: false,
            },
            effective: default_params,
            fallbacks: Vec::new(),
            metrics: UiMetrics {
                rtt_ms: 0.0,
                packet_loss_pct: 0.0,
                buffer_depth_ms: 0.0,
                jitter_buffer_depth_ms: 0.0,
                estimated_e2e_latency_ms: 0.0,
                audio_rms: 0.0,
                audio_peak: 0,
                uplink_kbps: 0.0,
            },
            runtime: UiRuntime {
                peer_addr: None,
                connected_seconds: 0,
                reconnect_attempts: 0,
                mic_permission: "未知".to_string(),
                virtual_mic_name: "NetMic Virtual Mic".to_string(),
            },
            devices: UiDevices {
                input: vec!["系统默认".to_string()],
            },
            logs: vec![UiLogEntry {
                ts_ms: now_ms(),
                level: "info".to_string(),
                message: "UI 已就绪（模拟状态）".to_string(),
            }],
        };

        Self {
            snapshot,
            streaming: false,
            metrics_loop: false,
            capture_loop: false,
            connected_since: None,
            latest_audio_rms: 0.0,
            latest_audio_peak: 0,
        }
    }
}

impl AppState {
    fn refresh_devices(&mut self) {
        let mut devices = vec!["系统默认".to_string()];
        match list_input_devices() {
            Ok(list) => {
                for name in list {
                    if name != "系统默认" {
                        devices.push(name);
                    }
                }
                self.snapshot.devices.input = devices;
            }
            Err(err) => {
                self.snapshot.devices.input = devices;
                self.push_log("warn", format!("输入设备枚举失败：{err}"));
            }
        }
    }

    fn push_log(&mut self, level: &str, message: impl Into<String>) {
        let entry = UiLogEntry {
            ts_ms: now_ms(),
            level: level.to_string(),
            message: message.into(),
        };
        self.snapshot.logs.insert(0, entry);
        self.snapshot.logs.truncate(200);
    }

    fn update_effective(&mut self) {
        let requested = SessionParams {
            codec: self.snapshot.config.codec.clone(),
            sample_rate_hz: self.snapshot.config.sample_rate_hz,
            channels: 1,
            chunk_ms: self.snapshot.config.chunk_ms,
            opus_bitrate_kbps: if self.snapshot.config.codec == "opus" {
                Some(self.snapshot.config.opus_bitrate_kbps)
            } else {
                None
            },
            jitter_buffer_ms: self.snapshot.config.jitter_buffer_ms,
        };
        let result = normalize_session_params(&requested);
        self.snapshot.effective = result.effective;
        self.snapshot.fallbacks = result
            .fallbacks
            .into_iter()
            .map(|item| UiFallbackEvent {
                field: item.field.to_string(),
                requested: item.requested,
                applied: item.applied,
                reason: item.reason.to_string(),
            })
            .collect();
    }

    fn reset_metrics(&mut self) {
        self.snapshot.metrics = UiMetrics {
            rtt_ms: 0.0,
            packet_loss_pct: 0.0,
            buffer_depth_ms: 0.0,
            jitter_buffer_depth_ms: 0.0,
            estimated_e2e_latency_ms: 0.0,
            audio_rms: 0.0,
            audio_peak: 0,
            uplink_kbps: 0.0,
        };
    }

    fn tick_metrics(&mut self) {
        if !self.streaming {
            return;
        }
        let since = self.connected_since.get_or_insert_with(Instant::now);
        let t = since.elapsed().as_secs_f32();
        self.snapshot.runtime.connected_seconds = t.floor() as u64;
        self.snapshot.metrics.rtt_ms = 4.0 + (t.sin().abs() * 6.0);
        self.snapshot.metrics.packet_loss_pct = (t / 2.0).cos().abs() * 1.8;
        self.snapshot.metrics.buffer_depth_ms = 80.0 + (t / 1.8).sin().abs() * 40.0;
        self.snapshot.metrics.jitter_buffer_depth_ms = 60.0 + (t / 2.6).sin().abs() * 30.0;
        self.snapshot.metrics.estimated_e2e_latency_ms =
            self.snapshot.metrics.buffer_depth_ms + self.snapshot.metrics.rtt_ms + 12.0;
        self.snapshot.metrics.audio_rms = self.latest_audio_rms;
        self.snapshot.metrics.audio_peak = self.latest_audio_peak;
        self.snapshot.metrics.uplink_kbps = if self.snapshot.config.codec == "opus" {
            self.snapshot.config.opus_bitrate_kbps as f32
        } else {
            (self.snapshot.config.sample_rate_hz as f32 * 16.0) / 1000.0
        };
    }
}

#[tauri::command]
fn get_status(state: State<SharedState>) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.refresh_devices();
    guard.snapshot.clone()
}

#[tauri::command]
fn set_mode(state: State<SharedState>, app: AppHandle, mode: String) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.snapshot.mode = if mode == "server" { "server" } else { "client" }.to_string();
        guard.snapshot.status = "idle".to_string();
        guard.snapshot.status_note = "准备就绪".to_string();
        guard.snapshot.runtime.peer_addr = None;
        guard.snapshot.runtime.connected_seconds = 0;
        guard.snapshot.runtime.mic_permission = if mode == "server" {
            "不适用".to_string()
        } else {
            "未知".to_string()
        };
        guard.streaming = false;
        guard.connected_since = None;
        guard.latest_audio_rms = 0.0;
        guard.latest_audio_peak = 0;
        guard.reset_metrics();
        guard.refresh_devices();
        let mode_label = guard.snapshot.mode.clone();
        guard.push_log("info", format!("切换到 {} 模式", mode_label));
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn set_config(state: State<SharedState>, app: AppHandle, config: UiConfig) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.snapshot.config = UiConfig { channels: 1, ..config };
        guard.update_effective();
        guard.push_log("info", "已更新配置");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn reset_defaults(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        *guard = AppState::default();
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn start(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let mode = {
        let guard = state.lock().expect("state lock");
        guard.snapshot.mode.clone()
    };

    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.streaming = true;
        guard.connected_since = None;
        guard.snapshot.runtime.connected_seconds = 0;
        if mode == "client" {
            guard.snapshot.status = "connecting".to_string();
            guard.snapshot.status_note = "正在建立连接".to_string();
        } else {
            guard.snapshot.status = "listening".to_string();
            guard.snapshot.status_note = "等待客户端连接".to_string();
        }
        guard.push_log("info", "开始运行");
        guard.snapshot.clone()
    };

    emit_snapshot(&app, &snapshot);
    ensure_metrics_loop(state.inner().clone(), app.clone());
    if mode == "client" {
        ensure_capture_loop(state.inner().clone(), app.clone());
    }

    if mode == "client" {
        let state = state.inner().clone();
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(800));
            let snapshot = {
                let mut guard = state.lock().expect("state lock");
                if !guard.streaming {
                    return;
                }
                guard.snapshot.status = "streaming".to_string();
                guard.snapshot.status_note = "推流中（模拟）".to_string();
                guard.snapshot.runtime.peer_addr = Some(format!(
                    "{}:{}",
                    guard.snapshot.config.server_addr, guard.snapshot.config.server_port
                ));
                guard.push_log("info", "已进入推流状态");
                guard.snapshot.clone()
            };
            emit_snapshot(&app, &snapshot);
        });
    }

    snapshot
}

#[tauri::command]
fn stop(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.streaming = false;
        guard.snapshot.status = "idle".to_string();
        guard.snapshot.status_note = "已停止".to_string();
        guard.snapshot.runtime.peer_addr = None;
        guard.snapshot.runtime.connected_seconds = 0;
        guard.connected_since = None;
        guard.latest_audio_rms = 0.0;
        guard.latest_audio_peak = 0;
        if guard.snapshot.mode == "client" {
            guard.snapshot.runtime.mic_permission = "未知".to_string();
        }
        guard.reset_metrics();
        guard.push_log("warn", "已停止运行");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn force_disconnect(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.snapshot.runtime.peer_addr = None;
        guard.snapshot.status = "listening".to_string();
        guard.snapshot.status_note = "已断开客户端".to_string();
        guard.push_log("warn", "已强制断开客户端");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[derive(Serialize)]
struct ExportResult {
    ok: bool,
}

#[tauri::command]
fn export_logs(state: State<SharedState>, app: AppHandle) -> ExportResult {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.push_log("info", "日志导出（模拟）");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    ExportResult { ok: true }
}

#[tauri::command]
fn clear_logs(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = {
        let mut guard = state.lock().expect("state lock");
        guard.snapshot.logs.clear();
        guard.push_log("info", "日志已清空");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

fn ensure_metrics_loop(state: SharedState, app: AppHandle) {
    let should_spawn = {
        let mut guard = state.lock().expect("state lock");
        if guard.metrics_loop {
            false
        } else {
            guard.metrics_loop = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(1000));
        let snapshot = {
            let mut guard = state.lock().expect("state lock");
            if !guard.streaming {
                guard.metrics_loop = false;
                return;
            }
            guard.tick_metrics();
            guard.snapshot.clone()
        };
        emit_snapshot(&app, &snapshot);
    });
}

fn ensure_capture_loop(state: SharedState, app: AppHandle) {
    let should_spawn = {
        let mut guard = state.lock().expect("state lock");
        if guard.capture_loop {
            false
        } else {
            guard.capture_loop = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    std::thread::spawn(move || {
        let (params, input_device) = {
            let guard = state.lock().expect("state lock");
            (
                guard.snapshot.effective.clone(),
                guard.snapshot.config.input_device.clone(),
            )
        };

        let device_name = if input_device == "系统默认" {
            None
        } else {
            Some(input_device.as_str())
        };

        let mut pipeline = match AudioPipeline::new_with_device(&params, device_name) {
            Ok(pipeline) => pipeline,
            Err(err) => {
                let snapshot = {
                    let mut guard = state.lock().expect("state lock");
                    guard.snapshot.status = "error".to_string();
                    guard.snapshot.status_note = "麦克风采集失败".to_string();
                    guard.streaming = false;
                    guard.connected_since = None;
                    guard.latest_audio_rms = 0.0;
                    guard.latest_audio_peak = 0;
                    guard.snapshot.runtime.mic_permission =
                        permission_label_from_error(&err).to_string();
                    guard.push_log("error", format!("采集初始化失败：{err}"));
                    guard.capture_loop = false;
                    guard.snapshot.clone()
                };
                emit_snapshot(&app, &snapshot);
                return;
            }
        };

        let waveform_interval = Duration::from_millis(50);
        let mut last_emit = Instant::now() - waveform_interval;

        loop {
            let (streaming, mode) = {
                let guard = state.lock().expect("state lock");
                (guard.streaming, guard.snapshot.mode.clone())
            };
            if !streaming || mode != "client" {
                let mut guard = state.lock().expect("state lock");
                guard.capture_loop = false;
                return;
            }

            let frame = match pipeline.next_frame() {
                Ok(frame) => frame,
                Err(err) => {
                    let snapshot = {
                        let mut guard = state.lock().expect("state lock");
                        guard.snapshot.status = "error".to_string();
                        guard.snapshot.status_note = "麦克风采集失败".to_string();
                        guard.streaming = false;
                        guard.connected_since = None;
                        guard.latest_audio_rms = 0.0;
                        guard.latest_audio_peak = 0;
                        guard.push_log("error", format!("采集失败：{err}"));
                        guard.capture_loop = false;
                        guard.snapshot.clone()
                    };
                    emit_snapshot(&app, &snapshot);
                    return;
                }
            };

            let (points, rms, peak) = build_waveform(&frame.samples, 128);
            {
                let mut guard = state.lock().expect("state lock");
                guard.latest_audio_rms = rms;
                guard.latest_audio_peak = peak;
                if guard.snapshot.mode == "client" {
                    guard.snapshot.runtime.mic_permission = "已授权".to_string();
                }
            }

            if last_emit.elapsed() >= waveform_interval {
                let waveform = UiWaveform {
                    ts_ms: now_ms(),
                    points,
                };
                emit_waveform(&app, &waveform);
                last_emit = Instant::now();
            }
        }
    });
}

fn emit_snapshot(app: &AppHandle, snapshot: &UiSnapshot) {
    let _ = app.emit(EVENT_SNAPSHOT, snapshot.clone());
}

fn emit_waveform(app: &AppHandle, waveform: &UiWaveform) {
    let _ = app.emit(EVENT_WAVEFORM, waveform.clone());
}

fn permission_label_from_error(err: &CaptureError) -> &'static str {
    match err {
        CaptureError::DeviceUnavailable(_) | CaptureError::DeviceListFailed(_) => "不可用",
        CaptureError::StreamConfigUnavailable(_)
        | CaptureError::StreamBuildFailed(_)
        | CaptureError::StreamPlayFailed(_) => "未授权",
        CaptureError::BufferTimeout { .. } | CaptureError::ResampleFailed(_) => "未知",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

fn build_waveform(samples: &[i16], points: usize) -> (Vec<f32>, f32, u32) {
    if samples.is_empty() || points == 0 {
        return (vec![0.0; points], 0.0, 0);
    }

    let len = samples.len();
    let step = len as f32 / points as f32;
    let mut out = Vec::with_capacity(points);
    let mut sum_sq = 0.0_f32;
    let mut peak = 0_u32;
    let denom = i16::MAX as f32;

    for sample in samples {
        let v = *sample as f32 / denom;
        sum_sq += v * v;
        let abs = (*sample as i32).unsigned_abs();
        if abs > peak {
            peak = abs;
        }
    }

    for idx in 0..points {
        let sample_index = ((idx as f32) * step) as usize;
        let sample = samples.get(sample_index).copied().unwrap_or(0);
        out.push(sample as f32 / denom);
    }

    let rms = (sum_sq / len as f32).sqrt();
    (out, rms, peak)
}

fn main() {
    let state: SharedState = Arc::new(Mutex::new(AppState::default()));
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_mode,
            set_config,
            reset_defaults,
            start,
            stop,
            force_disconnect,
            export_logs,
            clear_logs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
