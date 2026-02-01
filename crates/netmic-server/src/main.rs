//! NetMic 服务端入口（M1：UDP 接收骨架）。
//!
//! 设计说明（与文档对齐）：
//! - 协议事实来源：`docs/PROTO.md` 与 `MVP.md#5`。
//! - 先提供“可监听 + 可分流 + 单客户端占位”的最小正确性。
//! - 控制面/数据面解析均为占位实现，后续再接入真实握手/解码/注入。

use std::env;
use std::f32::consts::PI;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use netmic_proto::config::normalize_session_params;
use netmic_proto::control::{
    decode_control_message, decode_control_payload, encode_control_message,
    CONTROL_TYPE_HANDSHAKE_REQUEST, CONTROL_TYPE_HANDSHAKE_RESPONSE, CONTROL_TYPE_HEARTBEAT,
    CONTROL_TYPE_STATS,
    CONTROL_TYPE_SERVER_COMMAND_REQUEST, CONTROL_TYPE_SERVER_COMMAND_RESPONSE,
    CONTROL_TYPE_SERVER_STATUS_REQUEST, CONTROL_TYPE_SERVER_STATUS_RESPONSE,
};
use netmic_proto::datagram::{
    split_audio_pcm16_with_header, split_datagram, wrap_control_json, DatagramKind,
};
use netmic_proto::protocol::{
    HandshakeRequest, HandshakeResponse, Heartbeat, ServerCommandRequest, ServerCommandResponse,
    ServerStatusRequest, ServerStatusResponse, SessionParams, StatsSnapshot,
};
use tracing::{debug, info, warn};

/// 默认 UDP 监听端口（MVP 占位值，后续可统一到配置模块）。
const DEFAULT_UDP_PORT: u16 = 43_000;
/// 默认绑定地址（监听所有网卡）。
const DEFAULT_BIND_ADDR: &str = "0.0.0.0";
/// 绑定地址环境变量（支持 host 或 host:port）。
const ENV_BIND_ADDR: &str = "NETMIC_SERVER_BIND_ADDR";
/// 是否自动创建虚拟麦克风（1/true/on/yes）。
const ENV_VIRTUAL_MIC_AUTO_CREATE: &str = "NETMIC_SERVER_VIRTUAL_MIC_AUTO_CREATE";
/// 单次接收缓冲区大小（足够容纳 MVP 小包）。
const MAX_DATAGRAM_SIZE: usize = 1500;
/// 读超时（用于避免无流量时永久阻塞，便于日志可观测）。
const READ_TIMEOUT_MS: u64 = 250;
/// 进入重连状态的空闲宽限（避免偶发抖动立即触发重连）。
const RECONNECT_GRACE_SECS: u64 = 1;
/// 重连窗口目标（MVP 要求 10 秒内恢复）。
const RECONNECT_WINDOW_SECS: u64 = 10;
/// 指标输出间隔（占位值，先保证日志可观测）。
const METRICS_REPORT_SECS: u64 = 5;
/// 目标缓冲深度（占位：与 MVP 默认 100ms 目标一致）。
const BUFFER_TARGET_MS: u64 = 100;
/// 音频 sink 选择（pulse/null，默认 pulse）。
const ENV_AUDIO_SINK: &str = "NETMIC_SERVER_AUDIO_SINK";
/// 是否启用测试音注入（1/true/on/yes）。
const ENV_TEST_TONE: &str = "NETMIC_SERVER_TEST_TONE";
/// 测试音持续时长（秒，0 表示不限制）。
const ENV_TEST_TONE_SECS: &str = "NETMIC_SERVER_TEST_TONE_SECS";
/// 测试音频率（Hz）。
const ENV_TEST_TONE_HZ: &str = "NETMIC_SERVER_TEST_TONE_HZ";
/// 测试音幅度（0.0–1.0）。
const ENV_TEST_TONE_GAIN: &str = "NETMIC_SERVER_TEST_TONE_GAIN";

fn main() -> Result<()> {
    init_tracing();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-server starting with MVP defaults");

    let test_tone = should_enable_test_tone();
    let auto_create = should_auto_create_virtual_mic() || test_tone;
    if auto_create {
        ensure_virtual_mic_created()?;
    }

    let audio_sink = audio_sink_from_env(&params)?;

    if test_tone {
        info!("test tone enabled, start injecting sine wave");
        return run_test_tone(&params, audio_sink);
    }

    let port = udp_port_from_env();
    let bind_addr = bind_addr_from_env(port);
    let socket = bind_udp_socket(&bind_addr)?;

    info!(%bind_addr, "udp receiver skeleton ready");
    run_receiver_loop(&socket, &params, audio_sink)
}

#[cfg(test)]
mod tests {
    use super::*;
    use netmic_proto::config::{DEFAULT_CHUNK_MS, DEFAULT_SAMPLE_RATE_HZ};
    use netmic_proto::control::{decode_control_message, decode_control_payload};
    use netmic_proto::control::{CONTROL_TYPE_HANDSHAKE_REQUEST, CONTROL_TYPE_HANDSHAKE_RESPONSE};
    use netmic_proto::datagram::wrap_control_json;
    use netmic_proto::protocol::HandshakeRequest;

