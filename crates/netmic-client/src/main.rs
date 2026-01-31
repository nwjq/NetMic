//! NetMic 客户端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 先使用共享默认参数，后续再接入采集/发送链路。

use std::env;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use netmic_client::{AudioPipeline, Pcm16Frame};
use netmic_proto::control::{
    decode_control_message, decode_control_payload, encode_control_message,
    CONTROL_TYPE_HANDSHAKE_REQUEST, CONTROL_TYPE_HANDSHAKE_RESPONSE, CONTROL_TYPE_HEARTBEAT,
};
use netmic_proto::datagram::{
    wrap_audio_pcm16_with_header, wrap_control_json, DatagramKind, DATAGRAM_KIND_AUDIO_PCM16,
    DATAGRAM_KIND_CONTROL_JSON,
};
use netmic_proto::protocol::{
    AudioFrameHeader, HandshakeRequest, HandshakeResponse, Heartbeat, SessionParams,
};
use tracing::{info, warn};

/// 与服务端骨架保持一致的默认 UDP 地址。
const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:43000";
/// 服务端地址环境变量（host:port）。
const ENV_SERVER_ADDR: &str = "NETMIC_SERVER_ADDR";
/// 是否发送演示数据的开关（1/true/on/yes）。
const ENV_DEMO_SEND: &str = "NETMIC_CLIENT_DEMO_SEND";
/// 演示流时长（秒，0 表示不限制）。
const ENV_STREAM_SECS: &str = "NETMIC_CLIENT_STREAM_SECS";
/// 心跳间隔（毫秒，0 表示禁用）。
const ENV_HEARTBEAT_INTERVAL_MS: &str = "NETMIC_CLIENT_HEARTBEAT_MS";
/// 重连窗口目标（MVP 要求 10 秒内恢复）。
const RECONNECT_WINDOW_SECS: u64 = 10;
/// 演示发送失败后的重试间隔。
const RECONNECT_BACKOFF_MS: u64 = 500;
/// 演示发送最多重试次数（占位）。
const MAX_RECONNECT_ATTEMPTS: usize = 3;
/// 默认心跳间隔（ms）。
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 1_000;
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
        if let Err(err) = stream_with_reconnect(&params) {
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

/// 发送持续流（含心跳）+ 重连骨架。
fn stream_with_reconnect(params: &SessionParams) -> Result<(), String> {
    let server_addr = server_addr_from_env();
    let socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|err| format!("bind udp socket failed: {err}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(800)))
        .map_err(|err| format!("set udp timeout failed: {err}"))?;

    let heartbeat_interval = heartbeat_interval_from_env();
    let stream_limit = stream_duration_from_env();
    let mut ctx = ClientContext::new();
    let mut attempts = 0;

    loop {
        ctx.transition_to(ClientConnectionState::Connecting, "start handshake");
        match handshake_and_stream(
            &socket,
            &server_addr,
            params,
            &mut ctx.metrics,
            heartbeat_interval,
            stream_limit,
        ) {
            Ok(()) => {
                ctx.transition_to(ClientConnectionState::Active, "stream finished");
                ctx.maybe_report();
                return Ok(());
            }
            Err(err) => {
                ctx.metrics.on_send_error();
                ctx.transition_to(
                    ClientConnectionState::Reconnecting,
                    "stream failed, retrying",
                );
                attempts += 1;
                if attempts > MAX_RECONNECT_ATTEMPTS {
                    ctx.maybe_report();
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(RECONNECT_BACKOFF_MS));
            }
        }
    }
}

/// 构造握手请求 datagram（kind=0）。
fn build_handshake_datagram(request: &HandshakeRequest) -> Result<Vec<u8>, String> {
    let payload = encode_control_message(CONTROL_TYPE_HANDSHAKE_REQUEST, request)
        .map_err(|err| format!("encode handshake request failed: {err}"))?;
    Ok(wrap_control_json(&payload))
}

fn build_heartbeat_datagram(heartbeat: &Heartbeat) -> Result<Vec<u8>, String> {
    let payload = encode_control_message(CONTROL_TYPE_HEARTBEAT, heartbeat)
        .map_err(|err| format!("encode heartbeat failed: {err}"))?;
    Ok(wrap_control_json(&payload))
}

/// 构造数据面 datagram（kind=1）。
fn build_audio_datagram(
    frame: &Pcm16Frame,
    session_id: &str,
    seq: u64,
    timestamp_ms: u64,
) -> Result<Vec<u8>, String> {
    let payload = frame.to_bytes();
    let frame_samples = (frame.samples.len() as u32) / (frame.channels as u32).max(1);
    let header = AudioFrameHeader {
        session_id: session_id.to_string(),
        seq,
        timestamp_ms,
        frame_samples,
    };
    wrap_audio_pcm16_with_header(&header, &payload)
        .map_err(|err| format!("build audio datagram failed: {err}"))
}

fn handshake_and_stream(
    socket: &UdpSocket,
    server_addr: &str,
    params: &SessionParams,
    metrics: &mut ClientMetrics,
    heartbeat_interval: Option<Duration>,
    stream_limit: Option<Duration>,
) -> Result<(), String> {
    let request = HandshakeRequest {
        session_id: format!("session-{}", now_ms()),
        client_name: "netmic-client".to_string(),
        requested: params.clone(),
        token: None,
    };
    let control = build_handshake_datagram(&request)?;
    let control_len = socket
        .send_to(&control, server_addr)
        .map_err(|err| format!("send control datagram failed: {err}"))?;
    metrics.on_send(DatagramKind::ControlJson, control_len);
    info!(
        %server_addr,
        bytes = control_len,
        kind = DATAGRAM_KIND_CONTROL_JSON,
        "sent control datagram (handshake request)"
    );

    let response = wait_for_handshake_response(socket, &request.session_id)?;
    if !response.accepted || response.busy {
        return Err("server rejected handshake (busy)".to_string());
    }
    info!(?response.effective, "handshake accepted with effective params");

    let mut pipeline = AudioPipeline::new(&response.effective)
        .map_err(|err| format!("init audio pipeline failed: {err}"))?;
    let session_id = response.session_id.clone();
    let mut seq: u64 = 0;
    let mut heartbeat_seq: u64 = 0;
    let start_at = Instant::now();
    let mut next_heartbeat = heartbeat_interval.map(|interval| Instant::now() + interval);

    loop {
        if let Some(limit) = stream_limit {
            if start_at.elapsed() >= limit {
                info!("stream duration reached, stop streaming");
                break;
            }
        }

        let frame = pipeline
            .next_frame()
            .map_err(|err| format!("capture/resample failed: {err}"))?;
        let current_seq = seq;
        let audio = build_audio_datagram(&frame, &session_id, current_seq, now_ms())?;
        seq = seq.saturating_add(1);
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
            seq = current_seq,
            "sent audio datagram (pcm16)"
        );

        if let Some(next) = next_heartbeat {
            if Instant::now() >= next {
                let heartbeat = Heartbeat {
                    session_id: session_id.clone(),
                    seq: heartbeat_seq,
                    sent_at_ms: now_ms(),
                };
                heartbeat_seq = heartbeat_seq.saturating_add(1);
                let datagram = build_heartbeat_datagram(&heartbeat)?;
                let bytes = socket
                    .send_to(&datagram, server_addr)
                    .map_err(|err| format!("send heartbeat failed: {err}"))?;
                metrics.on_send(DatagramKind::ControlJson, bytes);
                next_heartbeat = heartbeat_interval.map(|interval| Instant::now() + interval);
            }
        }

        metrics.report_if_due(ClientConnectionState::Active);
        std::thread::sleep(pipeline.frame_interval());
    }

    Ok(())
}

fn wait_for_handshake_response(
    socket: &UdpSocket,
    session_id: &str,
) -> Result<HandshakeResponse, String> {
    let mut buf = [0_u8; 2048];
    let (len, _addr) = socket
        .recv_from(&mut buf)
        .map_err(|err| format!("recv handshake response failed: {err}"))?;
    let (kind, payload) = netmic_proto::datagram::split_datagram(&buf[..len])
        .ok_or_else(|| "invalid datagram (empty)".to_string())?;
    if kind != DatagramKind::ControlJson {
        return Err("unexpected datagram kind while waiting for handshake response".to_string());
    }
    let (msg_type, payload_value) =
        decode_control_message(payload).map_err(|err| format!("decode control failed: {err}"))?;
    if msg_type.as_str() != CONTROL_TYPE_HANDSHAKE_RESPONSE {
        return Err(format!("unexpected control message type: {msg_type}"));
    }
    let response: HandshakeResponse =
        decode_control_payload(payload_value).map_err(|err| format!("{err}"))?;
    if response.session_id != session_id {
        return Err("handshake response session_id mismatch".to_string());
    }
    Ok(response)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

fn heartbeat_interval_from_env() -> Option<Duration> {
    match env::var(ENV_HEARTBEAT_INTERVAL_MS) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(0) => None,
            Ok(ms) => Some(Duration::from_millis(ms)),
            Err(err) => {
                warn!(%raw, %err, "invalid heartbeat interval, fallback to default");
                Some(Duration::from_millis(DEFAULT_HEARTBEAT_INTERVAL_MS))
            }
        },
        Err(_) => Some(Duration::from_millis(DEFAULT_HEARTBEAT_INTERVAL_MS)),
    }
}

