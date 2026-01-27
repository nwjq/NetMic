#!/usr/bin/env bash
# 初始化 NetMic Rust 工作区骨架（幂等）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  echo "[bootstrap_workspace] 缺少 cargo。请先安装 Rust 工具链：https://rustup.rs" >&2
  exit 2
fi

mkdir -p crates

create_crate_if_missing() {
  local kind="$1"
  local path="$2"
  local name="$3"
  if [[ -e "$path/Cargo.toml" ]]; then
    return 0
  fi
  case "$kind" in
    lib)
      cargo new --lib "$path" --name "$name" >/dev/null
      ;;
    bin)
      cargo new --bin "$path" --name "$name" >/dev/null
      ;;
    *)
      echo "unknown kind: $kind" >&2
      exit 3
      ;;
  esac
}

create_crate_if_missing lib crates/netmic-proto netmic-proto
create_crate_if_missing bin crates/netmic-server netmic-server
create_crate_if_missing bin crates/netmic-client netmic-client

# 若已存在非本骨架的 Cargo.toml，避免直接覆盖。
if [[ -f Cargo.toml ]] && ! grep -q "crates/netmic-proto" Cargo.toml; then
  echo "[bootstrap_workspace] 发现已有 Cargo.toml，且非 NetMic 骨架，已跳过覆盖。" >&2
  exit 4
fi

# 生成/修正 workspace 根 Cargo.toml（尽量保守覆盖）。
cat <<'TOML' > Cargo.toml
[workspace]
members = [
  "crates/netmic-proto",
  "crates/netmic-server",
  "crates/netmic-client",
]
resolver = "2"

[workspace.package]
edition = "2021"
license = "MIT"
authors = ["NetMic Agents"]

[workspace.dependencies]
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
TOML

# 写入最小 proto 结构，避免空工程没有方向。
cat <<'RS' > crates/netmic-proto/src/lib.rs
//! NetMic 共享协议与配置结构（MVP 版）。
//!
//! 文档来源：`MVP.md` 与 `docs/ROADMAP.md`。

pub mod config;
pub mod protocol;
RS

cat <<'RS' > crates/netmic-proto/src/config.rs
//! 参数安全范围与默认值（以文档为准）。
//!
//! 参考：`MVP.md` 第 3 节与第 4 节。

/// MVP 支持的编码类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    Opus,
    Pcm16,
}

/// 内部标准格式（文档建议值）。
pub const INTERNAL_SAMPLE_RATE_HZ: u32 = 48_000;
pub const INTERNAL_CHANNELS: u16 = 1;
pub const INTERNAL_BITS_PER_SAMPLE: u16 = 16;

/// 文档给出的默认参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultParams {
    pub codec: CodecKind,
    pub sample_rate_hz: u32,
    pub chunk_ms: u32,
    pub jitter_buffer_ms: u32,
}

impl Default for DefaultParams {
    fn default() -> Self {
        Self {
            codec: CodecKind::Opus,
            sample_rate_hz: INTERNAL_SAMPLE_RATE_HZ,
            chunk_ms: 20,
            jitter_buffer_ms: 100,
        }
    }
}
RS

cat <<'RS' > crates/netmic-proto/src/protocol.rs
//! 最小协议结构（握手 / 心跳 / 统计 / 音频帧头）。
//!
//! 说明：
//! - 文档事实来源：`MVP.md` 与 `docs/PROTO.md`。
//! - 这里先提供“可编译、可序列化”的骨架，后续再接入真实网络与音频链路。

use serde::{Deserialize, Serialize};

/// 会话参数（客户端请求值 / 服务端生效值）。
///
/// 参数安全范围与回退规则以后续 `config` 模块为准；此处仅承载数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionParams {
    /// 编码类型：`"opus"` 或 `"pcm16"`。
    pub codec: String,
    /// 目标采样率（Hz），MVP 安全范围：16_000–48_000。
    pub sample_rate_hz: u32,
    /// 声道数（MVP 固定为 1 / mono）。
    pub channels: u16,
    /// 帧时长（ms），MVP 安全范围：10–60。
    pub chunk_ms: u32,
    /// Opus 比特率（kbps），仅在 codec=opus 时生效。
    pub opus_bitrate_kbps: Option<u32>,
    /// 接收端目标缓冲（ms），MVP 建议范围：40–400。
    pub jitter_buffer_ms: u32,
}