    #[test]
    fn receiver_locks_first_client_and_rejects_others() {
        let params = SessionParams::mvp_default();
        let mut ctx =
            ReceiverContext::new(params.sample_rate_hz, params.channels, Box::new(NullSink));
        let now = Instant::now();
        let addr_a: SocketAddr = "127.0.0.1:10001".parse().unwrap();
        let addr_b: SocketAddr = "127.0.0.1:10002".parse().unwrap();

        assert!(ctx.accept_or_lock_client(addr_a, now));
        assert_eq!(ctx.active_client, Some(addr_a));
        assert_eq!(ctx.state, ConnectionState::Active);

        assert!(!ctx.accept_or_lock_client(addr_b, now));
        assert_eq!(ctx.metrics.busy_rejects, 1);
    }

    #[test]
    fn receiver_reconnect_window_releases_client() {
        let params = SessionParams::mvp_default();
        let mut ctx =
            ReceiverContext::new(params.sample_rate_hz, params.channels, Box::new(NullSink));
        let base = Instant::now();
        let addr: SocketAddr = "127.0.0.1:10003".parse().unwrap();

        assert!(ctx.accept_or_lock_client(addr, base));
        ctx.update_reconnect_state(base + Duration::from_secs(RECONNECT_GRACE_SECS + 1));
        assert_eq!(ctx.state, ConnectionState::Reconnecting);

        ctx.update_reconnect_state(base + Duration::from_secs(RECONNECT_WINDOW_SECS + 1));
        assert_eq!(ctx.state, ConnectionState::Idle);
        assert!(ctx.active_client.is_none());
    }

    #[test]
    fn metrics_tracks_buffer_depth_for_audio_payloads() {
        let now = Instant::now();
        let mut metrics = ReceiverMetrics::new(now, 48_000, 1);
        let payload = vec![0_u8; 960 * 2];

        metrics.on_datagram(DatagramKind::AudioPcm16, &payload);
        assert_eq!(metrics.buffer_depth_ms(), 20);
    }

    #[test]
    fn metrics_reports_rms_and_peak() {
        let now = Instant::now();
        let mut metrics = ReceiverMetrics::new(now, 48_000, 1);
        let samples = [0_i16, 1000_i16, -1000_i16];
        let mut payload = Vec::new();
        for sample in samples {
            payload.extend_from_slice(&sample.to_le_bytes());
        }

        metrics.on_datagram(DatagramKind::AudioPcm16, &payload);
        let (rms, peak) = metrics.take_audio_level_snapshot();
        assert_eq!(peak, 1000);
        let expected = ((0.0_f32 * 0.0 + 1000.0_f32 * 1000.0 + 1000.0_f32 * 1000.0) / 3.0).sqrt();
        assert!((rms - expected).abs() < 0.01);
    }

    #[test]
    fn sine_frame_has_expected_length() {
        let mut phase = 0.0_f32;
        let frame = build_sine_frame(440.0, 0.2, 48_000, 960, &mut phase);
        assert_eq!(frame.len(), 960);
        assert!(frame.iter().any(|value| *value != 0));
    }