fn stream_duration_from_env() -> Option<Duration> {
    match env::var(ENV_STREAM_SECS) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(err) => {
                warn!(%raw, %err, "invalid stream duration, ignore");
                None
            }
        },
        Err(_) => None,
    }
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
        build_audio_datagram, build_handshake_datagram, build_heartbeat_datagram, now_ms,
        wait_for_handshake_response, DATAGRAM_KIND_AUDIO_PCM16, DATAGRAM_KIND_CONTROL_JSON,
    };
    use netmic_client::Pcm16Frame;
    use netmic_proto::control::{
        decode_control_message, decode_control_payload, encode_control_message,
        CONTROL_TYPE_HANDSHAKE_REQUEST, CONTROL_TYPE_HANDSHAKE_RESPONSE,
    };
    use netmic_proto::datagram::{
        split_audio_pcm16_with_header, split_datagram, wrap_control_json, DatagramKind,
    };
    use netmic_proto::protocol::{
        AudioFrameHeader, HandshakeRequest, HandshakeResponse, Heartbeat, SessionParams,
    };
    use std::net::UdpSocket;
    use std::time::Duration;

    #[test]
    fn control_datagram_prefixes_kind_and_is_splitable() {
        let params = SessionParams::mvp_default();
        let request = HandshakeRequest {
            session_id: "session-1".to_string(),
            client_name: "client".to_string(),
            requested: params,
            token: None,
        };
        let datagram = build_handshake_datagram(&request).expect("control datagram");
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_CONTROL_JSON));

        let (kind, payload) = split_datagram(&datagram).expect("split control datagram");
        assert_eq!(kind, DatagramKind::ControlJson);
        assert!(!payload.is_empty());
    }

    #[test]
    fn audio_datagram_prefixes_kind_and_preserves_payload() {
        let frame = Pcm16Frame {
            samples: vec![0_i16, 1024_i16],
            sample_rate_hz: 48_000,
            channels: 1,
        };
        let payload = frame.to_bytes();
        let datagram = build_audio_datagram(&frame, "session-1", 7, 1234).expect("audio");
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_AUDIO_PCM16));

        let (kind, split_payload) = split_datagram(&datagram).expect("split audio datagram");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        let (header, pcm) = split_audio_pcm16_with_header(split_payload).expect("header");
        assert_eq!(header.session_id, "session-1");
        assert_eq!(header.seq, 7);
        assert_eq!(header.timestamp_ms, 1234);
        assert_eq!(header.frame_samples, 2);
        assert_eq!(pcm, payload.as_slice());
    }

    #[test]
    fn udp_send_emits_control_and_audio_kinds() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("set timeout");
        let recv_addr = receiver.local_addr().expect("receiver addr");

        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");
        let params = SessionParams::mvp_default();
        let request = HandshakeRequest {
            session_id: "session-2".to_string(),
            client_name: "client".to_string(),
            requested: params,
            token: None,
        };
        let control = build_handshake_datagram(&request).expect("control datagram");

        let frame = Pcm16Frame {
            samples: vec![1_i16, -1_i16, 2_i16, -2_i16],
            sample_rate_hz: 48_000,
            channels: 1,
        };
        let audio = build_audio_datagram(&frame, "session-2", 0, 1000).expect("audio datagram");

        sender.send_to(&control, recv_addr).expect("send control");
        sender.send_to(&audio, recv_addr).expect("send audio 1");
        sender.send_to(&audio, recv_addr).expect("send audio 2");

        let mut control_count = 0;
        let mut audio_count = 0;
        let mut buf = [0_u8; 1500];

        for _ in 0..3 {
            let (len, _addr) = receiver.recv_from(&mut buf).expect("recv datagram");
            let packet = &buf[..len];
            let (kind, _payload) = split_datagram(packet).expect("split datagram");
            match kind {
                DatagramKind::ControlJson => control_count += 1,
                DatagramKind::AudioPcm16 => audio_count += 1,
                DatagramKind::Unknown(other) => panic!("unexpected datagram kind: {other}"),
            }
        }

        assert_eq!(control_count, 1);
        assert_eq!(audio_count, 2);
    }

    #[test]
    fn wait_for_handshake_response_parses_control_reply() {
        let receiver = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
        receiver
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("set timeout");
        let addr = receiver.local_addr().expect("addr");
        let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender");

        let response = HandshakeResponse {
            session_id: "session-test".to_string(),
            accepted: true,
            reason: None,
            effective: SessionParams::mvp_default(),
            busy: false,
        };
        let payload =
            encode_control_message(CONTROL_TYPE_HANDSHAKE_RESPONSE, &response).expect("encode");
        let datagram = wrap_control_json(&payload);
        sender.send_to(&datagram, addr).expect("send response");

        let parsed = wait_for_handshake_response(&receiver, "session-test").expect("parsed");
        assert!(parsed.accepted);
        assert_eq!(parsed.session_id, "session-test");
    }

    #[test]
    fn heartbeat_datagram_roundtrip() {
        let heartbeat = Heartbeat {
            session_id: "session-hb".to_string(),
            seq: 7,
            sent_at_ms: 123,
        };
        let datagram = build_heartbeat_datagram(&heartbeat).expect("heartbeat");
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_CONTROL_JSON));
        let (kind, payload) = split_datagram(&datagram).expect("split");
        assert_eq!(kind, DatagramKind::ControlJson);
        let (msg_type, payload_value) = decode_control_message(payload).expect("decode");
        assert_eq!(msg_type, "heartbeat");
        let decoded: Heartbeat = decode_control_payload(payload_value).expect("payload");
        assert_eq!(decoded.session_id, "session-hb");
        assert_eq!(decoded.seq, 7);
        assert_eq!(decoded.sent_at_ms, 123);
    }

    #[test]
    fn handshake_control_payload_roundtrip() {
        let request = HandshakeRequest {
            session_id: format!("session-{}", now_ms()),
            client_name: "client".to_string(),
            requested: SessionParams::mvp_default(),
            token: None,
        };
        let payload =
            encode_control_message(CONTROL_TYPE_HANDSHAKE_REQUEST, &request).expect("encode");
        let (msg_type, payload_value) = decode_control_message(&payload).expect("decode");
        assert_eq!(msg_type, CONTROL_TYPE_HANDSHAKE_REQUEST);
        let decoded: HandshakeRequest =
            decode_control_payload(payload_value).expect("payload decode");
        assert_eq!(decoded.session_id, request.session_id);
    }

    #[test]
    fn audio_header_is_encoded_and_decoded() {
        let header = AudioFrameHeader {
            session_id: "session-xyz".to_string(),
            seq: 99,
            timestamp_ms: 555,
            frame_samples: 480,
        };
        let frame = Pcm16Frame {
            samples: vec![0_i16; 480],
            sample_rate_hz: 48_000,
            channels: 1,
        };
        let datagram =
            build_audio_datagram(&frame, &header.session_id, header.seq, header.timestamp_ms)
                .expect("audio datagram");
        let (kind, payload) = split_datagram(&datagram).expect("split");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        let (decoded, pcm) = split_audio_pcm16_with_header(payload).expect("decode");
        assert_eq!(decoded.session_id, header.session_id);
        assert_eq!(decoded.seq, header.seq);
        assert_eq!(decoded.timestamp_ms, header.timestamp_ms);
        assert_eq!(decoded.frame_samples, header.frame_samples);
        assert_eq!(pcm.len(), frame.samples.len() * 2);
    }
}
