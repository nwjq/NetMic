//! NetMic 客户端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 先使用共享默认参数，后续再接入采集/发送链路。

use std::env;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

mod audio;

use audio::{AudioPipeline, Pcm16Frame};
use netmic_proto::datagram::{
    wrap_audio_pcm16, wrap_control_json, DatagramKind, DATAGRAM_KIND_AUDIO_PCM16,
    DATAGRAM_KIND_CONTROL_JSON,
};
use netmic_proto::protocol::SessionParams;
use tracing::{info, warn};

/// 与服务端骨架保持一致的默认 UDP 地址。
const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:43000";
/// 服务端地址环境变量（host:port）。
const ENV_SERVER_ADDR: &str = "NETMIC_SERVER_ADDR";
/// 是否发送演示数据的开关（1/true/on/yes）。
const ENV_DEMO_SEND: &str = "NETMIC_CLIENT_DEMO_SEND";
/// 重连窗口目标（MVP 要求 10 秒内恢复）。
const RECONNECT_WINDOW_SECS: u64 = 10;
/// 演示发送失败后的重试间隔。
const RECONNECT_BACKOFF_MS: u64 = 500;
/// 演示发送最多重试次数（占位）。
const MAX_RECONNECT_ATTEMPTS: usize = 3;
/// 演示发送帧数（占位）。
const DEMO_AUDIO_FRAMES: usize = 5;
/// 指标输出间隔（占位）。
const METRICS_REPORT_SECS: u64 = 5;
/// 发送侧缓冲深度（占位，后续接入采集队列）。
const SEND_BUFFER_DEPTH_FRAMES: usize = 0;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-client skeleton started");
    println!("netmic-client skeleton ready");

    if should_demo_send() {
        if let Err(err) = send_demo_packets(&params) {
            warn!(%err, "failed to send demo datagrams");
        }
    } else {
        info!(
            env = ENV_DEMO_SEND,
            "demo send disabled (set NETMIC_CLIENT_DEMO_SEND=1 to emit kind-framed UDP packets)"
        );
    }
}

/// 判断是否启用演示发送。
fn should_demo_send() -> bool {
    match env::var(ENV_DEMO_SEND) {
        Ok(raw) => matches!(raw.as_str(), "1" | "true" | "on" | "yes"),
        Err(_) => false,
    }
}

/// 解析服务端地址（默认回落到本机骨架端口）。
fn server_addr_from_env() -> String {
    env::var(ENV_SERVER_ADDR).unwrap_or_else(|_| DEFAULT_SERVER_ADDR.to_string())
}

/// 发送最小控制面 + 数据面占位包。
///
/// 目标：
/// - client 侧明确复用 `netmic-proto` 的 kind framing；
/// - 便于手工联调：`NETMIC_CLIENT_DEMO_SEND=1 cargo run -p netmic-client`。
fn send_demo_packets(params: &SessionParams) -> Result<(), String> {
    let server_addr = server_addr_from_env();
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind udp socket failed: {err}"))?;
    let mut ctx = ClientContext::new();
    let mut pipeline =
        AudioPipeline::new(params).map_err(|err| format!("init audio pipeline failed: {err}"))?;

    ctx.transition_to(ClientConnectionState::Connecting, "start demo send");

    for attempt in 0..=MAX_RECONNECT_ATTEMPTS {
        match send_control_and_audio(
            &socket,
            &server_addr,
            params,
            &mut ctx.metrics,
            &mut pipeline,
        ) {
            Ok(()) => {
                ctx.transition_to(ClientConnectionState::Active, "demo datagrams sent");
                ctx.maybe_report();
                return Ok(());
            }
            Err(err) => {
                ctx.metrics.on_send_error();
                ctx.transition_to(ClientConnectionState::Reconnecting, "send failed, retrying");
                if attempt >= MAX_RECONNECT_ATTEMPTS {
                    ctx.maybe_report();
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(RECONNECT_BACKOFF_MS));
            }
        }
    }

    Ok(())
}

/// 构造控制面 datagram（kind=0）。
fn build_control_datagram(params: &SessionParams) -> Result<Vec<u8>, serde_json::Error> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "type": "hello",
        "params": params,
    }))?;
    Ok(wrap_control_json(&payload))
}

/// 构造数据面 datagram（kind=1）。
fn build_audio_datagram(frame: &Pcm16Frame) -> Vec<u8> {
    let payload = frame.to_bytes();
    wrap_audio_pcm16(&payload)
}