    #[test]
    fn sine_frame_clamps_amplitude() {
        let mut phase = 0.0_f32;
        let frame = build_sine_frame(440.0, 1.5, 48_000, 10, &mut phase);
        let max = frame
            .iter()
            .map(|value| (*value as i32).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(max <= i16::MAX as u32);
    }

    #[test]
    fn handshake_request_returns_control_response() {
        let params = SessionParams::mvp_default();
        let mut ctx =
            ReceiverContext::new(params.sample_rate_hz, params.channels, Box::new(NullSink));
        let now = Instant::now();
        let addr: SocketAddr = "127.0.0.1:12001".parse().unwrap();
        let request = HandshakeRequest {
            session_id: "session-1".to_string(),
            client_name: "client".to_string(),
            requested: SessionParams {
                sample_rate_hz: 12_345,
                chunk_ms: 15,
                ..SessionParams::mvp_default()
            },
            token: None,
        };
        let payload = encode_control_message(CONTROL_TYPE_HANDSHAKE_REQUEST, &request).unwrap();
        let datagram = wrap_control_json(&payload);
        let (_, payload) = split_datagram(&datagram).expect("split");
        let response_bytes = ctx
            .handle_control_payload(payload, addr, now)
            .expect("response");
        let (_, response_payload) = split_datagram(&response_bytes).expect("split response");
        let (msg_type, payload_value) = decode_control_message(response_payload).expect("decode");
        assert_eq!(msg_type, CONTROL_TYPE_HANDSHAKE_RESPONSE);
        let response: HandshakeResponse =
            decode_control_payload(payload_value).expect("payload decode");
        assert!(response.accepted);
        assert!(!response.busy);
        assert_eq!(response.session_id, "session-1");
        assert_eq!(response.effective.sample_rate_hz, DEFAULT_SAMPLE_RATE_HZ);
        assert_eq!(response.effective.chunk_ms, DEFAULT_CHUNK_MS);
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

fn udp_port_from_env() -> u16 {
    match env::var("NETMIC_SERVER_UDP_PORT") {
        Ok(raw) => match raw.parse::<u16>() {
            Ok(port) => port,
            Err(err) => {
                warn!(%raw, %err, "invalid NETMIC_SERVER_UDP_PORT, fallback to default");
                DEFAULT_UDP_PORT
            }
        },
        Err(_) => DEFAULT_UDP_PORT,
    }
}

fn bind_addr_from_env(port: u16) -> String {
    let default_addr = format!("{DEFAULT_BIND_ADDR}:{port}");
    let Ok(raw) = env::var(ENV_BIND_ADDR) else {
        return default_addr;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return default_addr;
    }
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        info!(
            env = ENV_BIND_ADDR,
            value = %trimmed,
            "NETMIC_SERVER_BIND_ADDR includes port, NETMIC_SERVER_UDP_PORT ignored"
        );
        return addr.to_string();
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return format!("{trimmed}:{port}");
    }
    let colon_count = trimmed.matches(':').count();
    if colon_count == 1 {
        info!(
            env = ENV_BIND_ADDR,
            value = %trimmed,
            "NETMIC_SERVER_BIND_ADDR includes port, NETMIC_SERVER_UDP_PORT ignored"
        );
        return trimmed.to_string();
    }
    if colon_count > 1 {
        return format!("[{trimmed}]:{port}");
    }
    format!("{trimmed}:{port}")
}

fn bind_udp_socket(bind_addr: &str) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(bind_addr)
        .with_context(|| format!("failed to bind UDP socket at {bind_addr}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
        .context("failed to set UDP read timeout")?;
    Ok(socket)
}

fn run_receiver_loop(
    socket: &UdpSocket,
    params: &SessionParams,
    audio_sink: Box<dyn AudioSink>,
) -> Result<()> {
    let mut buf = [0_u8; MAX_DATAGRAM_SIZE];
    let mut ctx = ReceiverContext::new(params.sample_rate_hz, params.channels, audio_sink);

    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let packet = &buf[..len];
                let now = Instant::now();
                if let Some((kind, payload)) = split_datagram(packet) {
                    match kind {
                        DatagramKind::ControlJson => {
                            ctx.metrics.on_datagram(kind, payload);
                            if let Some(response) = ctx.handle_control_payload(payload, addr, now) {
                                if let Err(err) = socket.send_to(&response, addr) {
                                    warn!(%err, %addr, "failed to send control response");
                                }
                            }
                        }
                        DatagramKind::AudioPcm16 => {
                            if !ctx.accept_or_lock_client(addr, now) {
                                continue;
                            }
                            match split_audio_pcm16_with_header(payload) {
                                Ok((header, pcm)) => {
                                    ctx.metrics.on_datagram(kind, pcm);
                                    if let Err(err) = ctx.audio_sink.write_pcm16(pcm) {
                                        ctx.set_last_error(format!(
                                            "audio sink write failed: {err}"
                                        ));
                                        warn!(%err, "audio sink write failed");
                                    }
                                    info!(
                                        %addr,
                                        seq = header.seq,
                                        timestamp_ms = header.timestamp_ms,
                                        frame_samples = header.frame_samples,
                                        bytes = pcm.len(),
                                        kind = kind.as_str(),
                                        "received audio payload (pcm16)"
                                    );
                                }
                                Err(err) => {
                                    ctx.set_last_error(format!(
                                        "audio header decode failed: {err}"
                                    ));
                                    warn!(%addr, %err, "failed to decode audio header");
                                    ctx.metrics.on_datagram(kind, payload);
                                }
                            }
                        }
                        DatagramKind::Unknown(tag) => {
                            ctx.metrics.on_datagram(kind, payload);
                            warn!(
                                %addr,
                                tag,
                                bytes = payload.len(),
                                "received datagram with unknown kind"
                            );
                        }
                    }
                } else {
                    ctx.metrics.on_empty();
                    debug!(%addr, "received empty datagram");
                }
                ctx.maybe_report(now);
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                let now = Instant::now();
                ctx.on_timeout_tick(now);
                ctx.maybe_report(now);
                debug!("udp recv timeout (no packets yet)");
            }
            Err(err) => {
                warn!(%err, "udp recv error");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Idle,
    Active,
    Reconnecting,
}

impl ConnectionState {
    fn as_str(self) -> &'static str {
        match self {
            ConnectionState::Idle => "idle",
            ConnectionState::Active => "active",
            ConnectionState::Reconnecting => "reconnecting",
        }
    }
}

struct ReceiverContext {
    state: ConnectionState,
    active_client: Option<SocketAddr>,
    active_since: Option<Instant>,
    started_at: Instant,
    last_packet_at: Option<Instant>,
    last_error: Option<String>,
    metrics: ReceiverMetrics,
    audio_sink: Box<dyn AudioSink>,
}

impl ReceiverContext {
    fn new(sample_rate_hz: u32, channels: u16, audio_sink: Box<dyn AudioSink>) -> Self {
        let now = Instant::now();
        Self {
            state: ConnectionState::Idle,
            active_client: None,
            active_since: None,
            started_at: now,
            last_packet_at: None,
            last_error: None,
            metrics: ReceiverMetrics::new(now, sample_rate_hz, channels),
            audio_sink,
        }
    }

    /// 单客户端占位策略：
    /// - 首个发送方锁定为 active client；
    /// - 其他来源直接拒绝并打日志（后续可回发 BUSY 控制消息）。
    fn accept_or_lock_client(&mut self, addr: SocketAddr, now: Instant) -> bool {
        match self.active_client {
            Some(current) if current != addr => {
                self.metrics.on_busy_reject();
                warn!(%addr, active = %current, "reject packet from non-active client (busy)");
                false
            }
            Some(_) => {
                if self.state == ConnectionState::Reconnecting {
                    self.transition_to(ConnectionState::Active, "packet received, reconnect ok");
                }
                self.last_packet_at = Some(now);
                true
            }
            None => {
                info!(%addr, "lock active client");
                self.active_client = Some(addr);
                self.active_since = Some(now);
                self.last_packet_at = Some(now);
                self.transition_to(ConnectionState::Active, "lock active client");
                true
            }
        }
    }

    fn on_timeout_tick(&mut self, now: Instant) {
        self.metrics.on_timeout(now);
        self.update_reconnect_state(now);
    }

    fn update_reconnect_state(&mut self, now: Instant) {
        let Some(last_seen) = self.last_packet_at else {
            return;
        };
        let idle_for = now.saturating_duration_since(last_seen);

        if idle_for >= Duration::from_secs(RECONNECT_GRACE_SECS)
            && self.state == ConnectionState::Active
        {
            self.transition_to(
                ConnectionState::Reconnecting,
                "no packets, enter reconnecting",
            );
        }

        if idle_for >= Duration::from_secs(RECONNECT_WINDOW_SECS)
            && self.state == ConnectionState::Reconnecting
        {
            let released = self.active_client.take();
            self.active_since = None;
            self.last_packet_at = None;
            self.transition_to(
                ConnectionState::Idle,
                "reconnect window exceeded, release client",
            );
            if let Some(addr) = released {
                info!(%addr, "active client released after reconnect timeout");
            }
        }
    }

    fn maybe_report(&mut self, now: Instant) {
        self.metrics
            .report_if_due(now, self.state, self.active_client, self.last_packet_at);
    }

    fn transition_to(&mut self, next: ConnectionState, reason: &str) {
        if self.state == next {
            return;
        }
        info!(
            from = self.state.as_str(),
            to = next.as_str(),
            reason,
            "receiver state transition"
        );
        self.state = next;
    }

    fn handle_control_payload(
        &mut self,
        payload: &[u8],
        addr: SocketAddr,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let (msg_type, payload_value) = match decode_control_message(payload) {
            Ok(tuple) => tuple,
            Err(err) => {
                warn!(%addr, %err, "failed to decode control message");
                return None;
            }
        };

        if msg_type.as_str() == CONTROL_TYPE_HANDSHAKE_REQUEST {
            let request: HandshakeRequest = match decode_control_payload(payload_value) {
                Ok(value) => value,
                Err(err) => {
                    warn!(%addr, %err, "invalid handshake request payload");
                    return None;
                }
            };

            let accepted = match self.active_client {
                Some(current) => current == addr,
                None => true,
            };
            let normalize = normalize_session_params(&request.requested);
            let (busy, reason) = if accepted {
                (false, None)
            } else {
                (true, Some("busy".to_string()))
            };

            let response = HandshakeResponse {
                session_id: request.session_id,
                accepted,
                reason,
                effective: normalize.effective,
                busy,
            };
            let payload = match encode_control_message(CONTROL_TYPE_HANDSHAKE_RESPONSE, &response) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!(%addr, %err, "failed to encode handshake response");
                    return None;
                }
            };
            info!(
                %addr,
                accepted,
                busy,
                "handled handshake request"
            );
            return Some(wrap_control_json(&payload));
        }

        if msg_type.as_str() == CONTROL_TYPE_HEARTBEAT {
            let _heartbeat: Heartbeat = match decode_control_payload(payload_value) {
                Ok(value) => value,
                Err(err) => {
                    warn!(%addr, %err, "invalid heartbeat payload");
                    return None;
                }
            };
            if !self.accept_or_lock_client(addr, now) {
                return None;
            }
            let stats = self.metrics.build_stats_snapshot();
            let payload = match encode_control_message(CONTROL_TYPE_STATS, &stats) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!(%addr, %err, "failed to encode stats snapshot");
                    return None;
                }
            };
            return Some(wrap_control_json(&payload));
        }

        if msg_type.as_str() == CONTROL_TYPE_SERVER_STATUS_REQUEST {
            let request: ServerStatusRequest = match decode_control_payload(payload_value) {
                Ok(value) => value,
                Err(err) => {
                    warn!(%addr, %err, "invalid server status request payload");
                    return None;
                }
            };
            if !addr.ip().is_loopback() {
                warn!(%addr, "reject server status request from non-loopback address");
                return None;
            }
            let response = self.build_status_response(request.request_id, now);
            let payload = match encode_control_message(CONTROL_TYPE_SERVER_STATUS_RESPONSE, &response)
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!(%addr, %err, "failed to encode server status response");
                    return None;
                }
            };
            return Some(wrap_control_json(&payload));
        }

        if msg_type.as_str() == CONTROL_TYPE_SERVER_COMMAND_REQUEST {
            let request: ServerCommandRequest = match decode_control_payload(payload_value) {
                Ok(value) => value,
                Err(err) => {
                    warn!(%addr, %err, "invalid server command request payload");
                    return None;
                }
            };
            if !addr.ip().is_loopback() {
                warn!(%addr, "reject server command from non-loopback address");
                return None;
            }
            let response = self.handle_server_command(request, now);
            let payload = match encode_control_message(CONTROL_TYPE_SERVER_COMMAND_RESPONSE, &response)
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!(%addr, %err, "failed to encode server command response");
                    return None;
                }
            };
            return Some(wrap_control_json(&payload));
        }

        info!(%addr, msg_type, "received control json payload");
        None
    }

    fn handle_server_command(
        &mut self,
        request: ServerCommandRequest,
        _now: Instant,
    ) -> ServerCommandResponse {
        let action = request.action.as_str();
        match action {
            "force_disconnect" => {
                let had_client = self.active_client.take();
                self.active_since = None;
                self.last_packet_at = None;
                self.transition_to(ConnectionState::Idle, "force disconnect");
                if let Some(addr) = had_client {
                    info!(%addr, "force disconnected active client");
                }
                ServerCommandResponse {
                    request_id: request.request_id,
                    ok: true,
                    message: None,
                }
            }
            "virtual_mic_create" => match ensure_virtual_mic_created() {
                Ok(()) => ServerCommandResponse {
                    request_id: request.request_id,
                    ok: true,
                    message: None,
                },
                Err(err) => ServerCommandResponse {
                    request_id: request.request_id,
                    ok: false,
                    message: Some(format!("create virtual mic failed: {err}")),
                },
            },
            "virtual_mic_remove" => match remove_virtual_mic() {
                Ok(()) => ServerCommandResponse {
                    request_id: request.request_id,
                    ok: true,
                    message: None,
                },
                Err(err) => ServerCommandResponse {
                    request_id: request.request_id,
                    ok: false,
                    message: Some(format!("remove virtual mic failed: {err}")),
                },
            },
            other => ServerCommandResponse {
                request_id: request.request_id,
                ok: false,
                message: Some(format!("unsupported command: {other}")),
            },
        }
    }

    fn build_status_response(&mut self, request_id: String, now: Instant) -> ServerStatusResponse {
        let state = self.status_label();
        let active_client = self.active_client.map(|addr| addr.to_string());
        let active_client_seconds = self
            .active_since
            .map(|since| now.saturating_duration_since(since).as_secs())
            .unwrap_or(0);
        let uptime_ms = now
            .saturating_duration_since(self.started_at)
            .as_millis() as u64;
        let (virtual_mic_name, virtual_mic_ready, virtual_mic_error) = virtual_mic_status();
        let stats = self.metrics.build_stats_snapshot();
        ServerStatusResponse {
            request_id,
            state,
            active_client,
            active_client_seconds,
            uptime_ms,
            last_error: self.last_error.clone(),
            virtual_mic_name,
            virtual_mic_ready,
            virtual_mic_error,
            stats,
        }
    }

    fn status_label(&self) -> String {
        if self.active_client.is_none() {
            return "listening".to_string();
        }
        match self.state {
            ConnectionState::Active => "streaming".to_string(),
            ConnectionState::Reconnecting => "reconnecting".to_string(),
            ConnectionState::Idle => "listening".to_string(),
        }
    }

    fn set_last_error(&mut self, err: impl Into<String>) {
        self.last_error = Some(err.into());
    }
}