impl SessionParams {
    /// 生成与 MVP 文档默认值一致的参数集。
    pub fn mvp_default() -> Self {
        Self {
            codec: "opus".to_string(),
            sample_rate_hz: 48_000,
            channels: 1,
            chunk_ms: 20,
            opus_bitrate_kbps: Some(48),
            jitter_buffer_ms: 100,
        }
    }
}

/// 握手请求（Client → Server）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeRequest {
    /// 会话标识（建议客户端生成 UUID，服务端原样回显）。
    pub session_id: String,
    /// 客户端展示名（日志与诊断用途）。
    pub client_name: String,
    /// 客户端请求的会话参数。
    pub requested: SessionParams,
    /// 预留字段：鉴权 token（MVP 默认不启用）。
    pub token: Option<String>,
}

/// 握手响应（Server → Client）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandshakeResponse {
    /// 会话标识（与请求一致）。
    pub session_id: String,
    /// 是否接受本次会话。
    pub accepted: bool,
    /// 拒绝或回退原因（可选）。
    pub reason: Option<String>,
    /// 服务端最终生效参数（含回退后的结果）。
    pub effective: SessionParams,
    /// 单客户端占用标记：busy=true 表示服务端已被占用。
    pub busy: bool,
}

/// 心跳报文（双向）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heartbeat {
    /// 会话标识（来自握手阶段）。
    pub session_id: String,
    /// 单调递增序号（用于检测丢包 / 乱序）。
    pub seq: u64,
    /// 发送端时间戳（毫秒）。
    pub sent_at_ms: u64,
}

/// 统计快照（Server → Client 为主，Client → Server 可选）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatsSnapshot {
    /// 统计窗口内的接收包计数。
    pub packets_received: u64,
    /// 统计窗口内的丢包估计。
    pub packets_lost: u64,
    /// 当前抖动缓冲深度（毫秒）。
    pub jitter_buffer_depth_ms: f32,
    /// 端到端估算延迟（毫秒）。
    pub estimated_e2e_latency_ms: f32,
}

/// 数据面音频帧头（不含 payload）。
///
/// 注意：实际 UDP 负载通常为“帧头 + 编码音频数据”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioFrameHeader {
    /// 会话标识。
    pub session_id: String,
    /// 音频帧序号（单调递增）。
    pub seq: u64,
    /// 帧起始时间戳（毫秒）。
    pub timestamp_ms: u64,
    /// 本帧音频采样数（解码后）。
    pub frame_samples: u32,
}
RS

# 写入 server/client 的最小入口，保证 cargo check 可运行。
cat <<'RS' > crates/netmic-server/src/main.rs
//! NetMic 服务端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 通过日志明确后续需要接入的模块位置。

use netmic_proto::protocol::SessionParams;
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-server skeleton started");
    println!("netmic-server skeleton ready");
}
RS

cat <<'RS' > crates/netmic-client/src/main.rs
//! NetMic 客户端入口（MVP 骨架）。
//!
//! 当前目标：
//! - 保证工作区结构完整、可编译（待 cargo 可用时验证）。
//! - 先使用共享默认参数，后续再接入采集/发送链路。

use netmic_proto::protocol::SessionParams;
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let params = SessionParams::mvp_default();
    info!(?params, "netmic-client skeleton started");
    println!("netmic-client skeleton ready");
}
RS

# 为 server/client 添加对 proto 与 tracing 的依赖（尽量幂等覆盖）。
cat <<'TOML' > crates/netmic-server/Cargo.toml
[package]
name = "netmic-server"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = { workspace = true }
netmic-proto = { path = "../netmic-proto" }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
TOML

cat <<'TOML' > crates/netmic-client/Cargo.toml
[package]
name = "netmic-client"
version = "0.1.0"
edition = "2021"

[dependencies]
netmic-proto = { path = "../netmic-proto" }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
TOML

cat <<'TOML' > crates/netmic-proto/Cargo.toml
[package]
name = "netmic-proto"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
TOML

echo "[bootstrap_workspace] Rust 工作区骨架已就绪。"
