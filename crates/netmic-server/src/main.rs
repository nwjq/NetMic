//! NetMic 服务端入口（M1：UDP 接收骨架）。
//!
//! 设计说明（与文档对齐）：
//! - 协议事实来源：`docs/PROTO.md` 与 `MVP.md#5`。
//! - 先提供“可监听 + 可分流 + 单客户端占位”的最小正确性。
//! - 控制面/数据面解析均为占位实现，后续再接入真实握手/解码/注入。

use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use netmic_proto::datagram::{split_datagram, DatagramKind};
use netmic_proto::protocol::SessionParams;
use tracing::{debug, info, warn};

/// 默认 UDP 监听端口（MVP 占位值，后续可统一到配置模块）。
const DEFAULT_UDP_PORT: u16 = 43_000;
/// 默认绑定地址（监听所有网卡）。
const DEFAULT_BIND_ADDR: &str = "0.0.0.0";
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

fn main() -> Result<()> {
    init_tracing();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-server starting with MVP defaults");

    let port = udp_port_from_env();
    let bind_addr = format!("{DEFAULT_BIND_ADDR}:{port}");
    let socket = bind_udp_socket(&bind_addr)?;

    info!(%bind_addr, "udp receiver skeleton ready");
    run_receiver_loop(&socket)
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

fn bind_udp_socket(bind_addr: &str) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(bind_addr)
        .with_context(|| format!("failed to bind UDP socket at {bind_addr}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
        .context("failed to set UDP read timeout")?;
    Ok(socket)
}

fn run_receiver_loop(socket: &UdpSocket) -> Result<()> {
    let mut buf = [0_u8; MAX_DATAGRAM_SIZE];
    let mut ctx = ReceiverContext::new();

    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let packet = &buf[..len];
                let now = Instant::now();
                if !ctx.accept_or_lock_client(addr, now) {
                    continue;
                }
                if let Some((kind, payload)) = split_datagram(packet) {
                    ctx.metrics.on_datagram(kind, payload.len());
                    match kind {
                        DatagramKind::ControlJson => handle_control_payload(payload, addr),
                        DatagramKind::AudioPcm16 => {
                            info!(
                                %addr,
                                bytes = payload.len(),
                                kind = kind.as_str(),
                                "received audio payload (pcm16 placeholder)"
                            );
                        }
                        DatagramKind::Unknown(tag) => {
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

/// 控制面占位处理：
/// - 当前仅尝试解析 JSON 并读取 `type` 字段用于日志分流；
/// - 真正的握手/心跳/统计结构体解析将在后续里程碑接入。
fn handle_control_payload(payload: &[u8], addr: SocketAddr) {
    match serde_json::from_slice::<serde_json::Value>(payload) {
        Ok(value) => {
            let msg_type = value
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            info!(%addr, msg_type, "received control json payload");
        }
        Err(err) => {
            warn!(
                %addr,
                %err,
                bytes = payload.len(),
                "failed to parse control json payload"
            );
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
    last_packet_at: Option<Instant>,
    metrics: ReceiverMetrics,
}

impl ReceiverContext {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            state: ConnectionState::Idle,
            active_client: None,
            last_packet_at: None,
            metrics: ReceiverMetrics::new(now),
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
                self.last_packet_at = Some(now);
                self.transition_to(ConnectionState::Active, "lock active client");
                true
            }
        }
    }

    fn on_timeout_tick(&mut self, now: Instant) {
        self.metrics.on_timeout();
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
            self.transition_to(ConnectionState::Reconnecting, "no packets, enter reconnecting");
        }

        if idle_for >= Duration::from_secs(RECONNECT_WINDOW_SECS)
            && self.state == ConnectionState::Reconnecting
        {
            let released = self.active_client.take();
            self.last_packet_at = None;
            self.transition_to(ConnectionState::Idle, "reconnect window exceeded, release client");
            if let Some(addr) = released {
                info!(%addr, "active client released after reconnect timeout");
            }
        }
    }

    fn maybe_report(&mut self, now: Instant) {
        self.metrics.report_if_due(
            now,
            self.state,
            self.active_client,
            self.last_packet_at,
        );
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
    buffer_target_ms: u64,
    last_report_at: Instant,
}

impl ReceiverMetrics {
    fn new(now: Instant) -> Self {
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
            buffer_target_ms: BUFFER_TARGET_MS,
            last_report_at: now,
        }
    }

    fn on_datagram(&mut self, kind: DatagramKind, payload_len: usize) {
        self.packets_total += 1;
        self.bytes_total += payload_len as u64;
        match kind {
            DatagramKind::ControlJson => self.control_packets += 1,
            DatagramKind::AudioPcm16 => self.audio_packets += 1,
            DatagramKind::Unknown(_) => self.unknown_packets += 1,
        }
    }

    fn on_empty(&mut self) {
        self.empty_packets += 1;
    }

    fn on_busy_reject(&mut self) {
        self.busy_rejects += 1;
    }

    fn on_timeout(&mut self) {
        self.read_timeouts += 1;
    }

    fn report_if_due(
        &mut self,
        now: Instant,
        state: ConnectionState,
        active_client: Option<SocketAddr>,
        last_packet_at: Option<Instant>,
    ) {
        if now.duration_since(self.last_report_at) < Duration::from_secs(METRICS_REPORT_SECS) {
            return;
        }
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
            buffer_target_ms = self.buffer_target_ms,
            reconnect_window_secs = RECONNECT_WINDOW_SECS,
            idle_ms,
            "receiver metrics snapshot"
        );
        self.last_report_at = now;
    }
}