trait AudioSink {
    fn write_pcm16(&mut self, payload: &[u8]) -> Result<usize>;
}

struct NullSink;

impl AudioSink for NullSink {
    fn write_pcm16(&mut self, _payload: &[u8]) -> Result<usize> {
        Ok(0)
    }
}

#[cfg(target_os = "linux")]
struct PulseAudioSink {
    simple: libpulse_simple_binding::Simple,
}

#[cfg(target_os = "linux")]
impl PulseAudioSink {
    fn new(sink_name: &str, sample_rate_hz: u32, channels: u16) -> Result<Self> {
        use libpulse_binding::sample::{Format, Spec};
        use libpulse_binding::stream::Direction;

        let spec = Spec {
            format: Format::S16le,
            channels: channels as u8,
            rate: sample_rate_hz,
        };
        if !spec.is_valid() {
            return Err(anyhow::anyhow!(
                "invalid pulse sample spec: rate={sample_rate_hz}, channels={channels}"
            ));
        }
        let simple = libpulse_simple_binding::Simple::new(
            None,
            "netmic-server",
            Direction::Playback,
            Some(sink_name),
            "NetMic Virtual Mic",
            &spec,
            None,
            None,
        )
        .map_err(|err| anyhow::anyhow!("pulse simple init failed: {err}"))?;
        Ok(Self { simple })
    }
}

