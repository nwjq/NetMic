//! NetMic 客户端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 先使用共享默认参数，后续再接入采集/发送链路。

use std::env;
use std::net::UdpSocket;

use netmic_proto::datagram::{
    wrap_audio_pcm16, wrap_control_json, DATAGRAM_KIND_AUDIO_PCM16, DATAGRAM_KIND_CONTROL_JSON,
};
use netmic_proto::protocol::SessionParams;
use tracing::{info, warn};

/// 与服务端骨架保持一致的默认 UDP 地址。
const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:43000";
/// 服务端地址环境变量（host:port）。
const ENV_SERVER_ADDR: &str = "NETMIC_SERVER_ADDR";
/// 是否发送演示数据的开关（1/true/on/yes）。
const ENV_DEMO_SEND: &str = "NETMIC_CLIENT_DEMO_SEND";

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
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("bind udp socket failed: {err}"))?;

    let control = build_control_datagram(params)
        .map_err(|err| format!("build control datagram failed: {err}"))?;
    let control_len = socket
        .send_to(&control, &server_addr)
        .map_err(|err| format!("send control datagram failed: {err}"))?;
    info!(
        %server_addr,
        bytes = control_len,
        kind = DATAGRAM_KIND_CONTROL_JSON,
        "sent control datagram (json placeholder)"
    );

    let audio_payload = demo_pcm16_payload();
    let audio = build_audio_datagram(&audio_payload);
    let audio_len = socket
        .send_to(&audio, &server_addr)
        .map_err(|err| format!("send audio datagram failed: {err}"))?;
    info!(
        %server_addr,
        bytes = audio_len,
        kind = DATAGRAM_KIND_AUDIO_PCM16,
        "sent audio datagram (pcm16 placeholder)"
    );

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
fn build_audio_datagram(payload: &[u8]) -> Vec<u8> {
    wrap_audio_pcm16(payload)
}

/// 生成一小段 PCM16 占位数据（小端序）。
fn demo_pcm16_payload() -> Vec<u8> {
    // 这里用两帧简单样本：0 与 1024（便于在日志中观察长度）。
    let samples = [0_i16, 1024_i16];
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::{
        build_audio_datagram, build_control_datagram, demo_pcm16_payload,
        DATAGRAM_KIND_AUDIO_PCM16, DATAGRAM_KIND_CONTROL_JSON,
    };
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
        let payload = demo_pcm16_payload();
        let datagram = build_audio_datagram(&payload);
        assert_eq!(datagram.first().copied(), Some(DATAGRAM_KIND_AUDIO_PCM16));

        let (kind, split_payload) = split_datagram(&datagram).expect("split audio datagram");
        assert_eq!(kind, DatagramKind::AudioPcm16);
        assert_eq!(split_payload, payload.as_slice());
    }
}
