# NetMic 协议草案（MVP 骨架）

本文档描述当前代码骨架中已落地的最小协议结构。
事实来源：`MVP.md` 第 4/5/8 节。

## 设计边界（MVP）
- 传输层：UDP
- 单向音频：Client → Server
- 单客户端占用：服务端最多 1 个 active sender（冲突返回 BUSY）
- 控制面与数据面：可复用同端口，但需有明确消息类型

## Datagram 分流（M1 占位 framing）
为满足“同端口区分控制面/数据面”的最小能力，当前采用极简 framing：
- UDP payload 首字节为 `kind`
- 剩余部分为对应 payload

kind 定义（代码位置：`crates/netmic-proto/src/datagram.rs`）：
- `0`：控制面 JSON（占位实现，建议包含 `type` 字段）
- `1`：数据面 PCM16（占位实现，后续将替换为“帧头 + 编码数据”）
- 其他值：未知类型（服务端仅记录日志）

当前实现约定（用于降低 client/server 漂移风险）：
- datagram 打包 helper 统一放在 `netmic-proto::datagram`：
  - `wrap_datagram(kind, payload)`
  - `wrap_control_json(payload)`
  - `wrap_audio_pcm16(payload)`
- `netmic-client` 提供最小联调入口：
  - `NETMIC_CLIENT_DEMO_SEND=1 cargo run -p netmic-client`
  - 该入口会发送 1 个控制面包（kind=0）与 1 个数据面占位包（kind=1）

注意：
- 这是 M1 的“可演进骨架”，不是最终 wire format。
- 若调整 kind 或 framing 规则，必须同步更新本文件与实现。

## 会话参数：`SessionParams`
字段与含义（代码位置：`crates/netmic-proto/src/protocol.rs`）：
- `codec: String`
- `sample_rate_hz: u32`
- `channels: u16`（MVP 固定 1）
- `chunk_ms: u32`
- `opus_bitrate_kbps: Option<u32>`
- `jitter_buffer_ms: u32`

默认值（`SessionParams::mvp_default()`）：
- codec: opus
- sample_rate_hz: 48000
- channels: 1
- chunk_ms: 20
- opus_bitrate_kbps: 48
- jitter_buffer_ms: 100

> 参数安全范围以 `MVP.md` 为准；本结构仅承载数据。

## 控制面消息（结构）

### 握手请求：`HandshakeRequest`
Client → Server
- `session_id: String`
- `client_name: String`
- `requested: SessionParams`
- `token: Option<String>`（预留）

### 握手响应：`HandshakeResponse`
Server → Client
- `session_id: String`
- `accepted: bool`
- `reason: Option<String>`
- `effective: SessionParams`
- `busy: bool`

### 心跳：`Heartbeat`
双向
- `session_id: String`
- `seq: u64`
- `sent_at_ms: u64`

### 统计快照：`StatsSnapshot`
Server → Client 为主
- `packets_received: u64`
- `packets_lost: u64`
- `buffer_depth_frames: u64`（接收端音频缓冲估算帧数，占位）
- `buffer_depth_ms: u64`（接收端音频缓冲估算毫秒，占位）
- `jitter_buffer_depth_ms: f32`
- `estimated_e2e_latency_ms: f32`
- `audio_rms: f32`（占位，单位/算法后续冻结）
- `audio_peak: u32`（占位，通常为 PCM16 |sample| 最大值）

## 数据面消息（结构）

### 音频帧头：`AudioFrameHeader`
- `session_id: String`
- `seq: u64`
- `timestamp_ms: u64`
- `frame_samples: u32`

当前状态：
- 已定义结构体，但尚未冻结 wire format（编码方式/帧头布局/校验策略）。
- 后续若新增 datagram 编码/解码规则，需同步更新本文件与 `MVP.md` 的相关约束说明。