#[cfg(target_os = "linux")]
impl AudioSink for PulseAudioSink {
    fn write_pcm16(&mut self, payload: &[u8]) -> Result<usize> {
        self.simple
            .write(payload)
            .map_err(|err| anyhow::anyhow!("pulse write failed: {err}"))?;
        Ok(payload.len())
    }
}

struct FileDumpSink {
    path: PathBuf,
    file: std::fs::File,
    total_bytes: u64,
}

impl FileDumpSink {
    fn new(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open audio dump file: {}", path.display()))?;
        Ok(Self {
            path,
            file,
            total_bytes: 0,
        })
    }
}

impl AudioSink for FileDumpSink {
    fn write_pcm16(&mut self, payload: &[u8]) -> Result<usize> {
        self.file
            .write_all(payload)
            .with_context(|| format!("failed to append audio dump: {}", self.path.display()))?;
        self.total_bytes = self.total_bytes.saturating_add(payload.len() as u64);
        info!(
            bytes = payload.len(),
            total_bytes = self.total_bytes,
            path = %self.path.display(),
            "audio dump sink appended payload"
        );
        Ok(payload.len())
    }
}

fn audio_sink_from_env(params: &SessionParams) -> Result<Box<dyn AudioSink>> {
    let Ok(raw) = env::var("NETMIC_SERVER_AUDIO_DUMP") else {
        return build_sink_from_mode(params);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return build_sink_from_mode(params);
    }
    let path = PathBuf::from(trimmed);
    let sink = FileDumpSink::new(path)?;
    info!(path = %sink.path.display(), "NETMIC_SERVER_AUDIO_DUMP enabled");
    Ok(Box::new(sink))
}

fn should_auto_create_virtual_mic() -> bool {
    match env::var(ENV_VIRTUAL_MIC_AUTO_CREATE) {
        Ok(raw) => !matches!(raw.as_str(), "0" | "false" | "off" | "no"),
        Err(_) => true,
    }
}

fn should_enable_test_tone() -> bool {
    read_bool_env(ENV_TEST_TONE)
}