fn send_control_and_audio(
    socket: &UdpSocket,
    server_addr: &str,
    params: &SessionParams,
    metrics: &mut ClientMetrics,
    pipeline: &mut AudioPipeline,
) -> Result<(), String> {
    let control = build_control_datagram(params)
        .map_err(|err| format!("build control datagram failed: {err}"))?;
    let control_len = socket
        .send_to(&control, server_addr)
        .map_err(|err| format!("send control datagram failed: {err}"))?;
    metrics.on_send(DatagramKind::ControlJson, control_len);
    info!(
        %server_addr,
        bytes = control_len,
        kind = DATAGRAM_KIND_CONTROL_JSON,
        "sent control datagram (json placeholder)"
    );

    for idx in 0..DEMO_AUDIO_FRAMES {
        let frame = pipeline
            .next_frame()
            .map_err(|err| format!("capture/resample failed: {err}"))?;
        let audio = build_audio_datagram(&frame);
        let audio_len = socket
            .send_to(&audio, server_addr)
            .map_err(|err| format!("send audio datagram failed: {err}"))?;
        metrics.on_send(DatagramKind::AudioPcm16, audio_len);
        info!(
            %server_addr,
            bytes = audio_len,
            kind = DATAGRAM_KIND_AUDIO_PCM16,
            sample_rate_hz = frame.sample_rate_hz,
            channels = frame.channels,
            frame_index = idx,
            "sent audio datagram (pcm16 placeholder)"
        );
        if idx + 1 < DEMO_AUDIO_FRAMES {
            std::thread::sleep(pipeline.frame_interval());
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientConnectionState {
    Idle,
    Connecting,
    Active,
    Reconnecting,
}

impl ClientConnectionState {
    fn as_str(self) -> &'static str {
        match self {
            ClientConnectionState::Idle => "idle",
            ClientConnectionState::Connecting => "connecting",
            ClientConnectionState::Active => "active",
            ClientConnectionState::Reconnecting => "reconnecting",
        }
    }
}

struct ClientContext {
    state: ClientConnectionState,
    metrics: ClientMetrics,
}

impl ClientContext {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            state: ClientConnectionState::Idle,
            metrics: ClientMetrics::new(now),
        }
    }

    fn transition_to(&mut self, next: ClientConnectionState, reason: &str) {
        if self.state == next {
            return;
        }
        info!(
            from = self.state.as_str(),
            to = next.as_str(),
            reason,
            "client state transition"
        );
        self.state = next;
    }

    fn maybe_report(&mut self) {
        self.metrics.report_if_due(self.state);
    }
}

struct ClientMetrics {
    sent_packets: u64,
    sent_bytes: u64,
    control_packets: u64,
    audio_packets: u64,
    send_errors: u64,
    estimated_loss: u64,
    send_buffer_depth_frames: usize,
    last_send_at: Option<Instant>,
    last_report_at: Instant,
}

impl ClientMetrics {
    fn new(now: Instant) -> Self {
        Self {
            sent_packets: 0,
            sent_bytes: 0,
            control_packets: 0,
            audio_packets: 0,
            send_errors: 0,
            estimated_loss: 0,
            send_buffer_depth_frames: SEND_BUFFER_DEPTH_FRAMES,
            last_send_at: None,
            last_report_at: now,
        }
    }

    fn on_send(&mut self, kind: DatagramKind, bytes: usize) {
        self.sent_packets += 1;
        self.sent_bytes += bytes as u64;
        self.last_send_at = Some(Instant::now());
        match kind {
            DatagramKind::ControlJson => self.control_packets += 1,
            DatagramKind::AudioPcm16 => self.audio_packets += 1,
            DatagramKind::Unknown(_) => {}
        }
    }

    fn on_send_error(&mut self) {
        self.send_errors += 1;
    }

    fn report_if_due(&mut self, state: ClientConnectionState) {
        let now = Instant::now();
        if now.duration_since(self.last_report_at) < Duration::from_secs(METRICS_REPORT_SECS) {
            return;
        }
        let idle_ms = self
            .last_send_at
            .map(|ts| now.duration_since(ts).as_millis() as u64)
            .unwrap_or(0);
        info!(
            state = state.as_str(),
            sent_packets = self.sent_packets,
            sent_bytes = self.sent_bytes,
            control_packets = self.control_packets,
            audio_packets = self.audio_packets,
            send_errors = self.send_errors,
            estimated_loss = self.estimated_loss,
            send_buffer_depth_frames = self.send_buffer_depth_frames,
            reconnect_window_secs = RECONNECT_WINDOW_SECS,
            idle_ms,
            "client metrics snapshot"
        );
        self.last_report_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_audio_datagram, build_control_datagram, DATAGRAM_KIND_AUDIO_PCM16,
        DATAGRAM_KIND_CONTROL_JSON,
    };
    use crate::audio::Pcm16Frame;
    use netmic_proto::datagram::{split_datagram, DatagramKind};
    use netmic_proto::protocol::SessionParams;

    #[test]
    fn control_datagram_prefixes_kind_and_is_splitable() {
        let params = SessionParams::mvp_default();
        let datagram = build_control_datagram(&params).expect("control datagram");
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_CONTROL_JSON));

        let (kind, payload) = split_datagram(&datagram).expect("split control datagram");
        assert_eq!(kind, DatagramKind::ControlJson);
        assert!(!payload.is_empty());
    }

    #[test]
    fn audio_datagram_prefixes_kind_and_preserves_payload() {
        let frame = Pcm16Frame::new(vec![0_i16, 1024_i16], 48_000, 1);
        let payload = frame.to_bytes();
        let datagram = build_audio_datagram(&frame);
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_AUDIO_PCM16));

        let (kind, split_payload) = split_datagram(&datagram).expect("split audio datagram");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        assert_eq!(split_payload, payload.as_slice());
    }
}
