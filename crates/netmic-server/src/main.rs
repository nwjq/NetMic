//! NetMic 服务端入口（M1：UDP 接收骨架）。
//!
//! 设计说明（与文档对齐）：
//! - 协议事实来源：`docs/PROTO.md` 与 `MVP.md#5`。
//! - 先提供“可监听 + 可分流 + 单客户端占位”的最小正确性。
//! - 控制面/数据面解析均为占位实现，后续再接入真实握手/解码/注入。

use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

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
    let mut active_client: Option<SocketAddr> = None;

    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let packet = &buf[..len];
                if !accept_or_lock_client(&mut active_client, addr) {
                    continue;
                }
                handle_datagram(packet, addr);
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                debug!("udp recv timeout (no packets yet)");
            }
            Err(err) => {
                warn!(%err, "udp recv error");
            }
        }
    }
}

/// 单客户端占位策略：
/// - 首个发送方锁定为 active client；
/// - 其他来源直接拒绝并打日志（后续可回发 BUSY 控制消息）。
fn accept_or_lock_client(active_client: &mut Option<SocketAddr>, addr: SocketAddr) -> bool {
    match active_client {
        Some(current) if *current != addr => {
            warn!(%addr, active = %current, "reject packet from non-active client (busy)");
            false
        }
        Some(_) => true,
        None => {
            info!(%addr, "lock active client");
            *active_client = Some(addr);
            true
        }
    }
}

fn handle_datagram(packet: &[u8], addr: SocketAddr) {
    match split_datagram(packet) {
        Some((kind, payload)) => match kind {
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
        },
        None => {
            debug!(%addr, "received empty datagram");
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