fn test_tone_duration_from_env() -> Option<Duration> {
    match env::var(ENV_TEST_TONE_SECS) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(err) => {
                warn!(%raw, %err, "invalid test tone duration, fallback to 300s");
                Some(Duration::from_secs(300))
            }
        },
        Err(_) => Some(Duration::from_secs(300)),
    }
}

fn test_tone_freq_from_env() -> f32 {
    match env::var(ENV_TEST_TONE_HZ) {
        Ok(raw) => raw.parse::<f32>().unwrap_or(440.0),
        Err(_) => 440.0,
    }
}

fn test_tone_gain_from_env() -> f32 {
    match env::var(ENV_TEST_TONE_GAIN) {
        Ok(raw) => raw.parse::<f32>().unwrap_or(0.2),
        Err(_) => 0.2,
    }
}

fn run_test_tone(params: &SessionParams, mut sink: Box<dyn AudioSink>) -> Result<()> {
    let duration = test_tone_duration_from_env();
    let freq_hz = test_tone_freq_from_env().max(1.0);
    let gain = test_tone_gain_from_env().clamp(0.0, 0.9);
    let frame_samples = ((params.sample_rate_hz as u64)
        .saturating_mul(params.chunk_ms as u64)
        / 1000)
        .max(1) as usize;
    let mut phase = 0.0_f32;
    let started = Instant::now();
    let interval = Duration::from_millis(params.chunk_ms.max(1) as u64);

    loop {
        if let Some(limit) = duration {
            if started.elapsed() >= limit {
                info!("test tone duration reached, stop");
                break;
            }
        }
        let frame = build_sine_frame(freq_hz, gain, params.sample_rate_hz, frame_samples, &mut phase);
        let payload = pcm16_to_bytes(&frame);
        sink.write_pcm16(&payload)?;
        std::thread::sleep(interval);
    }
    Ok(())
}

fn build_sine_frame(
    freq_hz: f32,
    gain: f32,
    sample_rate_hz: u32,
    frames: usize,
    phase: &mut f32,
) -> Vec<i16> {
    let mut out = Vec::with_capacity(frames);
    let step = 2.0 * PI * freq_hz / sample_rate_hz.max(1) as f32;
    for _ in 0..frames {
        let value = (*phase).sin() * gain;
        let clamped = value.clamp(-1.0, 1.0);
        out.push((clamped * i16::MAX as f32) as i16);
        *phase += step;
        if *phase >= 2.0 * PI {
            *phase -= 2.0 * PI;
        }
    }
    out
}

fn pcm16_to_bytes(samples: &[i16]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    buf
}

fn read_bool_env(name: &str) -> bool {
    match env::var(name) {
        Ok(raw) => matches!(raw.as_str(), "1" | "true" | "on" | "yes"),
        Err(_) => false,
    }
}

fn build_sink_from_mode(params: &SessionParams) -> Result<Box<dyn AudioSink>> {
    let mode = env::var(ENV_AUDIO_SINK).unwrap_or_else(|_| "pulse".to_string());
    match mode.as_str() {
        "null" => Ok(Box::new(NullSink)),
        "pulse" => {
            #[cfg(target_os = "linux")]
            {
                let config = virtual_mic_config_from_env();
                let sink = PulseAudioSink::new(
                    config.sink_name.as_str(),
                    params.sample_rate_hz,
                    params.channels,
                )?;
                info!(
                    sink = %config.sink_name,
                    sample_rate_hz = params.sample_rate_hz,
                    channels = params.channels,
                    "pulse audio sink ready"
                );
                return Ok(Box::new(sink));
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(anyhow::anyhow!("pulse sink only supported on linux"))
            }
        }
        other => Err(anyhow::anyhow!(
            "unsupported NETMIC_SERVER_AUDIO_SINK={other} (expected pulse/null)"
        )),
    }
}

fn virtual_mic_status() -> (String, bool, Option<String>) {
    let config = virtual_mic_config_from_env();
    match source_exists(&config.source_name) {
        Ok(true) => (config.source_name, true, None),
        Ok(false) => (
            config.source_name,
            false,
            Some("未检测到虚拟麦克风".to_string()),
        ),
        Err(err) => (
            config.source_name,
            false,
            Some(format!("pactl 查询失败: {err}")),
        ),
    }
}

struct VirtualMicConfig {
    state_path: PathBuf,
    sink_name: String,
    source_name: String,
    sink_desc: String,
    source_desc: String,
}

fn virtual_mic_config_from_env() -> VirtualMicConfig {
    let prefix = env_or_default("NETMIC_VIRTUAL_MIC_PREFIX", "netmic");
    let sink_name = env_or_default("NETMIC_VIRTUAL_MIC_SINK_NAME", &format!("{prefix}_sink"));
    let source_name =
        env_or_default("NETMIC_VIRTUAL_MIC_SOURCE_NAME", &format!("{prefix}_source"));
    let sink_desc = env_or_default("NETMIC_VIRTUAL_MIC_SINK_DESC", "NetMic_Virtual_Sink");
    let source_desc = env_or_default("NETMIC_VIRTUAL_MIC_SOURCE_DESC", "NetMic_Virtual_Mic");
    let state_path = PathBuf::from(env_or_default(
        "NETMIC_VIRTUAL_MIC_STATE",
        "/tmp/netmic_virtual_mic.env",
    ));
    VirtualMicConfig {
        state_path,
        sink_name,
        source_name,
        sink_desc,
        source_desc,
    }
}

