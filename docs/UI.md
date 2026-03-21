# NetMic MVP UI 视图模型与 IPC（草案）

本文档描述当前 Tauri UI 的最小视图模型与 IPC 约定。
事实来源：`MVP.md` 第 3/7/8 节；实现位置：`apps/netmic-ui/`。

## 设计边界
- UI 仅覆盖 MVP 需求：配置 / 状态 / 日志。
- 参数安全范围仍以 `MVP.md` 为准；UI 只是输入与提示层。
- IPC 采用最小命令集：`get_status` / `set_client_config` / `set_server_config` / `start` / `stop` / `set_mode` / `force_disconnect` / `virtual_mic_create` / `virtual_mic_remove`。
- **独立进程模式**：服务端独立运行，UI 通过 UDP 控制面请求状态/发送命令（默认使用 `listen_port`）。为了减少用户心智负担，Server 模式点击“启动监听”会自动拉起服务端进程（若未运行）；可用 `NETMIC_SERVER_BIN` 指定服务端可执行文件路径。编译 UI 时会一并编译 `netmic-server` 与 `netmic-client`，并放在 `target/<profile>/` 供启动与联调。

## 视图模型（UiSnapshot）

### 顶层结构
- `mode: "client" | "server"`
- `status: "idle" | "connecting" | "listening" | "connected" | "streaming" | "error"`
- `status_note: String`
- `client_config: UiClientConfig`
- `server_config: UiServerConfig`
- `effective: SessionParams`（`netmic-proto`）
- `fallbacks: UiFallbackEvent[]`
- `metrics: UiMetrics`
- `runtime: UiRuntime`
- `devices: UiDevices`
- `logs: UiLogEntry[]`

### UiClientConfig（客户端配置）
- `server_addr: String`
- `server_port: u16`
- `input_device: String`
- `codec: String`（opus / pcm16）
- `sample_rate_hz: u32`
- `channels: u16`（MVP 固定为 1）
- `chunk_ms: u32`
- `opus_bitrate_kbps: u32`
- `jitter_buffer_ms: u32`
- `auto_reconnect: bool`
- `pairing_token: String`（预留）

### UiServerConfig（服务端配置）
- `listen_port: u16`
- `force_takeover: bool`（默认 false）
- `virtual_mic_enabled: bool`（默认 true；切换时触发虚拟麦克风创建/移除）

### UiMetrics（展示指标）
- `rtt_ms: f32`
- `packet_loss_pct: f32`
- `buffer_depth_ms: f32`
- `jitter_buffer_depth_ms: f32`
- `estimated_e2e_latency_ms: f32`
- `audio_rms: f32`
- `audio_peak: u32`
- `uplink_kbps: f32`

### UiWaveform（时域波形事件载荷）
- `ts_ms: u64`（毫秒时间戳）
- `points: f32[]`（长度 128，范围 [-1, 1]，20 FPS）

### UiRuntime（运行态）
- `peer_addr: Option<String>`
- `connected_seconds: u64`
- `reconnect_attempts: u32`
- `mic_permission: String`（macOS 权限状态：已授权 / 未授权 / 不可用 / 未知 / 不适用）
- `virtual_mic_name: String`
- `server_status_updated_ms: u64`（服务端可观察状态最后更新时间戳，毫秒；包括 Server 模式轮询结果，以及 Client 模式下握手/统计等来自服务端的真实响应）
- `last_error: Option<String>`（最近一次错误原因，用于状态页明确提示）

### UiDevices
- `input: String[]`（输入设备列表，运行时从系统枚举刷新）

### UiFallbackEvent
- `field: String`
- `requested: String`
- `applied: String`
- `reason: String`

### UiLogEntry
- `ts_ms: u64`
- `level: "info" | "warn" | "error"`
- `message: String`

## IPC 命令
- `get_status() -> UiSnapshot`
- `set_mode(mode: String) -> UiSnapshot`
- `set_client_config(config: UiClientConfig) -> UiSnapshot`
- `set_server_config(config: UiServerConfig) -> UiSnapshot`
- `reset_defaults() -> UiSnapshot`
- `start() -> UiSnapshot`
- `stop() -> UiSnapshot`
- `force_disconnect() -> UiSnapshot`
- `virtual_mic_create() -> UiSnapshot`
- `virtual_mic_remove() -> UiSnapshot`
- `clear_logs() -> UiSnapshot`
- `export_logs() -> { ok: bool }`

## 配置持久化
- UI 会持久化最近一次的模式与配置（client/server 分离）。
- 启动时优先读取本地配置，再回退到默认值。
- 默认路径：`BaseDirectory::AppConfig/netmic-ui.json`

## 服务端状态联动（独立进程）
UI 在 Server 模式下通过 UDP 控制面轮询：
- 请求：`ServerStatusRequest`
- 响应：`ServerStatusResponse`
默认目标地址：`server_addr:listen_port`（建议为 `127.0.0.1` 本地端口）。
状态页需明确展示：
- 本地控制端口（= `listen_port`）
- 虚拟麦克风就绪状态（含错误提示）

## 虚拟麦克风联动（独立进程）
- Server 模式点击“启动监听”时，若 `virtual_mic_enabled=true`，会发送 `virtual_mic_create` 请求以确保虚拟麦克风就绪。
- 配置页切换“启用虚拟麦克风”会触发 `virtual_mic_create` / `virtual_mic_remove`。
- 自动拉起服务端时，默认会带上 `NETMIC_SERVER_UDP_PORT` 与 `NETMIC_SERVER_VIRTUAL_MIC_AUTO_CREATE=1`。

## 服务端进程管理（UI 行为）
- Server 模式点击“启动监听”时，若服务端未运行，UI 会自动拉起服务端进程。
- Server 模式点击“停止监听”时，若服务端由 UI 启动且 `NETMIC_UI_SERVER_AUTO_STOP=true`，UI 会自动结束该进程。
- Server 模式点击“启动监听”会先清理其他 `netmic-server` 进程，确保单实例运行。

## 事件
- `netmic://snapshot`：UI 订阅后接收 `UiSnapshot` 推送
- `netmic://waveform`：客户端采集到的时域波形（20 FPS / 128 点）

## UI 真相模型
- 所有可见状态应来源于 `UiSnapshot` 或 `UiWaveform`
- 同一字段在配置页、状态页、日志页的呈现必须一致
- 若后端状态已更新但 UI 未及时反映，应视为 UI 缺陷
- 若状态已过期但 UI 未给出提示，应视为 UI 缺陷
- Harness 需要区分“后端写出 snapshot”和“前端完成渲染”；M3 应以前端 render ack 作为真实刷新依据

## 刷新要求
- `netmic://snapshot` 到达后，页面应在一次正常渲染周期内更新
- Server 状态轮询周期当前为 1000 ms
- 波形目标刷新频率当前为 20 FPS
- UI 刷新时效与真实 App 验收要求统一见 `docs/UI_TESTING.md`

## UI 对齐 MVP 的关键点
- 参数范围：采样率 / 帧长 / bitrate / buffer 必须限制在 `MVP.md` 范围内
- 回退提示：展示 `fallbacks` 列表用于提示“已回退到 X”
- 状态页：连接状态 + 指标 + 当前生效参数
- 日志页：按级别过滤 + 导出入口（MVP）

## UI 测试
- 渲染层、交互层、IPC 集成层和真实 App 验收统一见 `docs/UI_TESTING.md`

## macOS 权限说明
- 需要在 `apps/netmic-ui/src-tauri/Info.plist` 提供 `NSMicrophoneUsageDescription`
