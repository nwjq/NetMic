#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use netmic_client::{list_input_devices, AudioPipeline, CaptureError};
use netmic_proto::config::normalize_session_params;
use netmic_proto::control::{
    decode_control_message, decode_control_payload, encode_control_message,
    CONTROL_TYPE_HANDSHAKE_REQUEST, CONTROL_TYPE_HANDSHAKE_RESPONSE, CONTROL_TYPE_HEARTBEAT,
    CONTROL_TYPE_SERVER_COMMAND_REQUEST, CONTROL_TYPE_SERVER_COMMAND_RESPONSE,
    CONTROL_TYPE_SERVER_STATUS_REQUEST, CONTROL_TYPE_SERVER_STATUS_RESPONSE, CONTROL_TYPE_STATS,
};
use netmic_proto::datagram::{
    split_datagram, wrap_audio_pcm16_with_header, wrap_control_json, DatagramKind,
};
use netmic_proto::protocol::{
    AudioFrameHeader, HandshakeRequest, HandshakeResponse, Heartbeat, ServerCommandRequest,
    ServerCommandResponse, ServerStatusRequest, ServerStatusResponse, SessionParams, StatsSnapshot,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{
    path::BaseDirectory, AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, Window,
    WindowEvent, Wry,
};

const EVENT_SNAPSHOT: &str = "netmic://snapshot";
const EVENT_WAVEFORM: &str = "netmic://waveform";
const AUTOSTART_ARG: &str = "--autostart";
const SERVER_STATUS_POLL_MS: u64 = 1_000;
const ENV_SERVER_BIN: &str = "NETMIC_SERVER_BIN";
const ENV_UI_SERVER_AUTO_STOP: &str = "NETMIC_UI_SERVER_AUTO_STOP";
const ENV_CLIENT_HEARTBEAT_MS: &str = "NETMIC_CLIENT_HEARTBEAT_MS";
const ENV_HARNESS_AUTOSTART: &str = "NETMIC_UI_HARNESS_AUTOSTART";
const ENV_HARNESS_MODE: &str = "NETMIC_UI_HARNESS_MODE";
const ENV_HARNESS_SERVER_ADDR: &str = "NETMIC_UI_HARNESS_SERVER_ADDR";
const ENV_HARNESS_SERVER_PORT: &str = "NETMIC_UI_HARNESS_SERVER_PORT";
const ENV_HARNESS_INPUT_DEVICE: &str = "NETMIC_UI_HARNESS_INPUT_DEVICE";
const ENV_HARNESS_SNAPSHOT_PATH: &str = "NETMIC_UI_HARNESS_SNAPSHOT_PATH";
const ENV_HARNESS_EVENT_LOG: &str = "NETMIC_UI_HARNESS_EVENT_LOG";
const ENV_HARNESS_RENDER_LOG: &str = "NETMIC_UI_HARNESS_RENDER_LOG";
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 1_000;
const CLIENT_SERVER_TIMEOUT_MULTIPLIER: u64 = 3;
const CLIENT_RECONNECT_BASE_DELAY_MS: u64 = 1_000;
const CLIENT_RECONNECT_MAX_DELAY_MS: u64 = 4_000;
const TRAY_ID: &str = "netmic-tray";
const TRAY_MENU_TOGGLE_ID: &str = "tray-toggle-runtime";
const TRAY_MENU_OPEN_ID: &str = "tray-open-main";
const TRAY_MENU_HIDE_ID: &str = "tray-hide-main";
const TRAY_MENU_QUIT_ID: &str = "tray-quit-app";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiClientConfig {
    server_addr: String,
    server_port: u16,
    input_device: String,
    codec: String,
    sample_rate_hz: u32,
    channels: u16,
    chunk_ms: u32,
    opus_bitrate_kbps: u32,
    jitter_buffer_ms: u32,
    auto_reconnect: bool,
    pairing_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiServerConfig {
    listen_port: u16,
    force_takeover: bool,
    virtual_mic_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiAppSettings {
    launch_at_login: bool,
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
    virtual_mic_ready: bool,
    virtual_mic_error: Option<String>,
    server_status_updated_ms: u64,
    last_error: Option<String>,
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

struct ClientSender {
    socket: UdpSocket,
    server_addr: String,
    session_id: Option<String>,
    seq: u64,
    heartbeat_interval: Option<Duration>,
    next_heartbeat: Option<Instant>,
    heartbeat_seq: u64,
}

impl ClientSender {
    fn new(server_addr: String) -> Result<Self, String> {
        let socket =
            UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind udp socket failed: {err}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|err| format!("set udp nonblocking failed: {err}"))?;
        let heartbeat_interval = heartbeat_interval_from_env();
        let next_heartbeat = heartbeat_interval.map(|interval| Instant::now() + interval);
        Ok(Self {
            socket,
            server_addr,
            session_id: None,
            seq: 0,
            heartbeat_interval,
            next_heartbeat,
            heartbeat_seq: 0,
        })
    }

    fn perform_handshake(
        &mut self,
        request: &HandshakeRequest,
        timeout: Duration,
    ) -> Result<HandshakeResponse, String> {
        let payload = encode_control_message(CONTROL_TYPE_HANDSHAKE_REQUEST, request)
            .map_err(|err| format!("encode handshake request failed: {err}"))?;
        let datagram = wrap_control_json(&payload);
        self.socket
            .send_to(&datagram, &self.server_addr)
            .map_err(|err| format!("send handshake request failed: {err}"))?;

        let mut buf = [0_u8; 2048];
        let deadline = Instant::now() + timeout;
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    let (kind, payload) =
                        split_datagram(&buf[..len]).ok_or("invalid handshake datagram")?;
                    if kind != DatagramKind::ControlJson {
                        continue;
                    }
                    let (msg_type, payload_value) =
                        decode_control_message(payload).map_err(|err| format!("{err}"))?;
                    if msg_type.as_str() != CONTROL_TYPE_HANDSHAKE_RESPONSE {
                        continue;
                    }
                    let response: HandshakeResponse =
                        decode_control_payload(payload_value).map_err(|err| format!("{err}"))?;
                    if response.session_id != request.session_id {
                        continue;
                    }
                    self.session_id = Some(response.session_id.clone());
                    return Ok(response);
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("recv handshake response failed: timeout".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => return Err(format!("recv handshake response failed: {err}")),
            }
        }
    }

    fn send_frame(&mut self, frame: &netmic_client::Pcm16Frame) -> Result<(), String> {
        let session_id = self
            .session_id
            .as_ref()
            .ok_or_else(|| "handshake not completed".to_string())?;
        let payload = frame.to_bytes();
        let frame_samples = (frame.samples.len() as u32) / (frame.channels as u32).max(1);
        let header = AudioFrameHeader {
            session_id: session_id.clone(),
            seq: self.seq,
            timestamp_ms: now_ms(),
            frame_samples,
        };
        let datagram = wrap_audio_pcm16_with_header(&header, &payload)
            .map_err(|err| format!("build audio datagram failed: {err}"))?;
        self.socket
            .send_to(&datagram, &self.server_addr)
            .map_err(|err| format!("send audio datagram failed: {err}"))?;
        self.seq = self.seq.saturating_add(1);
        self.maybe_send_heartbeat()
    }

    fn maybe_send_heartbeat(&mut self) -> Result<(), String> {
        let Some(next) = self.next_heartbeat else {
            return Ok(());
        };
        if Instant::now() < next {
            return Ok(());
        }
        let session_id = self
            .session_id
            .as_ref()
            .ok_or_else(|| "handshake not completed".to_string())?;
        let heartbeat = Heartbeat {
            session_id: session_id.clone(),
            seq: self.heartbeat_seq,
            sent_at_ms: now_ms(),
        };
        let payload = encode_control_message(CONTROL_TYPE_HEARTBEAT, &heartbeat)
            .map_err(|err| format!("encode heartbeat failed: {err}"))?;
        let datagram = wrap_control_json(&payload);
        self.socket
            .send_to(&datagram, &self.server_addr)
            .map_err(|err| format!("send heartbeat failed: {err}"))?;
        self.heartbeat_seq = self.heartbeat_seq.saturating_add(1);
        self.next_heartbeat = self
            .heartbeat_interval
            .map(|interval| Instant::now() + interval);
        Ok(())
    }

    fn poll_stats(&mut self) -> Result<Option<StatsSnapshot>, String> {
        let mut buf = [0_u8; 2048];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    let Some((kind, payload)) = split_datagram(&buf[..len]) else {
                        continue;
                    };
                    if kind != DatagramKind::ControlJson {
                        continue;
                    }
                    let (msg_type, payload_value) = decode_control_message(payload)
                        .map_err(|err| format!("decode control failed: {err}"))?;
                    if msg_type.as_str() != CONTROL_TYPE_STATS {
                        continue;
                    }
                    let stats: StatsSnapshot =
                        decode_control_payload(payload_value).map_err(|err| format!("{err}"))?;
                    return Ok(Some(stats));
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(None),
                Err(err) => return Err(format!("recv control failed: {err}")),
            }
        }
    }

    fn timeout_ms(&self) -> Option<u64> {
        self.heartbeat_interval.map(|interval| {
            let base = interval.as_millis() as u64;
            let timeout = base.saturating_mul(CLIENT_SERVER_TIMEOUT_MULTIPLIER);
            timeout.max(DEFAULT_HEARTBEAT_INTERVAL_MS * 2)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiSnapshot {
    mode: String,
    status: String,
    status_note: String,
    client_config: UiClientConfig,
    server_config: UiServerConfig,
    app_settings: UiAppSettings,
    effective: SessionParams,
    fallbacks: Vec<UiFallbackEvent>,
    metrics: UiMetrics,
    runtime: UiRuntime,
    devices: UiDevices,
    logs: Vec<UiLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessVisibleState {
    active_tab: String,
    status_label: String,
    status_note: String,
    primary_action: String,
    connection_lines: Vec<String>,
    metrics_lines: Vec<String>,
    audio_lines: Vec<String>,
    params_lines: Vec<String>,
    events_lines: Vec<String>,
    config_connection_lines: Vec<String>,
    config_audio_lines: Vec<String>,
    config_client_lines: Vec<String>,
    config_server_lines: Vec<String>,
    fallback_lines: Vec<String>,
    log_filter: String,
    log_lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HarnessRenderAck {
    snapshot: UiSnapshot,
    visible: HarnessVisibleState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    mode: String,
    client_config: UiClientConfig,
    server_config: UiServerConfig,
}

struct TrayHandles {
    _tray: TrayIcon<Wry>,
    toggle_item: CheckMenuItem<Wry>,
    open_item: MenuItem<Wry>,
    hide_item: MenuItem<Wry>,
    quit_item: MenuItem<Wry>,
}

struct ExitRequested(AtomicBool);

struct AppState {
    snapshot: UiSnapshot,
    persist_path: PathBuf,
    streaming: bool,
    metrics_loop: bool,
    capture_loop: bool,
    client_handshake_loop: bool,
    server_monitor_loop: bool,
    server_process: Option<Child>,
    connected_since: Option<Instant>,
    latest_audio_rms: f32,
    latest_audio_peak: u32,
    pending_sender: Option<ClientSender>,
    last_server_seen_ms: Option<u64>,
}

type SharedState = Arc<Mutex<AppState>>;

fn default_client_config(default_params: &SessionParams) -> UiClientConfig {
    UiClientConfig {
        server_addr: "127.0.0.1".to_string(),
        server_port: 43000,
        input_device: "系统默认".to_string(),
        codec: default_params.codec.clone(),
        sample_rate_hz: default_params.sample_rate_hz,
        channels: default_params.channels,
        chunk_ms: default_params.chunk_ms,
        opus_bitrate_kbps: default_params.opus_bitrate_kbps.unwrap_or(48),
        jitter_buffer_ms: default_params.jitter_buffer_ms,
        auto_reconnect: true,
        pairing_token: "".to_string(),
    }
}

fn default_server_config() -> UiServerConfig {
    UiServerConfig {
        listen_port: 43000,
        force_takeover: false,
        virtual_mic_enabled: true,
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new_with_path(PathBuf::from("netmic-ui.json"), None)
    }
}

impl AppState {
    fn new_with_path(persist_path: PathBuf, persisted: Option<PersistedState>) -> Self {
        let default_params = SessionParams::mvp_default();
        let (client_config, server_config, mode) = if let Some(persisted) = persisted {
            (
                persisted.client_config,
                persisted.server_config,
                if persisted.mode == "server" {
                    "server".to_string()
                } else {
                    "client".to_string()
                },
            )
        } else {
            (
                default_client_config(&default_params),
                default_server_config(),
                "client".to_string(),
            )
        };

        let snapshot = UiSnapshot {
            mode,
            status: "idle".to_string(),
            status_note: "准备就绪".to_string(),
            client_config,
            server_config,
            app_settings: UiAppSettings {
                launch_at_login: false,
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
                virtual_mic_ready: false,
                virtual_mic_error: None,
                server_status_updated_ms: 0,
                last_error: None,
            },
            devices: UiDevices {
                input: vec!["系统默认".to_string()],
            },
            logs: vec![UiLogEntry {
                ts_ms: now_ms(),
                level: "info".to_string(),
                message: "UI 已就绪".to_string(),
            }],
        };
        let mut snapshot = snapshot;
        if snapshot.mode == "server" {
            snapshot.runtime.mic_permission = "不适用".to_string();
        }

        Self {
            snapshot,
            persist_path,
            streaming: false,
            metrics_loop: false,
            capture_loop: false,
            client_handshake_loop: false,
            server_monitor_loop: false,
            server_process: None,
            connected_since: None,
            latest_audio_rms: 0.0,
            latest_audio_peak: 0,
            pending_sender: None,
            last_server_seen_ms: None,
        }
    }

    fn load_or_default(app: &AppHandle) -> Self {
        let persist_path = app
            .path()
            .resolve("netmic-ui.json", BaseDirectory::AppConfig)
            .unwrap_or_else(|_| PathBuf::from("netmic-ui.json"));
        let persisted = read_persisted_state(&persist_path);
        let mut state = AppState::new_with_path(persist_path, persisted);
        state.update_effective();
        state.snapshot.app_settings.launch_at_login = system_launch_at_login_enabled(app);
        state
    }

    fn persist(&mut self) {
        let payload = PersistedState {
            mode: self.snapshot.mode.clone(),
            client_config: self.snapshot.client_config.clone(),
            server_config: self.snapshot.server_config.clone(),
        };
        if let Err(err) = write_persisted_state(&self.persist_path, &payload) {
            self.push_log("warn", format!("配置保存失败：{err}"));
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
        let config = &self.snapshot.client_config;
        let requested = SessionParams {
            codec: config.codec.clone(),
            sample_rate_hz: config.sample_rate_hz,
            channels: 1,
            chunk_ms: config.chunk_ms,
            opus_bitrate_kbps: if config.codec == "opus" {
                Some(config.opus_bitrate_kbps)
            } else {
                None
            },
            jitter_buffer_ms: config.jitter_buffer_ms,
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
        if self.last_server_seen_ms.is_some() {
            return;
        }
        self.snapshot.metrics.rtt_ms = 4.0 + (t.sin().abs() * 6.0);
        self.snapshot.metrics.packet_loss_pct = (t / 2.0).cos().abs() * 1.8;
        self.snapshot.metrics.buffer_depth_ms = 80.0 + (t / 1.8).sin().abs() * 40.0;
        self.snapshot.metrics.jitter_buffer_depth_ms = 60.0 + (t / 2.6).sin().abs() * 30.0;
        self.snapshot.metrics.estimated_e2e_latency_ms =
            self.snapshot.metrics.buffer_depth_ms + self.snapshot.metrics.rtt_ms + 12.0;
        self.snapshot.metrics.audio_rms = self.latest_audio_rms;
        self.snapshot.metrics.audio_peak = self.latest_audio_peak;
        let config = &self.snapshot.client_config;
        self.snapshot.metrics.uplink_kbps = if config.codec == "opus" {
            config.opus_bitrate_kbps as f32
        } else {
            (config.sample_rate_hz as f32 * 16.0) / 1000.0
        };
    }
}

fn read_persisted_state(path: &PathBuf) -> Option<PersistedState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_persisted_state(path: &PathBuf, state: &PersistedState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }
    let payload =
        serde_json::to_vec_pretty(state).map_err(|err| format!("配置序列化失败：{err}"))?;
    fs::write(path, payload).map_err(|err| format!("配置写入失败：{err}"))
}

fn launched_from_autostart() -> bool {
    env::args().any(|arg| arg == AUTOSTART_ARG)
}

fn is_runtime_active(snapshot: &UiSnapshot) -> bool {
    matches!(
        snapshot.status.as_str(),
        "connecting" | "streaming" | "listening" | "connected"
    )
}

fn tray_toggle_label(mode: &str) -> &'static str {
    if mode == "server" {
        "监听"
    } else {
        "推流"
    }
}

fn home_dir() -> Option<PathBuf> {
    env_trimmed("HOME").map(PathBuf::from)
}

fn escape_xml(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_desktop_exec_token(raw: &str) -> String {
    raw.chars()
        .flat_map(|ch| match ch {
            ' ' | '\t' | '\n' | '"' | '\'' | '\\' => ['\\', ch].into_iter().collect::<Vec<_>>(),
            _ => [ch].into_iter().collect(),
        })
        .collect()
}

fn launch_at_login_path(app: &AppHandle) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return home_dir().map(|home| {
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{}.plist", app.config().identifier))
        });
    }

    #[cfg(target_os = "linux")]
    {
        let config_home = env_trimmed("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".config")));
        return config_home.map(|dir| {
            dir.join("autostart")
                .join(format!("{}.desktop", app.config().identifier))
        });
    }

    #[allow(unreachable_code)]
    None
}

fn build_launch_agent_plist(label: &str, exe_path: &Path) -> String {
    let exe = escape_xml(&exe_path.to_string_lossy());
    let arg = escape_xml(AUTOSTART_ARG);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
      <string>{exe}</string>
      <string>{arg}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
  </dict>
</plist>
"#
    )
}

fn build_linux_autostart_entry(name: &str, exe_path: &Path) -> String {
    let exe = escape_desktop_exec_token(&exe_path.to_string_lossy());
    let arg = escape_desktop_exec_token(AUTOSTART_ARG);
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nExec={exe} {arg}\nTerminal=false\nHidden=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

fn system_launch_at_login_enabled(app: &AppHandle) -> bool {
    launch_at_login_path(app)
        .map(|path| path.exists())
        .unwrap_or(false)
}

fn set_system_launch_at_login(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let path = launch_at_login_path(app).ok_or_else(|| "当前平台暂不支持开机自启".to_string())?;
    let exe_path = env::current_exe().map_err(|err| format!("读取当前可执行文件失败：{err}"))?;

    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建自启动目录失败：{err}"))?;
        }
        let payload = if cfg!(target_os = "macos") {
            build_launch_agent_plist(&app.config().identifier, &exe_path)
        } else {
            build_linux_autostart_entry(&app.package_info().name, &exe_path)
        };
        fs::write(&path, payload).map_err(|err| format!("写入自启动配置失败：{err}"))?;
    } else if path.exists() {
        fs::remove_file(&path).map_err(|err| format!("移除自启动配置失败：{err}"))?;
    }

    Ok(())
}

fn ensure_main_window(app: &AppHandle) -> Option<WebviewWindow<Wry>> {
    if let Some(window) = app.get_webview_window("main") {
        return Some(window);
    }
    let config = app.config().app.windows.first()?;
    tauri::WebviewWindowBuilder::from_config(app, config)
        .ok()?
        .build()
        .ok()
}

fn open_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }
    if let Some(window) = ensure_main_window(app) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main_window_for_app(app: &AppHandle) -> bool {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
        #[cfg(target_os = "macos")]
        {
            let _ = app.hide();
        }
        return true;
    }
    false
}

fn hide_main_window(window: &Window) {
    let _ = window.hide();
    #[cfg(target_os = "macos")]
    {
        let _ = window.app_handle().hide();
    }
}

fn env_trimmed(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn truthy_env(name: &str) -> bool {
    matches!(
        env_trimmed(name).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

fn maybe_write_harness_snapshot(snapshot: &UiSnapshot) {
    if let Some(path) = env_trimmed(ENV_HARNESS_SNAPSHOT_PATH) {
        if let Ok(payload) = serde_json::to_vec_pretty(snapshot) {
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(path, payload);
        }
    }

    if let Some(path) = env_trimmed(ENV_HARNESS_EVENT_LOG) {
        append_harness_json_line(
            PathBuf::from(path),
            &serde_json::json!({
                "ts_ms": now_ms(),
                "snapshot": snapshot,
            }),
        );
    }
}

fn append_harness_json_line(path: PathBuf, payload: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(line) = serde_json::to_string(payload) {
        use std::io::Write;

        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[tauri::command]
fn report_harness_render(ack: HarnessRenderAck) -> bool {
    let Some(path) = env_trimmed(ENV_HARNESS_RENDER_LOG) else {
        return false;
    };
    append_harness_json_line(
        PathBuf::from(path),
        &serde_json::json!({
            "ts_ms": now_ms(),
            "snapshot": ack.snapshot,
            "visible": ack.visible,
        }),
    );
    true
}

fn harness_server_port() -> u16 {
    env_trimmed(ENV_HARNESS_SERVER_PORT)
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(43000)
}

fn harness_mode() -> String {
    match env_trimmed(ENV_HARNESS_MODE).as_deref() {
        Some("server") => "server".to_string(),
        _ => "client".to_string(),
    }
}

fn harness_client_config() -> UiClientConfig {
    let default = SessionParams::mvp_default();
    UiClientConfig {
        server_addr: env_trimmed(ENV_HARNESS_SERVER_ADDR)
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        server_port: harness_server_port(),
        input_device: env_trimmed(ENV_HARNESS_INPUT_DEVICE)
            .unwrap_or_else(|| "系统默认".to_string()),
        codec: default.codec,
        sample_rate_hz: default.sample_rate_hz,
        channels: default.channels,
        chunk_ms: default.chunk_ms,
        opus_bitrate_kbps: default.opus_bitrate_kbps.unwrap_or(48),
        jitter_buffer_ms: default.jitter_buffer_ms,
        auto_reconnect: true,
        pairing_token: "".to_string(),
    }
}

fn harness_server_config() -> UiServerConfig {
    let mut config = default_server_config();
    config.listen_port = harness_server_port();
    config
}

fn maybe_start_harness_autostart(app: &AppHandle) {
    if !truthy_env(ENV_HARNESS_AUTOSTART) {
        return;
    }

    let state = app.state::<SharedState>().inner().clone();
    let app = app.clone();
    std::thread::spawn(move || {
        let mode = harness_mode();
        let mode_snapshot = apply_set_mode(&state, &mode);
        emit_snapshot(&app, &mode_snapshot);

        let config_snapshot = if mode == "server" {
            apply_set_server_config(&state, harness_server_config())
        } else {
            apply_set_client_config(&state, harness_client_config())
        };
        emit_snapshot(&app, &config_snapshot);

        start_runtime(state.clone(), app.clone());
    });
}

fn stop_runtime(state: &SharedState, app: &AppHandle) -> UiSnapshot {
    let snapshot = apply_stop(state);
    emit_snapshot(app, &snapshot);
    stop_server_process_if_needed(state);
    let snapshot = { state.lock().expect("state lock").snapshot.clone() };
    emit_snapshot(app, &snapshot);
    snapshot
}

fn toggle_runtime(state: SharedState, app: AppHandle) -> UiSnapshot {
    let snapshot = { state.lock().expect("state lock").snapshot.clone() };
    if is_runtime_active(&snapshot) {
        stop_runtime(&state, &app)
    } else {
        start_runtime(state, app)
    }
}

fn quit_application(state: &SharedState, app: &AppHandle) {
    app.state::<ExitRequested>()
        .0
        .store(true, Ordering::Relaxed);
    stop_server_process_if_needed(state);
    app.exit(0);
}

fn update_tray(app: &AppHandle, snapshot: &UiSnapshot) {
    let handles = app.state::<TrayHandles>();
    let active = is_runtime_active(snapshot);
    let toggle_text = tray_toggle_label(&snapshot.mode);
    let tooltip = format!(
        "NetMic · {} · {}",
        if snapshot.mode == "server" {
            "Server"
        } else {
            "Client"
        },
        snapshot.status_note
    );
    let _ = handles.toggle_item.set_text(toggle_text);
    let _ = handles.toggle_item.set_checked(active);
    let _ = handles.open_item.set_enabled(true);
    let _ = handles.hide_item.set_enabled(true);
    let _ = handles.quit_item.set_enabled(true);
    let _ = handles._tray.set_tooltip(Some(tooltip));
}

fn load_tray_icon(_app: &AppHandle) -> Option<Image<'static>> {
    #[cfg(target_os = "macos")]
    {
        return Image::from_bytes(include_bytes!("../icons/tray-template.png")).ok();
    }

    #[cfg(not(target_os = "macos"))]
    {
        _app.default_window_icon().cloned()
    }
}

fn create_tray(app: &AppHandle) -> Result<TrayHandles, String> {
    let toggle_item =
        CheckMenuItem::with_id(app, TRAY_MENU_TOGGLE_ID, "推流", true, false, None::<&str>)
            .map_err(|err| format!("创建托盘开关失败：{err}"))?;
    let open_item = MenuItem::with_id(app, TRAY_MENU_OPEN_ID, "打开主窗口", true, None::<&str>)
        .map_err(|err| format!("创建托盘打开项失败：{err}"))?;
    let hide_item =
        MenuItem::with_id(app, TRAY_MENU_HIDE_ID, "最小化到后台", true, None::<&str>)
            .map_err(|err| format!("创建托盘隐藏项失败：{err}"))?;
    let quit_item = MenuItem::with_id(app, TRAY_MENU_QUIT_ID, "退出应用", true, None::<&str>)
        .map_err(|err| format!("创建托盘退出项失败：{err}"))?;
    let separator =
        PredefinedMenuItem::separator(app).map_err(|err| format!("创建托盘分隔符失败：{err}"))?;
    let menu = Menu::with_items(
        app,
        &[&toggle_item, &separator, &open_item, &hide_item, &quit_item],
    )
    .map_err(|err| format!("创建托盘菜单失败：{err}"))?;

    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("NetMic")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_MENU_TOGGLE_ID => {
                let state = app.state::<SharedState>().inner().clone();
                let _ = toggle_runtime(state, app.clone());
            }
            TRAY_MENU_OPEN_ID => open_main_window(app),
            TRAY_MENU_HIDE_ID => {
                let _ = hide_main_window_for_app(app);
            }
            TRAY_MENU_QUIT_ID => {
                let state = app.state::<SharedState>().inner().clone();
                quit_application(&state, app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = load_tray_icon(app) {
        tray_builder = tray_builder.icon(icon);
        #[cfg(target_os = "macos")]
        {
            tray_builder = tray_builder.icon_as_template(true);
        }
    }

    let tray = tray_builder
        .build(app)
        .map_err(|err| format!("创建托盘失败：{err}"))?;

    Ok(TrayHandles {
        _tray: tray,
        toggle_item,
        open_item,
        hide_item,
        quit_item,
    })
}

#[tauri::command]
fn get_status(state: State<SharedState>) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.refresh_devices();
    guard.snapshot.clone()
}

#[tauri::command]
fn set_mode(state: State<SharedState>, app: AppHandle, mode: String) -> UiSnapshot {
    let snapshot = apply_set_mode(state.inner(), &mode);
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn set_client_config(
    state: State<SharedState>,
    app: AppHandle,
    config: UiClientConfig,
) -> UiSnapshot {
    let snapshot = apply_set_client_config(state.inner(), config);
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn set_server_config(
    state: State<SharedState>,
    app: AppHandle,
    config: UiServerConfig,
) -> UiSnapshot {
    let snapshot = apply_set_server_config(state.inner(), config);
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn set_launch_at_login(state: State<SharedState>, app: AppHandle, enabled: bool) -> UiSnapshot {
    let snapshot = apply_set_launch_at_login(state.inner(), &app, enabled);
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn hide_to_tray(app: AppHandle) -> bool {
    hide_main_window_for_app(&app)
}

#[tauri::command]
fn reset_defaults(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = apply_reset_defaults(state.inner());
    emit_snapshot(&app, &snapshot);
    snapshot
}

fn start_runtime(state: SharedState, app: AppHandle) -> UiSnapshot {
    let mode = { state.lock().expect("state lock").snapshot.mode.clone() };
    let snapshot = apply_start(&state);

    emit_snapshot(&app, &snapshot);
    if mode == "client" {
        ensure_metrics_loop(state.clone(), app.clone());
        // 先启动本地采集，确保 macOS 触发权限提示并预览波形。
        ensure_capture_loop(state.clone(), app.clone());
        ensure_client_handshake_loop(state.clone(), app.clone());
    } else {
        if let Err(err) = ensure_server_running(&state, &app) {
            let snapshot = {
                let mut guard = state.lock().expect("state lock");
                guard.snapshot.status = "error".to_string();
                guard.snapshot.status_note = "服务端启动失败".to_string();
                guard.snapshot.runtime.last_error = Some(err.clone());
                guard.push_log("error", format!("服务端启动失败：{err}"));
                guard.snapshot.clone()
            };
            emit_snapshot(&app, &snapshot);
            return snapshot;
        }
        ensure_server_monitor_loop(state.clone(), app.clone());
        if let Some(snapshot) = refresh_server_status(&state) {
            emit_snapshot(&app, &snapshot);
        }
        let should_create = {
            let guard = state.lock().expect("state lock");
            guard.snapshot.server_config.virtual_mic_enabled
        };
        if should_create {
            let snapshot = apply_server_command(
                &state,
                &app,
                "virtual_mic_create",
                "已请求创建虚拟麦克风",
                "虚拟麦克风创建失败",
            );
            emit_snapshot(&app, &snapshot);
            if let Some(snapshot) = refresh_server_status(&state) {
                emit_snapshot(&app, &snapshot);
            }
        }
    }

    let snapshot = {
        let guard = state.lock().expect("state lock");
        guard.snapshot.clone()
    };
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn start(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    start_runtime(state.inner().clone(), app)
}

#[tauri::command]
fn stop(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    stop_runtime(state.inner(), &app)
}

#[tauri::command]
fn force_disconnect(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let mode = { state.lock().expect("state lock").snapshot.mode.clone() };
    if mode == "server" {
        let server_addr = {
            let guard = state.lock().expect("state lock");
            format!("127.0.0.1:{}", guard.snapshot.server_config.listen_port)
        };
        let result = send_server_command(&server_addr, "force_disconnect");

        let snapshot = {
            let mut guard = state.lock().expect("state lock");
            match result {
                Ok(resp) if resp.ok => {
                    guard.push_log("warn", "已向服务端发送强制断开命令");
                    guard.snapshot.runtime.peer_addr = None;
                    guard.snapshot.status = "listening".to_string();
                    guard.snapshot.status_note = "已断开客户端".to_string();
                    guard.snapshot.runtime.connected_seconds = 0;
                    guard.snapshot.runtime.last_error = None;
                    guard.snapshot.clone()
                }
                Ok(resp) => {
                    let message = resp
                        .message
                        .unwrap_or_else(|| "服务端拒绝强制断开".to_string());
                    guard.snapshot.runtime.last_error = Some(message.clone());
                    guard.push_log("error", message);
                    guard.snapshot.clone()
                }
                Err(err) => {
                    guard.snapshot.runtime.last_error = Some(format!("服务端未响应：{err}"));
                    guard.push_log("error", format!("服务端未响应（{server_addr}）：{err}"));
                    guard.snapshot.clone()
                }
            }
        };
        emit_snapshot(&app, &snapshot);
        return snapshot;
    }

    let snapshot = apply_force_disconnect(state.inner());
    emit_snapshot(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn virtual_mic_create(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = apply_server_command(
        state.inner(),
        &app,
        "virtual_mic_create",
        "已请求创建虚拟麦克风",
        "虚拟麦克风创建失败",
    );
    emit_snapshot(&app, &snapshot);
    if let Some(snapshot) = refresh_server_status(state.inner()) {
        emit_snapshot(&app, &snapshot);
        return snapshot;
    }
    snapshot
}

#[tauri::command]
fn virtual_mic_remove(state: State<SharedState>, app: AppHandle) -> UiSnapshot {
    let snapshot = apply_server_command(
        state.inner(),
        &app,
        "virtual_mic_remove",
        "已请求移除虚拟麦克风",
        "虚拟麦克风移除失败",
    );
    emit_snapshot(&app, &snapshot);
    if let Some(snapshot) = refresh_server_status(state.inner()) {
        emit_snapshot(&app, &snapshot);
        return snapshot;
    }
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

fn reconnect_backoff(attempt: u32) -> Duration {
    let factor = 1_u64.checked_shl(attempt.saturating_sub(1)).unwrap_or(8);
    let delay_ms = CLIENT_RECONNECT_BASE_DELAY_MS.saturating_mul(factor);
    Duration::from_millis(delay_ms.min(CLIENT_RECONNECT_MAX_DELAY_MS))
}

fn apply_client_runtime_failure(
    state: &SharedState,
    summary: String,
    reconnect_note: &str,
) -> (UiSnapshot, bool) {
    let mut guard = state.lock().expect("state lock");
    guard.pending_sender = None;
    guard.last_server_seen_ms = None;
    guard.connected_since = None;
    guard.snapshot.runtime.peer_addr = None;
    guard.snapshot.runtime.connected_seconds = 0;
    guard.reset_metrics();

    let should_reconnect = guard.streaming
        && guard.snapshot.mode == "client"
        && guard.snapshot.client_config.auto_reconnect;
    if should_reconnect {
        guard.snapshot.status = "connecting".to_string();
        guard.snapshot.status_note = reconnect_note.to_string();
        guard.snapshot.runtime.last_error = Some(summary.clone());
        guard.push_log("warn", summary);
    } else {
        guard.streaming = false;
        guard.capture_loop = false;
        guard.client_handshake_loop = false;
        guard.snapshot.status = "error".to_string();
        guard.snapshot.status_note = reconnect_note.to_string();
        guard.snapshot.runtime.last_error = Some(summary.clone());
        guard.push_log("error", summary);
    }
    (guard.snapshot.clone(), should_reconnect)
}

fn ensure_client_handshake_loop(state: SharedState, app: AppHandle) {
    let should_spawn = {
        let mut guard = state.lock().expect("state lock");
        if guard.client_handshake_loop {
            false
        } else {
            guard.client_handshake_loop = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    std::thread::spawn(move || {
        let mut attempt: u32 = 0;
        loop {
            let (streaming, mode, config) = {
                let guard = state.lock().expect("state lock");
                (
                    guard.streaming,
                    guard.snapshot.mode.clone(),
                    guard.snapshot.client_config.clone(),
                )
            };
            if !streaming || mode != "client" {
                let mut guard = state.lock().expect("state lock");
                guard.client_handshake_loop = false;
                return;
            }

            let server_addr = format!("{}:{}", config.server_addr, config.server_port);
            let request = build_handshake_request(&config);
            let mut sender = match ClientSender::new(server_addr.clone()) {
                Ok(sender) => sender,
                Err(err) => {
                    let (snapshot, should_reconnect) = apply_client_runtime_failure(
                        &state,
                        format!("发送初始化失败：{err}"),
                        "等待服务端重连",
                    );
                    emit_snapshot(&app, &snapshot);
                    if !should_reconnect {
                        return;
                    }
                    attempt = attempt.saturating_add(1);
                    {
                        let mut guard = state.lock().expect("state lock");
                        guard.snapshot.runtime.reconnect_attempts = attempt;
                    }
                    std::thread::sleep(reconnect_backoff(attempt));
                    continue;
                }
            };

            let response = sender.perform_handshake(&request, Duration::from_millis(1200));
            match response {
                Ok(response) if response.accepted && !response.busy => {
                    let snapshot = {
                        let mut guard = state.lock().expect("state lock");
                        if !guard.streaming || guard.snapshot.mode != "client" {
                            guard.client_handshake_loop = false;
                            return;
                        }
                        let accepted =
                            apply_handshake_outcome(&mut guard, &server_addr, Ok(response));
                        if accepted {
                            guard.pending_sender = Some(sender);
                            guard.snapshot.runtime.reconnect_attempts = 0;
                        }
                        guard.client_handshake_loop = false;
                        guard.snapshot.clone()
                    };
                    emit_snapshot(&app, &snapshot);
                    ensure_capture_loop(state.clone(), app.clone());
                    return;
                }
                Ok(response) => {
                    let reason = if response.busy {
                        "握手失败：服务端忙".to_string()
                    } else {
                        let detail = response
                            .reason
                            .clone()
                            .filter(|item| !item.trim().is_empty())
                            .unwrap_or_else(|| "被拒绝".to_string());
                        format!("握手失败：{detail}")
                    };
                    let should_reconnect = config.auto_reconnect;
                    let snapshot = {
                        let mut guard = state.lock().expect("state lock");
                        guard.pending_sender = None;
                        guard.last_server_seen_ms = None;
                        guard.connected_since = None;
                        guard.snapshot.runtime.peer_addr = None;
                        guard.snapshot.runtime.connected_seconds = 0;
                        guard.reset_metrics();
                        if should_reconnect && guard.streaming && guard.snapshot.mode == "client" {
                            attempt = attempt.saturating_add(1);
                            guard.snapshot.runtime.reconnect_attempts = attempt;
                            guard.snapshot.status = "connecting".to_string();
                            guard.snapshot.status_note = "等待服务端重连".to_string();
                            guard.snapshot.runtime.last_error = Some(reason.clone());
                            guard.push_log("warn", format!("重连第 {attempt} 次失败：{reason}"));
                            guard.snapshot.clone()
                        } else {
                            guard.client_handshake_loop = false;
                            let _ = apply_handshake_outcome(&mut guard, &server_addr, Ok(response));
                            guard.snapshot.clone()
                        }
                    };
                    emit_snapshot(&app, &snapshot);
                    if !should_reconnect {
                        return;
                    }
                }
                Err(err) => {
                    let detail = format!("握手失败：{err}");
                    let should_reconnect = config.auto_reconnect;
                    let snapshot = {
                        let mut guard = state.lock().expect("state lock");
                        guard.pending_sender = None;
                        guard.last_server_seen_ms = None;
                        guard.connected_since = None;
                        guard.snapshot.runtime.peer_addr = None;
                        guard.snapshot.runtime.connected_seconds = 0;
                        guard.reset_metrics();
                        if should_reconnect && guard.streaming && guard.snapshot.mode == "client" {
                            attempt = attempt.saturating_add(1);
                            guard.snapshot.runtime.reconnect_attempts = attempt;
                            guard.snapshot.status = "connecting".to_string();
                            guard.snapshot.status_note = "等待服务端重连".to_string();
                            guard.snapshot.runtime.last_error = Some(detail.clone());
                            guard.push_log("warn", format!("重连第 {attempt} 次失败：{detail}"));
                            guard.snapshot.clone()
                        } else {
                            guard.client_handshake_loop = false;
                            let _ = apply_handshake_outcome(&mut guard, &server_addr, Err(err));
                            guard.snapshot.clone()
                        }
                    };
                    emit_snapshot(&app, &snapshot);
                    if !should_reconnect {
                        return;
                    }
                }
            }

            std::thread::sleep(reconnect_backoff(attempt.max(1)));
        }
    });
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

fn ensure_server_monitor_loop(state: SharedState, app: AppHandle) {
    let should_spawn = {
        let mut guard = state.lock().expect("state lock");
        if guard.server_monitor_loop {
            false
        } else {
            guard.server_monitor_loop = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    std::thread::spawn(move || loop {
        let (streaming, mode) = {
            let guard = state.lock().expect("state lock");
            (guard.streaming, guard.snapshot.mode.clone())
        };

        if !streaming || mode != "server" {
            let mut guard = state.lock().expect("state lock");
            guard.server_monitor_loop = false;
            return;
        }

        if let Some(snapshot) = refresh_server_status(&state) {
            emit_snapshot(&app, &snapshot);
        }
        std::thread::sleep(Duration::from_millis(SERVER_STATUS_POLL_MS));
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
        let mut pipeline: Option<AudioPipeline> = None;
        let mut active_params: Option<SessionParams> = None;
        let mut active_device: Option<String> = None;
        let mut sender: Option<ClientSender> = None;
        let waveform_interval = Duration::from_millis(50);
        let mut last_emit = Instant::now() - waveform_interval;

        loop {
            let (streaming, mode, params, input_device, pending_sender) = {
                let mut guard = state.lock().expect("state lock");
                (
                    guard.streaming,
                    guard.snapshot.mode.clone(),
                    guard.snapshot.effective.clone(),
                    guard.snapshot.client_config.input_device.clone(),
                    guard.pending_sender.take(),
                )
            };
            if !streaming || mode != "client" {
                let mut guard = state.lock().expect("state lock");
                guard.capture_loop = false;
                return;
            }

            let should_rebuild = pipeline.is_none()
                || active_params.as_ref() != Some(&params)
                || active_device.as_deref() != Some(input_device.as_str());
            if should_rebuild {
                let device_name = if input_device == "系统默认" {
                    None
                } else {
                    Some(input_device.as_str())
                };
                match AudioPipeline::new_with_device(&params, device_name) {
                    Ok(next) => {
                        pipeline = Some(next);
                        active_params = Some(params.clone());
                        active_device = Some(input_device.clone());
                    }
                    Err(err) => {
                        let snapshot = {
                            let mut guard = state.lock().expect("state lock");
                            guard.snapshot.status = "error".to_string();
                            guard.snapshot.status_note = "麦克风采集失败".to_string();
                            guard.streaming = false;
                            guard.connected_since = None;
                            guard.latest_audio_rms = 0.0;
                            guard.latest_audio_peak = 0;
                            guard.snapshot.runtime.last_error =
                                Some(format!("采集初始化失败：{err}"));
                            guard.snapshot.runtime.mic_permission =
                                permission_label_from_error(&err).to_string();
                            guard.push_log("error", format!("采集初始化失败：{err}"));
                            guard.capture_loop = false;
                            guard.pending_sender = None;
                            guard.snapshot.clone()
                        };
                        emit_snapshot(&app, &snapshot);
                        return;
                    }
                }
            }

            if let Some(next) = pending_sender {
                let snapshot = {
                    let mut guard = state.lock().expect("state lock");
                    guard.push_log("info", "已开始发送音频数据");
                    if guard.snapshot.status != "streaming" {
                        guard.snapshot.status = "streaming".to_string();
                        guard.snapshot.status_note = "推流中（发送中）".to_string();
                    }
                    guard.last_server_seen_ms = Some(now_ms());
                    guard.snapshot.clone()
                };
                emit_snapshot(&app, &snapshot);
                sender = Some(next);
            }

            let frame = match pipeline.as_mut().expect("pipeline ready").next_frame() {
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
                        guard.snapshot.runtime.last_error = Some(format!("采集失败：{err}"));
                        guard.push_log("error", format!("采集失败：{err}"));
                        guard.capture_loop = false;
                        guard.pending_sender = None;
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

            if let Some(active_sender) = sender.as_mut() {
                if let Err(err) = active_sender.send_frame(&frame) {
                    sender = None;
                    let (snapshot, should_reconnect) = apply_client_runtime_failure(
                        &state,
                        format!("发送失败：{err}"),
                        "等待服务端重连",
                    );
                    emit_snapshot(&app, &snapshot);
                    if should_reconnect {
                        ensure_client_handshake_loop(state.clone(), app.clone());
                        continue;
                    }
                    return;
                }

                let stats = match active_sender.poll_stats() {
                    Ok(stats) => stats,
                    Err(err) => {
                        sender = None;
                        let (snapshot, should_reconnect) = apply_client_runtime_failure(
                            &state,
                            format!("服务端响应失败：{err}"),
                            "等待服务端重连",
                        );
                        emit_snapshot(&app, &snapshot);
                        if should_reconnect {
                            ensure_client_handshake_loop(state.clone(), app.clone());
                            continue;
                        }
                        return;
                    }
                };

                let now = now_ms();
                let timeout_ms = active_sender.timeout_ms();
                let (snapshot, should_emit, timed_out) = {
                    let mut guard = state.lock().expect("state lock");
                    let mut should_emit = false;
                    let mut timed_out = false;
                    if let Some(stats) = stats {
                        apply_client_stats_update(&mut guard, &stats, now);
                        should_emit = true;
                    }

                    if let (Some(timeout_ms), Some(last_seen)) =
                        (timeout_ms, guard.last_server_seen_ms)
                    {
                        if now.saturating_sub(last_seen) > timeout_ms {
                            should_emit = true;
                            timed_out = true;
                        }
                    }

                    (guard.snapshot.clone(), should_emit, timed_out)
                };
                if timed_out {
                    sender = None;
                }
                if should_emit {
                    let needs_reconnect = timed_out
                        || (snapshot.status != "streaming"
                            && snapshot.runtime.last_error.is_none());
                    if needs_reconnect {
                        let (snapshot, should_reconnect) = apply_client_runtime_failure(
                            &state,
                            "服务端无响应".to_string(),
                            "等待服务端重连",
                        );
                        emit_snapshot(&app, &snapshot);
                        if should_reconnect {
                            ensure_client_handshake_loop(state.clone(), app.clone());
                            continue;
                        }
                        return;
                    }
                    emit_snapshot(&app, &snapshot);
                }
            }
        }
    });
}

fn apply_server_status(state: &mut AppState, response: ServerStatusResponse) {
    let status = response.state.as_str();
    let (ui_status, note) = match status {
        "listening" | "idle" => ("listening", "等待客户端连接"),
        "streaming" => ("streaming", "已连接客户端"),
        "reconnecting" => ("connecting", "等待客户端重连"),
        _ => ("connecting", "服务端运行中"),
    };

    state.snapshot.status = ui_status.to_string();
    state.snapshot.status_note = note.to_string();
    state.snapshot.runtime.peer_addr = response.active_client.clone();
    state.snapshot.runtime.connected_seconds = response.active_client_seconds;
    state.snapshot.runtime.last_error = response.last_error.clone();
    state.snapshot.runtime.virtual_mic_name = response.virtual_mic_name.clone();
    state.snapshot.runtime.virtual_mic_ready = response.virtual_mic_ready;
    state.snapshot.runtime.virtual_mic_error = response.virtual_mic_error.clone();
    state.snapshot.runtime.server_status_updated_ms = now_ms();

    apply_stats_to_metrics(&response.stats, &mut state.snapshot.metrics);
}

fn apply_server_status_error(state: &mut AppState, err: String) {
    state.snapshot.status = "error".to_string();
    state.snapshot.status_note = "服务端未响应".to_string();
    state.snapshot.runtime.peer_addr = None;
    state.snapshot.runtime.connected_seconds = 0;
    state.snapshot.runtime.last_error = Some(err);
    state.snapshot.runtime.virtual_mic_ready = false;
    state.snapshot.runtime.virtual_mic_error = Some("服务端未响应".to_string());
    state.snapshot.runtime.server_status_updated_ms = now_ms();
    state.reset_metrics();
}

fn apply_stats_to_metrics(stats: &StatsSnapshot, metrics: &mut UiMetrics) {
    let loss_pct = if stats.packets_received > 0 {
        (stats.packets_lost as f32 / stats.packets_received as f32) * 100.0
    } else {
        0.0
    };
    metrics.rtt_ms = 0.0;
    metrics.packet_loss_pct = loss_pct;
    metrics.buffer_depth_ms = stats.buffer_depth_ms as f32;
    metrics.jitter_buffer_depth_ms = stats.jitter_buffer_depth_ms;
    metrics.estimated_e2e_latency_ms = stats.estimated_e2e_latency_ms;
    metrics.audio_rms = stats.audio_rms;
    metrics.audio_peak = stats.audio_peak;
    metrics.uplink_kbps = 0.0;
}

fn apply_client_stats_update(state: &mut AppState, stats: &StatsSnapshot, observed_at_ms: u64) {
    apply_stats_to_metrics(stats, &mut state.snapshot.metrics);
    state.last_server_seen_ms = Some(observed_at_ms);
    state.snapshot.runtime.server_status_updated_ms = observed_at_ms;
    if state.snapshot.status != "streaming" {
        state.snapshot.status = "streaming".to_string();
        state.snapshot.status_note = "推流中（服务端已响应）".to_string();
    }
}

fn build_handshake_request(config: &UiClientConfig) -> HandshakeRequest {
    let requested = SessionParams {
        codec: config.codec.clone(),
        sample_rate_hz: config.sample_rate_hz,
        channels: config.channels,
        chunk_ms: config.chunk_ms,
        opus_bitrate_kbps: if config.codec == "opus" {
            Some(config.opus_bitrate_kbps)
        } else {
            None
        },
        jitter_buffer_ms: config.jitter_buffer_ms,
    };
    HandshakeRequest {
        session_id: format!("session-{}", now_ms()),
        client_name: "netmic-ui".to_string(),
        requested,
        token: if config.pairing_token.trim().is_empty() {
            None
        } else {
            Some(config.pairing_token.clone())
        },
    }
}

fn send_server_status_request(server_addr: &str) -> Result<ServerStatusResponse, String> {
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind udp socket failed: {err}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(800)))
        .map_err(|err| format!("set udp timeout failed: {err}"))?;

    let request = ServerStatusRequest {
        request_id: format!("status-{}", now_ms()),
    };
    let payload = encode_control_message(CONTROL_TYPE_SERVER_STATUS_REQUEST, &request)
        .map_err(|err| format!("encode status request failed: {err}"))?;
    let datagram = wrap_control_json(&payload);
    socket
        .send_to(&datagram, server_addr)
        .map_err(|err| format!("send status request failed: {err}"))?;

    let mut buf = [0_u8; 2048];
    let (len, _addr) = socket
        .recv_from(&mut buf)
        .map_err(|err| format!("recv status response failed: {err}"))?;
    let (kind, payload) = split_datagram(&buf[..len]).ok_or("invalid status datagram")?;
    if kind != DatagramKind::ControlJson {
        return Err("unexpected status response kind".to_string());
    }
    let (msg_type, payload_value) =
        decode_control_message(payload).map_err(|err| format!("{err}"))?;
    if msg_type.as_str() != CONTROL_TYPE_SERVER_STATUS_RESPONSE {
        return Err(format!("unexpected status response type: {msg_type}"));
    }
    let response: ServerStatusResponse =
        decode_control_payload(payload_value).map_err(|err| format!("{err}"))?;
    if response.request_id != request.request_id {
        return Err("status response request_id mismatch".to_string());
    }
    Ok(response)
}

fn send_server_command(server_addr: &str, action: &str) -> Result<ServerCommandResponse, String> {
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind udp socket failed: {err}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(800)))
        .map_err(|err| format!("set udp timeout failed: {err}"))?;

    let request = ServerCommandRequest {
        request_id: format!("cmd-{}", now_ms()),
        action: action.to_string(),
    };
    let payload = encode_control_message(CONTROL_TYPE_SERVER_COMMAND_REQUEST, &request)
        .map_err(|err| format!("encode server command failed: {err}"))?;
    let datagram = wrap_control_json(&payload);
    socket
        .send_to(&datagram, server_addr)
        .map_err(|err| format!("send server command failed: {err}"))?;

    let mut buf = [0_u8; 2048];
    let (len, _addr) = socket
        .recv_from(&mut buf)
        .map_err(|err| format!("recv server command response failed: {err}"))?;
    let (kind, payload) = split_datagram(&buf[..len]).ok_or("invalid command datagram")?;
    if kind != DatagramKind::ControlJson {
        return Err("unexpected command response kind".to_string());
    }
    let (msg_type, payload_value) =
        decode_control_message(payload).map_err(|err| format!("{err}"))?;
    if msg_type.as_str() != CONTROL_TYPE_SERVER_COMMAND_RESPONSE {
        return Err(format!("unexpected command response type: {msg_type}"));
    }
    let response: ServerCommandResponse =
        decode_control_payload(payload_value).map_err(|err| format!("{err}"))?;
    if response.request_id != request.request_id {
        return Err("command response request_id mismatch".to_string());
    }
    Ok(response)
}

fn refresh_server_status(state: &SharedState) -> Option<UiSnapshot> {
    let server_addr = {
        let guard = state.lock().expect("state lock");
        format!("127.0.0.1:{}", guard.snapshot.server_config.listen_port)
    };
    let response = send_server_status_request(&server_addr);
    let mut guard = state.lock().expect("state lock");
    let prev_error = guard.snapshot.runtime.last_error.clone();
    let prev_virtual_mic_ready = guard.snapshot.runtime.virtual_mic_ready;
    let prev_virtual_mic_error = guard.snapshot.runtime.virtual_mic_error.clone();
    let prev_virtual_mic_name = guard.snapshot.runtime.virtual_mic_name.clone();
    let first_fetch = guard.snapshot.runtime.server_status_updated_ms == 0;
    match response {
        Ok(status) => {
            let should_log_status = first_fetch
                || status.virtual_mic_ready != prev_virtual_mic_ready
                || status.virtual_mic_error != prev_virtual_mic_error
                || status.virtual_mic_name != prev_virtual_mic_name;
            if should_log_status {
                let error_note = status
                    .virtual_mic_error
                    .clone()
                    .unwrap_or_else(|| "无".to_string());
                let message = format!(
                    "服务端状态：虚拟麦={}（{}），错误={}",
                    if status.virtual_mic_ready {
                        "已就绪"
                    } else {
                        "未就绪"
                    },
                    status.virtual_mic_name,
                    error_note
                );
                let level = if status.virtual_mic_ready {
                    "info"
                } else {
                    "warn"
                };
                guard.push_log(level, message.clone());
                eprintln!("[netmic-ui] {message}");
            }
            apply_server_status(&mut guard, status);
            if first_fetch {
                let message = format!("已拉取服务端状态（{server_addr}）");
                guard.push_log("info", message.clone());
                eprintln!("[netmic-ui] {message}");
            } else if prev_error.is_some() {
                let message = format!("服务端状态拉取已恢复（{server_addr}）");
                guard.push_log("info", message.clone());
            }
        }
        Err(err) => {
            let should_log = prev_error.as_deref() != Some(&err);
            apply_server_status_error(&mut guard, err.clone());
            if should_log {
                let message = format!("服务端状态拉取失败（{server_addr}）：{err}");
                guard.push_log("error", message.clone());
                eprintln!("[netmic-ui] {message}");
            }
        }
    }
    Some(guard.snapshot.clone())
}

fn ensure_server_running(state: &SharedState, app: &AppHandle) -> Result<(), String> {
    let listen_port = {
        let guard = state.lock().expect("state lock");
        guard.snapshot.server_config.listen_port
    };
    let server_addr = format!("127.0.0.1:{listen_port}");
    ensure_single_server_instance(state);
    if send_server_status_request(&server_addr).is_ok() {
        return Ok(());
    }

    let maybe_child = {
        let mut guard = state.lock().expect("state lock");
        if let Some(child) = guard.server_process.as_mut() {
            if let Ok(Some(_status)) = child.try_wait() {
                guard.server_process = None;
            } else {
                guard.push_log("info", "服务端已在运行中（由 UI 启动）");
                return Ok(());
            }
        }
        match spawn_server_process(listen_port, app) {
            Ok(child) => {
                guard.push_log("info", "已启动服务端进程");
                Some(child)
            }
            Err(err) => {
                return Err(err);
            }
        }
    };

    if let Some(child) = maybe_child {
        let mut guard = state.lock().expect("state lock");
        guard.server_process = Some(child);
    }

    std::thread::sleep(Duration::from_millis(200));
    if send_server_status_request(&server_addr).is_err() {
        let mut guard = state.lock().expect("state lock");
        guard.push_log("warn", "服务端启动中，状态暂未就绪");
    }
    Ok(())
}

fn ensure_single_server_instance(state: &SharedState) {
    let mut guard = state.lock().expect("state lock");
    guard.server_process = None;
    drop(guard);

    #[cfg(target_family = "unix")]
    {
        let status = Command::new("pkill")
            .arg("-f")
            .arg("netmic-server")
            .status();
        let mut guard = state.lock().expect("state lock");
        match status {
            Ok(status) if status.success() => {
                guard.push_log("warn", "已终止其他服务端进程（确保单实例）");
            }
            Ok(_status) => {
                guard.push_log("info", "未发现其他服务端进程");
            }
            Err(err) => {
                guard.push_log("warn", format!("终止其他服务端进程失败：{err}"));
            }
        }
    }
}

fn should_auto_stop_server() -> bool {
    match env::var(ENV_UI_SERVER_AUTO_STOP) {
        Ok(raw) => !matches!(raw.as_str(), "0" | "false" | "off" | "no"),
        Err(_) => true,
    }
}

fn stop_server_process_if_needed(state: &SharedState) {
    let should_stop = should_auto_stop_server();
    let child = {
        let mut guard = state.lock().expect("state lock");
        if guard.snapshot.mode != "server" {
            return;
        }
        if guard.server_process.is_none() {
            return;
        }
        if !should_stop {
            guard.push_log("info", "已保留服务端进程运行");
            return;
        }
        guard.server_process.take()
    };

    if let Some(mut child) = child {
        if let Err(err) = child.kill() {
            let mut guard = state.lock().expect("state lock");
            guard.push_log("warn", format!("停止服务端进程失败：{err}"));
            return;
        }
        let _ = child.wait();
        let mut guard = state.lock().expect("state lock");
        guard.push_log("info", "已停止服务端进程");
    }
}

fn spawn_server_process(listen_port: u16, app: &AppHandle) -> Result<Child, String> {
    let bin = resolve_server_binary(app)?;
    let mut cmd = Command::new(&bin);
    cmd.env("NETMIC_SERVER_UDP_PORT", listen_port.to_string())
        .env("NETMIC_SERVER_VIRTUAL_MIC_AUTO_CREATE", "1");
    cmd.spawn()
        .map_err(|err| format!("启动服务端失败（{}）：{err}", bin.display()))
}

fn resolve_server_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(raw) = env::var(ENV_SERVER_BIN) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(current) = env::current_exe() {
        if let Some(dir) = current.parent() {
            let name = if cfg!(windows) {
                "netmic-server.exe"
            } else {
                "netmic-server"
            };
            let candidate = dir.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("netmic-server");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Ok(PathBuf::from("netmic-server"))
}

fn apply_handshake_outcome(
    state: &mut AppState,
    server_addr: &str,
    response: Result<HandshakeResponse, String>,
) -> bool {
    match response {
        Ok(response) if response.accepted && !response.busy => {
            state.snapshot.effective = response.effective;
            state.snapshot.status = "streaming".to_string();
            state.snapshot.status_note = "推流中（握手成功）".to_string();
            state.snapshot.runtime.peer_addr = Some(server_addr.to_string());
            state.snapshot.runtime.reconnect_attempts = 0;
            state.snapshot.runtime.last_error = None;
            state.snapshot.runtime.server_status_updated_ms = now_ms();
            state.pending_sender = None;
            state.last_server_seen_ms = None;
            state.push_log("info", "握手成功，已进入推流状态");
            true
        }
        Ok(response) if response.busy => {
            state.streaming = false;
            state.pending_sender = None;
            state.last_server_seen_ms = None;
            state.snapshot.status = "error".to_string();
            state.snapshot.status_note = "握手失败：服务端忙".to_string();
            state.snapshot.runtime.peer_addr = None;
            state.snapshot.runtime.connected_seconds = 0;
            state.snapshot.runtime.last_error = Some("握手失败：服务端忙".to_string());
            state.connected_since = None;
            state.reset_metrics();
            state.push_log("warn", "握手失败：服务端忙");
            false
        }
        Ok(response) => {
            state.streaming = false;
            state.pending_sender = None;
            state.last_server_seen_ms = None;
            state.snapshot.status = "error".to_string();
            let reason = response
                .reason
                .filter(|item| !item.trim().is_empty())
                .unwrap_or_else(|| "被拒绝".to_string());
            state.snapshot.status_note = format!("握手失败：{reason}");
            state.snapshot.runtime.peer_addr = None;
            state.snapshot.runtime.connected_seconds = 0;
            state.snapshot.runtime.last_error = Some(format!("握手失败：{reason}"));
            state.connected_since = None;
            state.reset_metrics();
            state.push_log("error", format!("握手失败：{reason}"));
            false
        }
        Err(err) => {
            state.streaming = false;
            state.pending_sender = None;
            state.last_server_seen_ms = None;
            state.snapshot.status = "error".to_string();
            state.snapshot.status_note = format!("握手失败：{err}");
            state.snapshot.runtime.peer_addr = None;
            state.snapshot.runtime.connected_seconds = 0;
            state.snapshot.runtime.last_error = Some(format!("握手失败：{err}"));
            state.connected_since = None;
            state.reset_metrics();
            state.push_log("error", format!("握手失败：{err}"));
            false
        }
    }
}

fn apply_set_mode(state: &SharedState, mode: &str) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.snapshot.mode = if mode == "server" { "server" } else { "client" }.to_string();
    guard.snapshot.status = "idle".to_string();
    guard.snapshot.status_note = "准备就绪".to_string();
    guard.snapshot.runtime.peer_addr = None;
    guard.snapshot.runtime.connected_seconds = 0;
    guard.pending_sender = None;
    guard.last_server_seen_ms = None;
    guard.snapshot.runtime.mic_permission = if mode == "server" {
        "不适用".to_string()
    } else {
        "未知".to_string()
    };
    guard.snapshot.runtime.last_error = None;
    guard.snapshot.runtime.virtual_mic_ready = false;
    guard.snapshot.runtime.virtual_mic_error = None;
    guard.streaming = false;
    guard.connected_since = None;
    guard.client_handshake_loop = false;
    guard.latest_audio_rms = 0.0;
    guard.latest_audio_peak = 0;
    guard.reset_metrics();
    guard.refresh_devices();
    guard.update_effective();
    let mode_label = guard.snapshot.mode.clone();
    guard.push_log("info", format!("切换到 {} 模式", mode_label));
    guard.persist();
    guard.snapshot.clone()
}

fn apply_set_client_config(state: &SharedState, config: UiClientConfig) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.snapshot.client_config = UiClientConfig {
        channels: 1,
        ..config
    };
    guard.update_effective();
    guard.push_log("info", "已更新客户端配置");
    guard.persist();
    guard.snapshot.clone()
}

fn apply_set_server_config(state: &SharedState, config: UiServerConfig) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.snapshot.server_config = config;
    guard.push_log("info", "已更新服务端配置");
    guard.persist();
    guard.snapshot.clone()
}

fn apply_set_launch_at_login(state: &SharedState, app: &AppHandle, enabled: bool) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    match set_system_launch_at_login(app, enabled) {
        Ok(()) => {
            guard.snapshot.app_settings.launch_at_login = system_launch_at_login_enabled(app);
            guard.snapshot.runtime.last_error = None;
            let enabled_now = guard.snapshot.app_settings.launch_at_login;
            guard.push_log(
                "info",
                if enabled_now {
                    "已开启开机自启"
                } else {
                    "已关闭开机自启"
                },
            );
        }
        Err(err) => {
            guard.snapshot.runtime.last_error = Some(err.clone());
            guard.push_log("error", format!("切换开机自启失败：{err}"));
        }
    }
    guard.snapshot.clone()
}

fn apply_reset_defaults(state: &SharedState) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    let default_params = SessionParams::mvp_default();
    guard.snapshot.mode = "client".to_string();
    guard.snapshot.status = "idle".to_string();
    guard.snapshot.status_note = "准备就绪".to_string();
    guard.snapshot.client_config = default_client_config(&default_params);
    guard.snapshot.server_config = default_server_config();
    guard.snapshot.effective = default_params;
    guard.snapshot.fallbacks.clear();
    guard.snapshot.runtime.peer_addr = None;
    guard.snapshot.runtime.connected_seconds = 0;
    guard.snapshot.runtime.mic_permission = "未知".to_string();
    guard.snapshot.runtime.last_error = None;
    guard.snapshot.runtime.virtual_mic_ready = false;
    guard.snapshot.runtime.virtual_mic_error = None;
    guard.streaming = false;
    guard.metrics_loop = false;
    guard.capture_loop = false;
    guard.client_handshake_loop = false;
    guard.server_monitor_loop = false;
    guard.pending_sender = None;
    guard.last_server_seen_ms = None;
    guard.connected_since = None;
    guard.latest_audio_rms = 0.0;
    guard.latest_audio_peak = 0;
    guard.reset_metrics();
    guard.refresh_devices();
    guard.snapshot.logs.clear();
    guard.push_log("info", "已恢复默认配置");
    guard.persist();
    guard.snapshot.clone()
}

fn apply_start(state: &SharedState) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    let mode = guard.snapshot.mode.clone();
    guard.streaming = true;
    guard.pending_sender = None;
    guard.last_server_seen_ms = None;
    guard.connected_since = None;
    guard.snapshot.runtime.connected_seconds = 0;
    guard.snapshot.runtime.reconnect_attempts = 0;
    guard.snapshot.runtime.last_error = None;
    if mode == "client" {
        guard.snapshot.status = "connecting".to_string();
        guard.snapshot.status_note = "正在建立连接".to_string();
    } else {
        guard.snapshot.status = "listening".to_string();
        guard.snapshot.status_note = "等待客户端连接".to_string();
    }
    guard.push_log("info", "开始运行");
    guard.snapshot.clone()
}

fn apply_stop(state: &SharedState) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.streaming = false;
    guard.pending_sender = None;
    guard.last_server_seen_ms = None;
    guard.client_handshake_loop = false;
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
    guard.snapshot.runtime.last_error = None;
    guard.snapshot.runtime.virtual_mic_ready = false;
    guard.snapshot.runtime.virtual_mic_error = None;
    guard.reset_metrics();
    guard.push_log("warn", "已停止运行");
    guard.snapshot.clone()
}

fn apply_force_disconnect(state: &SharedState) -> UiSnapshot {
    let mut guard = state.lock().expect("state lock");
    guard.snapshot.runtime.peer_addr = None;
    guard.snapshot.status = "listening".to_string();
    guard.snapshot.status_note = "已断开客户端".to_string();
    guard.push_log("warn", "已强制断开客户端");
    guard.snapshot.clone()
}

fn apply_server_command(
    state: &SharedState,
    app: &AppHandle,
    action: &str,
    ok_message: &str,
    fail_prefix: &str,
) -> UiSnapshot {
    let mode = { state.lock().expect("state lock").snapshot.mode.clone() };
    if mode != "server" {
        let mut guard = state.lock().expect("state lock");
        guard.push_log("warn", "仅服务端模式支持该操作");
        return guard.snapshot.clone();
    }
    if let Err(err) = ensure_server_running(state, app) {
        let mut guard = state.lock().expect("state lock");
        guard.snapshot.runtime.last_error = Some(err.clone());
        guard.push_log("error", format!("服务端启动失败：{err}"));
        return guard.snapshot.clone();
    }

    let server_addr = {
        let guard = state.lock().expect("state lock");
        format!("127.0.0.1:{}", guard.snapshot.server_config.listen_port)
    };
    let result = send_server_command(&server_addr, action);

    let mut guard = state.lock().expect("state lock");
    match result {
        Ok(resp) if resp.ok => {
            guard.snapshot.runtime.last_error = None;
            guard.push_log("info", ok_message);
        }
        Ok(resp) => {
            let message = resp.message.unwrap_or_else(|| format!("{fail_prefix}"));
            guard.snapshot.runtime.last_error = Some(message.clone());
            guard.push_log("error", message);
        }
        Err(err) => {
            guard.snapshot.runtime.last_error = Some(format!("服务端未响应：{err}"));
            guard.push_log("error", format!("服务端未响应（{server_addr}）：{err}"));
        }
    }
    guard.snapshot.clone()
}

fn emit_snapshot(app: &AppHandle, snapshot: &UiSnapshot) {
    let _ = app.emit(EVENT_SNAPSHOT, snapshot.clone());
    update_tray(app, snapshot);
    maybe_write_harness_snapshot(snapshot);
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

fn heartbeat_interval_from_env() -> Option<Duration> {
    match env::var(ENV_CLIENT_HEARTBEAT_MS) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(0) => None,
            Ok(ms) => Some(Duration::from_millis(ms)),
            Err(_) => Some(Duration::from_millis(DEFAULT_HEARTBEAT_INTERVAL_MS)),
        },
        Err(_) => Some(Duration::from_millis(DEFAULT_HEARTBEAT_INTERVAL_MS)),
    }
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
    let app = tauri::Builder::default()
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let exit_requested = window
                    .app_handle()
                    .try_state::<ExitRequested>()
                    .map(|state| state.0.load(Ordering::Relaxed))
                    .unwrap_or(false);
                if !exit_requested {
                    api.prevent_close();
                    hide_main_window(window);
                }
            }
        })
        .setup(|app| {
            let state = AppState::load_or_default(app.handle());
            app.manage(Arc::new(Mutex::new(state)));
            app.manage(ExitRequested(AtomicBool::new(false)));
            let tray = create_tray(app.handle())?;
            app.manage(tray);
            let snapshot = {
                app.state::<SharedState>()
                    .lock()
                    .expect("state lock")
                    .snapshot
                    .clone()
            };
            update_tray(app.handle(), &snapshot);
            if launched_from_autostart() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    #[cfg(target_os = "macos")]
                    {
                        let _ = app.hide();
                    }
                }
            }
            maybe_start_harness_autostart(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_mode,
            set_client_config,
            set_server_config,
            set_launch_at_login,
            hide_to_tray,
            reset_defaults,
            start,
            stop,
            force_disconnect,
            virtual_mic_create,
            virtual_mic_remove,
            export_logs,
            clear_logs,
            report_harness_render
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let exit_requested = app
                .try_state::<ExitRequested>()
                .map(|state| state.0.load(Ordering::Relaxed))
                .unwrap_or(false);
            if !exit_requested {
                api.prevent_exit();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                    #[cfg(target_os = "macos")]
                    {
                        let _ = app.hide();
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn with_env_var<T>(key: &str, value: Option<&str>, run: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().expect("env lock");
        let previous = std::env::var_os(key);
        match value {
            Some(next) => std::env::set_var(key, next),
            None => std::env::remove_var(key),
        }
        let result = run();
        match previous {
            Some(prev) => std::env::set_var(key, prev),
            None => std::env::remove_var(key),
        }
        result
    }

    fn fresh_state() -> SharedState {
        Arc::new(Mutex::new(AppState::new_with_path(
            PathBuf::from("/tmp/netmic-ui-test.json"),
            None,
        )))
    }

    #[test]
    fn ipc_set_mode_resets_state_and_permissions() {
        let state = fresh_state();
        {
            let mut guard = state.lock().expect("state lock");
            guard.snapshot.runtime.peer_addr = Some("127.0.0.1:1".to_string());
            guard.snapshot.runtime.connected_seconds = 42;
            guard.streaming = true;
        }
        let snapshot = apply_set_mode(&state, "server");
        assert_eq!(snapshot.mode, "server");
        assert_eq!(snapshot.status, "idle");
        assert_eq!(snapshot.status_note, "准备就绪");
        assert_eq!(snapshot.runtime.peer_addr, None);
        assert_eq!(snapshot.runtime.connected_seconds, 0);
        assert_eq!(snapshot.runtime.mic_permission, "不适用");

        let snapshot = apply_set_mode(&state, "client");
        assert_eq!(snapshot.mode, "client");
        assert_eq!(snapshot.runtime.mic_permission, "未知");
    }

    #[test]
    fn ipc_set_client_config_updates_effective_and_fallbacks() {
        let state = fresh_state();
        let config = UiClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port: 43000,
            input_device: "系统默认".to_string(),
            codec: "opus".to_string(),
            sample_rate_hz: 12_345,
            channels: 2,
            chunk_ms: 15,
            opus_bitrate_kbps: 999,
            jitter_buffer_ms: 999,
            auto_reconnect: true,
            pairing_token: "".to_string(),
        };
        let snapshot = apply_set_client_config(&state, config);
        assert!(snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "sample_rate_hz"));
        assert!(snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "chunk_ms"));
        assert!(snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "opus_bitrate_kbps"));
        assert!(snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "jitter_buffer_ms"));
        assert_eq!(snapshot.effective.channels, 1);
    }

    #[test]
    fn ipc_start_sets_status_by_mode() {
        let state = fresh_state();
        let snapshot = apply_set_mode(&state, "client");
        assert_eq!(snapshot.mode, "client");
        let snapshot = apply_start(&state);
        assert_eq!(snapshot.status, "connecting");
        assert_eq!(snapshot.status_note, "正在建立连接");

        let _snapshot = apply_set_mode(&state, "server");
        let snapshot = apply_start(&state);
        assert_eq!(snapshot.status, "listening");
        assert_eq!(snapshot.status_note, "等待客户端连接");
    }

    #[test]
    fn ipc_stop_clears_runtime_fields() {
        let state = fresh_state();
        let _ = apply_set_mode(&state, "client");
        let _ = apply_start(&state);
        {
            let mut guard = state.lock().expect("state lock");
            guard.snapshot.runtime.peer_addr = Some("127.0.0.1:9".to_string());
            guard.snapshot.runtime.connected_seconds = 9;
            guard.latest_audio_rms = 0.9;
            guard.latest_audio_peak = 123;
        }
        let snapshot = apply_stop(&state);
        assert_eq!(snapshot.status, "idle");
        assert_eq!(snapshot.runtime.peer_addr, None);
        assert_eq!(snapshot.runtime.connected_seconds, 0);
        assert_eq!(snapshot.runtime.mic_permission, "未知");
    }

    #[test]
    fn ipc_force_disconnect_marks_listening() {
        let state = fresh_state();
        let _ = apply_set_mode(&state, "server");
        {
            let mut guard = state.lock().expect("state lock");
            guard.snapshot.runtime.peer_addr = Some("127.0.0.1:9".to_string());
        }
        let snapshot = apply_force_disconnect(&state);
        assert_eq!(snapshot.status, "listening");
        assert_eq!(snapshot.status_note, "已断开客户端");
        assert_eq!(snapshot.runtime.peer_addr, None);
    }

    #[test]
    fn harness_mode_defaults_to_client() {
        with_env_var(ENV_HARNESS_MODE, None, || {
            assert_eq!(harness_mode(), "client");
        });
    }

    #[test]
    fn harness_mode_accepts_server() {
        with_env_var(ENV_HARNESS_MODE, Some("server"), || {
            assert_eq!(harness_mode(), "server");
        });
    }

    #[test]
    fn harness_server_config_uses_harness_port() {
        with_env_var(ENV_HARNESS_SERVER_PORT, Some("43123"), || {
            let config = harness_server_config();

            assert_eq!(config.listen_port, 43123);
            assert!(config.virtual_mic_enabled);
            assert!(!config.force_takeover);
        });
    }

    #[test]
    fn build_waveform_returns_fixed_length_for_empty_input() {
        let (points, rms, peak) = build_waveform(&[], 128);
        assert_eq!(points.len(), 128);
        assert!(points.iter().all(|value| *value == 0.0));
        assert_eq!(rms, 0.0);
        assert_eq!(peak, 0);
    }

    #[test]
    fn build_waveform_reports_rms_and_peak() {
        let samples = vec![0_i16, i16::MAX];
        let (points, rms, peak) = build_waveform(&samples, 2);
        assert_eq!(points.len(), 2);
        assert_eq!(peak, i16::MAX as u32);
        assert!((rms - 0.707).abs() < 0.02);
    }

    #[test]
    fn launch_agent_plist_contains_autostart_exec() {
        let payload = build_launch_agent_plist(
            "io.netmic.app",
            Path::new("/Applications/NetMic.app/Contents/MacOS/netmic-ui"),
        );

        assert!(payload.contains("<string>io.netmic.app</string>"));
        assert!(
            payload.contains("<string>/Applications/NetMic.app/Contents/MacOS/netmic-ui</string>")
        );
        assert!(payload.contains("<string>--autostart</string>"));
        assert!(payload.contains("<key>RunAtLoad</key>"));
    }

    #[test]
    fn linux_autostart_entry_contains_exec_and_flag() {
        let payload =
            build_linux_autostart_entry("NetMic", Path::new("/opt/NetMic Bundle/netmic-ui"));

        assert!(payload.contains("Name=NetMic"));
        assert!(payload.contains("Exec=/opt/NetMic\\ Bundle/netmic-ui --autostart"));
        assert!(payload.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn tray_toggle_label_matches_mode() {
        assert_eq!(tray_toggle_label("server"), "监听");
        assert_eq!(tray_toggle_label("client"), "推流");
    }

    #[test]
    fn permission_label_maps_errors() {
        assert_eq!(
            permission_label_from_error(&CaptureError::DeviceUnavailable("x".into())),
            "不可用"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::DeviceListFailed("x".into())),
            "不可用"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::StreamConfigUnavailable("x".into())),
            "未授权"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::StreamBuildFailed("x".into())),
            "未授权"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::StreamPlayFailed("x".into())),
            "未授权"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::BufferTimeout {
                wanted: 1,
                available: 0
            }),
            "未知"
        );
        assert_eq!(
            permission_label_from_error(&CaptureError::ResampleFailed("x".into())),
            "未知"
        );
    }

    #[test]
    fn update_effective_records_fallbacks() {
        let mut state = AppState::new_with_path(PathBuf::from("/tmp/netmic-ui-test.json"), None);
        state.snapshot.client_config.codec = "opus".to_string();
        state.snapshot.client_config.sample_rate_hz = 12_345;
        state.snapshot.client_config.chunk_ms = 15;
        state.snapshot.client_config.opus_bitrate_kbps = 999;
        state.snapshot.client_config.jitter_buffer_ms = 999;

        state.update_effective();

        assert!(state
            .snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "sample_rate_hz"));
        assert!(state
            .snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "chunk_ms"));
        assert!(state
            .snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "opus_bitrate_kbps"));
        assert!(state
            .snapshot
            .fallbacks
            .iter()
            .any(|item| item.field == "jitter_buffer_ms"));

        let defaults = SessionParams::mvp_default();
        assert_eq!(
            state.snapshot.effective.sample_rate_hz,
            defaults.sample_rate_hz
        );
        assert_eq!(state.snapshot.effective.chunk_ms, defaults.chunk_ms);
        assert_eq!(
            state.snapshot.effective.opus_bitrate_kbps,
            defaults.opus_bitrate_kbps
        );
        assert_eq!(
            state.snapshot.effective.jitter_buffer_ms,
            defaults.jitter_buffer_ms
        );
    }

    #[test]
    fn handshake_outcome_updates_status_on_success() {
        let mut state = AppState::default();
        let response = HandshakeResponse {
            session_id: "session-1".to_string(),
            accepted: true,
            reason: None,
            effective: SessionParams::mvp_default(),
            busy: false,
        };
        let accepted = apply_handshake_outcome(&mut state, "127.0.0.1:43000", Ok(response));
        assert!(accepted);
        assert_eq!(state.snapshot.status, "streaming");
        assert_eq!(state.snapshot.status_note, "推流中（握手成功）");
        assert_eq!(
            state.snapshot.runtime.peer_addr,
            Some("127.0.0.1:43000".to_string())
        );
        assert!(state.snapshot.runtime.server_status_updated_ms > 0);
    }

    #[test]
    fn client_stats_update_refreshes_server_status_timestamp() {
        let mut state = AppState::default();
        state.snapshot.status = "connecting".to_string();
        state.snapshot.status_note = "正在建立连接".to_string();
        let stats = StatsSnapshot {
            packets_received: 120,
            packets_lost: 3,
            buffer_depth_frames: 84,
            buffer_depth_ms: 42,
            jitter_buffer_depth_ms: 21.0,
            estimated_e2e_latency_ms: 55.0,
            audio_rms: 0.15,
            audio_peak: 1234,
        };

        apply_client_stats_update(&mut state, &stats, 123_456);

        assert_eq!(state.snapshot.status, "streaming");
        assert_eq!(state.snapshot.status_note, "推流中（服务端已响应）");
        assert_eq!(state.snapshot.runtime.server_status_updated_ms, 123_456);
        assert_eq!(state.last_server_seen_ms, Some(123_456));
        assert_eq!(state.snapshot.metrics.buffer_depth_ms, 42.0);
        assert_eq!(state.snapshot.metrics.packet_loss_pct, 2.5);
    }

    #[test]
    fn handshake_outcome_marks_busy_as_error() {
        let mut state = AppState::default();
        state.streaming = true;
        let response = HandshakeResponse {
            session_id: "session-1".to_string(),
            accepted: false,
            reason: Some("busy".to_string()),
            effective: SessionParams::mvp_default(),
            busy: true,
        };
        let accepted = apply_handshake_outcome(&mut state, "127.0.0.1:43000", Ok(response));
        assert!(!accepted);
        assert_eq!(state.snapshot.status, "error");
        assert_eq!(state.snapshot.status_note, "握手失败：服务端忙");
        assert_eq!(state.streaming, false);
    }

    #[test]
    fn handshake_outcome_reports_reject_reason() {
        let mut state = AppState::default();
        state.streaming = true;
        let response = HandshakeResponse {
            session_id: "session-1".to_string(),
            accepted: false,
            reason: Some("unauthorized".to_string()),
            effective: SessionParams::mvp_default(),
            busy: false,
        };
        let accepted = apply_handshake_outcome(&mut state, "127.0.0.1:43000", Ok(response));
        assert!(!accepted);
        assert_eq!(state.snapshot.status, "error");
        assert!(state.snapshot.status_note.contains("unauthorized"));
    }

    #[test]
    fn handshake_outcome_reports_transport_error() {
        let mut state = AppState::default();
        state.streaming = true;
        let accepted =
            apply_handshake_outcome(&mut state, "127.0.0.1:43000", Err("timeout".to_string()));
        assert!(!accepted);
        assert_eq!(state.snapshot.status, "error");
        assert!(state.snapshot.status_note.contains("timeout"));
    }

    #[test]
    fn harness_render_ack_writes_frontend_log() {
        let log_path = std::env::temp_dir().join(format!("netmic-ui-render-{}.ndjson", now_ms()));

        with_env_var(
            ENV_HARNESS_RENDER_LOG,
            Some(log_path.to_string_lossy().as_ref()),
            || {
                let ack = HarnessRenderAck {
                    snapshot: AppState::default().snapshot,
                    visible: HarnessVisibleState {
                        active_tab: "status".to_string(),
                        status_label: "推流中".to_string(),
                        status_note: "推流中（握手成功）".to_string(),
                        primary_action: "停止推流".to_string(),
                        connection_lines: vec!["模式：Client".to_string()],
                        metrics_lines: vec!["RTT".to_string(), "4.0 ms".to_string()],
                        audio_lines: vec!["时域波形".to_string()],
                        params_lines: vec!["Codec：opus".to_string()],
                        events_lines: vec!["[INFO] 准备就绪".to_string()],
                        config_connection_lines: vec!["Server IP".to_string()],
                        config_audio_lines: vec!["音频参数".to_string()],
                        config_client_lines: vec!["输入设备".to_string()],
                        config_server_lines: vec![],
                        fallback_lines: vec!["暂无回退记录".to_string()],
                        log_filter: "all".to_string(),
                        log_lines: vec!["INFO 准备就绪".to_string()],
                    },
                };

                assert!(report_harness_render(ack));

                let payload = fs::read_to_string(&log_path).expect("render log should exist");
                assert!(payload.contains("\"active_tab\":\"status\""), "{payload}");
                assert!(payload.contains("\"status_label\":\"推流中\""), "{payload}");
                assert!(
                    payload.contains("\"primary_action\":\"停止推流\""),
                    "{payload}"
                );
                assert!(
                    payload.contains("\"connection_lines\":[\"模式：Client\"]"),
                    "{payload}"
                );
            },
        );

        let _ = fs::remove_file(&log_path);
    }
}