fn env_or_default(name: &str, fallback: &str) -> String {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn ensure_virtual_mic_created() -> Result<()> {
    let config = virtual_mic_config_from_env();
    if source_exists(&config.source_name)? {
        info!(source = %config.source_name, "virtual mic already exists");
        return Ok(());
    }

    let sink_id = load_pactl_module(
        "module-null-sink",
        &[
            format!("sink_name={}", config.sink_name),
            format!("sink_properties=device.description={}", config.sink_desc),
        ],
    )?;

    let source_id = load_pactl_module(
        "module-remap-source",
        &[
            format!("master={}.monitor", config.sink_name),
            format!("source_name={}", config.source_name),
            format!("source_properties=device.description={}", config.source_desc),
        ],
    )
    .map_err(|err| {
        let _ = unload_pactl_module(&sink_id);
        err
    })?;

    if !source_exists(&config.source_name)? {
        let _ = unload_pactl_module(&source_id);
        let _ = unload_pactl_module(&sink_id);
        return Err(anyhow::anyhow!(
            "created virtual mic but source not found: {}",
            config.source_name
        ));
    }

    write_virtual_mic_state(&config, &sink_id, &source_id)?;
    info!(
        sink = %config.sink_name,
        source = %config.source_name,
        sink_id,
        source_id,
        "virtual mic created"
    );
    Ok(())
}

fn remove_virtual_mic() -> Result<()> {
    let config = virtual_mic_config_from_env();
    let (mut sink_id, mut source_id) = read_virtual_mic_state(&config.state_path);

    if sink_id.is_empty() {
        sink_id = find_module_id("module-null-sink", &format!("sink_name={}", config.sink_name))
            .unwrap_or_default();
    }
    if source_id.is_empty() {
        source_id = find_module_id("module-remap-source", &format!("source_name={}", config.source_name))
            .unwrap_or_default();
    }

    if !source_id.is_empty() {
        unload_pactl_module(&source_id)?;
    }
    if !sink_id.is_empty() {
        unload_pactl_module(&sink_id)?;
    }

    let _ = fs::remove_file(&config.state_path);

    if source_exists(&config.source_name)? {
        return Err(anyhow::anyhow!(
            "virtual mic still exists after remove: {}",
            config.source_name
        ));
    }

    info!(
        sink = %config.sink_name,
        source = %config.source_name,
        "virtual mic removed"
    );
    Ok(())
}

fn read_virtual_mic_state(path: &PathBuf) -> (String, String) {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut sink_id = String::new();
    let mut source_id = String::new();
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("SINK_MODULE_ID=") {
            sink_id = value.trim_matches('"').to_string();
        } else if let Some(value) = line.strip_prefix("SOURCE_MODULE_ID=") {
            source_id = value.trim_matches('"').to_string();
        }
    }
    (sink_id, source_id)
}

fn write_virtual_mic_state(
    config: &VirtualMicConfig,
    sink_id: &str,
    source_id: &str,
) -> Result<()> {
    let payload = format!(
        "SINK_MODULE_ID=\"{sink_id}\"\nSOURCE_MODULE_ID=\"{source_id}\"\nSINK_NAME=\"{}\"\nSOURCE_NAME=\"{}\"\n",
        config.sink_name, config.source_name
    );
    if let Some(parent) = config.state_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&config.state_path, payload)
        .map_err(|err| anyhow::anyhow!("write virtual mic state failed: {err}"))?;
    Ok(())
}

fn find_module_id(module: &str, needle: &str) -> Option<String> {
    let output = Command::new("pactl")
        .args(["list", "short", "modules"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let id = parts.next()?;
        let name = parts.next()?;
        if name != module {
            continue;
        }
        let rest = parts.collect::<Vec<_>>().join(" ");
        if rest.contains(needle) {
            return Some(id.to_string());
        }
    }
    None
}

fn load_pactl_module(module: &str, args: &[String]) -> Result<String> {
    let mut cmd = Command::new("pactl");
    cmd.arg("load-module").arg(module);
    for arg in args {
        cmd.arg(arg);
    }
    let output = cmd.output().map_err(|err| anyhow::anyhow!("pactl 不可用: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "pactl load-module failed: {}",
            stderr.trim()
        ));
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        return Err(anyhow::anyhow!(
            "pactl load-module returned empty module id"
        ));
    }
    Ok(id)
}

