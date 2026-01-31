# NetMic 协议草案（MVP 骨架）

本文档描述当前代码骨架中已落地的最小协议结构。
事实来源：`MVP.md` 第 4/5/8 节。

## 设计边界（MVP）
- 传输层：UDP
- 单向音频：Client → Server
- 单客户端占用：服务端最多 1 个 active sender（冲突返回 BUSY）
- 控制面与数据面：可复用同端口，但需有明确消息类型
- UI/管理端控制：复用控制面消息，建议仅接受 loopback

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

### JSON envelope（MVP 约定）
控制面统一使用 JSON 包裹：
- `type`: 消息类型（字符串）
- `payload`: 对应结构体

示例（握手请求）：
```json
{
  "type": "handshake_request",
  "payload": { ...HandshakeRequest }
}
```

当前实现对齐位置：`netmic-proto/src/control.rs`（`encode_control_message` / `decode_control_message`）。

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

### 服务端状态请求：`ServerStatusRequest`
UI/管理端 → Server（仅建议 loopback）
- `request_id: String`

### 服务端状态响应：`ServerStatusResponse`
Server → UI/管理端
- `request_id: String`
- `state: String`（idle/listening/streaming/reconnecting）
- `active_client: Option<String>`（host:port）
- `active_client_seconds: u64`
- `uptime_ms: u64`
- `last_error: Option<String>`
- `virtual_mic_name: String`
- `virtual_mic_ready: bool`
- `virtual_mic_error: Option<String>`
- `stats: StatsSnapshot`

### 服务端命令：`ServerCommandRequest`
UI/管理端 → Server（仅建议 loopback）
- `request_id: String`
- `action: String`（当前支持：`force_disconnect` / `virtual_mic_create` / `virtual_mic_remove`）

### 服务端命令响应：`ServerCommandResponse`
Server → UI/管理端
- `request_id: String`
- `ok: bool`
- `message: Option<String>`

## 数据面消息（结构）

### 音频帧头：`AudioFrameHeader`
- `session_id: String`
- `seq: u64`
- `timestamp_ms: u64`
- `frame_samples: u32`

### PCM16 音频 datagram（MVP 现行）
kind=1 的 payload 结构（小端）：
```
[u32 header_len][header_json][pcm16_bytes]
```
- `header_len`: `AudioFrameHeader` 的 JSON 字节长度（u32 little-endian）
- `header_json`: `AudioFrameHeader` 的 JSON 编码
- `pcm16_bytes`: PCM16 原始字节流（小端）

当前实现位置：`netmic-proto/src/datagram.rs`（`wrap_audio_pcm16_with_header` / `split_audio_pcm16_with_header`）。