fn unload_pactl_module(id: &str) -> Result<()> {
    if id.trim().is_empty() {
        return Ok(());
    }
    let output = Command::new("pactl")
        .args(["unload-module", id])
        .output()
        .map_err(|err| anyhow::anyhow!("pactl 不可用: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("pactl unload-module failed: {}", stderr.trim()));
    }
    Ok(())
}

fn source_exists(name: &str) -> Result<bool> {
    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .map_err(|err| anyhow::anyhow!("pactl 不可用: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("pactl 执行失败: {}", stderr.trim()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let _index = parts.next();
        let source = parts.next();
        if source == Some(name) {
            return Ok(true);
        }
    }
    Ok(false)
}

struct ReceiverMetrics {
    packets_total: u64,
    bytes_total: u64,
    control_packets: u64,
    audio_packets: u64,
    unknown_packets: u64,
    empty_packets: u64,
    busy_rejects: u64,
    read_timeouts: u64,
    buffer_depth_frames: usize,
    audio_level_peak: u32,
    audio_level_sum_squares: u64,
    audio_level_samples: u64,
    buffer_target_ms: u64,
    sample_rate_hz: u32,
    channels: u16,
    last_consume_at: Instant,
    last_report_at: Instant,
}

impl ReceiverMetrics {
    fn new(now: Instant, sample_rate_hz: u32, channels: u16) -> Self {
        Self {
            packets_total: 0,
            bytes_total: 0,
            control_packets: 0,
            audio_packets: 0,
            unknown_packets: 0,
            empty_packets: 0,
            busy_rejects: 0,
            read_timeouts: 0,
            buffer_depth_frames: 0,
            audio_level_peak: 0,
            audio_level_sum_squares: 0,
            audio_level_samples: 0,
            buffer_target_ms: BUFFER_TARGET_MS,
            sample_rate_hz,
            channels,
            last_consume_at: now,
            last_report_at: now,
        }
    }

    fn on_datagram(&mut self, kind: DatagramKind, payload: &[u8]) {
        self.packets_total += 1;
        self.bytes_total += payload.len() as u64;
        match kind {
            DatagramKind::ControlJson => self.control_packets += 1,
            DatagramKind::AudioPcm16 => {
                self.audio_packets += 1;
                self.push_audio_payload(payload);
            }
            DatagramKind::Unknown(_) => self.unknown_packets += 1,
        }
    }

    fn on_empty(&mut self) {
        self.empty_packets += 1;
    }

    fn on_busy_reject(&mut self) {
        self.busy_rejects += 1;
    }

    fn on_timeout(&mut self, now: Instant) {
        self.read_timeouts += 1;
        self.consume_buffer(now);
    }

    fn report_if_due(
        &mut self,
        now: Instant,
        state: ConnectionState,
        active_client: Option<SocketAddr>,
        last_packet_at: Option<Instant>,
    ) {
        self.consume_buffer(now);
        if now.duration_since(self.last_report_at) < Duration::from_secs(METRICS_REPORT_SECS) {
            return;
        }
        let (audio_rms, audio_peak) = self.take_audio_level_snapshot();
        let buffer_depth_ms = self.buffer_depth_ms();
        let idle_ms = last_packet_at
            .map(|ts| now.duration_since(ts).as_millis() as u64)
            .unwrap_or(0);
        info!(
            state = state.as_str(),
            active = ?active_client,
            packets_total = self.packets_total,
            bytes_total = self.bytes_total,
            control_packets = self.control_packets,
            audio_packets = self.audio_packets,
            unknown_packets = self.unknown_packets,
            empty_packets = self.empty_packets,
            busy_rejects = self.busy_rejects,
            read_timeouts = self.read_timeouts,
            buffer_depth_frames = self.buffer_depth_frames,
            buffer_depth_ms,
            buffer_target_ms = self.buffer_target_ms,
            audio_rms,
            audio_peak,
            reconnect_window_secs = RECONNECT_WINDOW_SECS,
            idle_ms,
            "receiver metrics snapshot"
        );
        self.last_report_at = now;
    }

    fn push_audio_payload(&mut self, payload: &[u8]) {
        self.push_audio_frames(payload.len());
        self.accumulate_audio_levels(payload);
    }

    fn push_audio_frames(&mut self, payload_len: usize) {
        let bytes_per_frame = (self.channels.max(1) as usize) * 2;
        let frames = payload_len / bytes_per_frame;
        if frames > 0 {
            self.buffer_depth_frames = self.buffer_depth_frames.saturating_add(frames);
        }
    }

    fn accumulate_audio_levels(&mut self, payload: &[u8]) {
        for chunk in payload.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
            let abs_sample = sample.unsigned_abs();
            if abs_sample > self.audio_level_peak {
                self.audio_level_peak = abs_sample;
            }
            self.audio_level_sum_squares = self
                .audio_level_sum_squares
                .saturating_add((sample as i64 * sample as i64) as u64);
            self.audio_level_samples = self.audio_level_samples.saturating_add(1);
        }
    }

    fn take_audio_level_snapshot(&mut self) -> (f32, u32) {
        let rms = if self.audio_level_samples > 0 {
            let mean_square = self.audio_level_sum_squares as f64 / self.audio_level_samples as f64;
            mean_square.sqrt() as f32
        } else {
            0.0
        };
        let peak = self.audio_level_peak;
        self.audio_level_peak = 0;
        self.audio_level_sum_squares = 0;
        self.audio_level_samples = 0;
        (rms, peak)
    }

    fn consume_buffer(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_consume_at);
        if elapsed.is_zero() {
            return;
        }
        let sample_rate_hz = self.sample_rate_hz as u64;
        if sample_rate_hz == 0 {
            self.last_consume_at = now;
            return;
        }
        let frames_to_consume =
            (sample_rate_hz.saturating_mul(elapsed.as_millis() as u64) / 1000) as usize;
        if frames_to_consume > 0 {
            self.buffer_depth_frames = self.buffer_depth_frames.saturating_sub(frames_to_consume);
        }
        self.last_consume_at = now;
    }

    fn buffer_depth_ms(&self) -> u64 {
        let sample_rate_hz = self.sample_rate_hz as u64;
        if sample_rate_hz == 0 {
            return 0;
        }
        (self.buffer_depth_frames as u64)
            .saturating_mul(1000)
            .saturating_div(sample_rate_hz)
    }

    fn build_stats_snapshot(&mut self) -> StatsSnapshot {
        let (audio_rms, audio_peak) = self.take_audio_level_snapshot();
        let buffer_depth_ms = self.buffer_depth_ms();
        StatsSnapshot {
            packets_received: self.audio_packets,
            packets_lost: 0,
            buffer_depth_frames: self.buffer_depth_frames as u64,
            buffer_depth_ms,
            jitter_buffer_depth_ms: buffer_depth_ms as f32,
            estimated_e2e_latency_ms: buffer_depth_ms as f32,
            audio_rms,
            audio_peak,
        }
    }
}
