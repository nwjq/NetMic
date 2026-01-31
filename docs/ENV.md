# NetMic 运行时环境变量清单（NETMIC_*）

说明：
- 本文只覆盖 `NETMIC_*` 前缀变量；通用变量（如 `RUST_LOG`）不在此列。
- 未设置时均按代码/脚本中的默认值处理。

## Client（netmic-client）

- `NETMIC_SERVER_ADDR`
  - 默认值：`127.0.0.1:43000`
  - 用途：客户端发送 UDP 控制面/数据面时的目标地址（`host:port`）。
- `NETMIC_CLIENT_DEMO_SEND`
  - 默认值：未设置（关闭）
  - 用途：设置为 `1/true/on/yes` 时启用演示发送，发出 kind-framed UDP 包，便于手工联调。
- `NETMIC_CLIENT_STREAM_SECS`
  - 默认值：未设置（不限制时长）
  - 用途：限制客户端持续发送的时长（秒）；设置为 `0` 表示不限制。
- `NETMIC_CLIENT_HEARTBEAT_MS`
  - 默认值：`1000`
  - 用途：客户端心跳间隔（毫秒）；设置为 `0` 可禁用心跳。

## Server（netmic-server）

- `NETMIC_SERVER_BIND_ADDR`
  - 默认值：`0.0.0.0`
  - 用途：服务端 UDP 绑定地址（host 或 host:port）；如包含端口则优先使用该端口并忽略 `NETMIC_SERVER_UDP_PORT`。
- `NETMIC_SERVER_UDP_PORT`
  - 默认值：`43000`
  - 用途：服务端 UDP 监听端口；若解析失败会回退到默认值。独立进程模式下 UI 默认使用该端口查询状态。
- `NETMIC_SERVER_BIN`
  - 默认值：未设置
  - 用途：指定 UI 自动启动服务端时的可执行文件路径；未设置则尝试使用与 UI 同目录的 `netmic-server`，再回退到 `$PATH`。
- `NETMIC_UI_SERVER_AUTO_STOP`
  - 默认值：`true`（开启）
  - 用途：UI 在 Server 模式点击“停止监听”时，若服务端由 UI 启动则自动结束该进程；设置为 `0/false/off/no` 可保留服务端继续运行。
- `NETMIC_SERVER_AUDIO_DUMP`
  - 默认值：未设置（关闭）
  - 用途：设置为文件路径时，将收到的 PCM16 payload 追加写入文件，供排障使用。
- `NETMIC_SERVER_AUDIO_SINK`
  - 默认值：`pulse`
  - 用途：选择服务端音频输出 sink（`pulse`/`null`）。`pulse` 会把 PCM 写入虚拟 sink；`null` 用于仅跑网络链路。
- `NETMIC_SERVER_VIRTUAL_MIC_AUTO_CREATE`
  - 默认值：`true`（开启）
  - 用途：服务端启动时自动创建虚拟麦克风（module-null-sink + module-remap-source）。设置为 `0/false/off/no` 可关闭。
- `NETMIC_SERVER_TEST_TONE`
  - 默认值：未设置（关闭）
  - 用途：设置为 `1/true/on/yes` 时，服务端不走 UDP 接收，直接向虚拟麦克风持续写入测试音（正弦波）。
- `NETMIC_SERVER_TEST_TONE_SECS`
  - 默认值：`300`
  - 用途：测试音持续时长（秒）。设置为 `0` 表示不限制。
- `NETMIC_SERVER_TEST_TONE_HZ`
  - 默认值：`440`
  - 用途：测试音频率（Hz）。
- `NETMIC_SERVER_TEST_TONE_GAIN`
  - 默认值：`0.2`
  - 用途：测试音幅度（0.0–1.0，内部会 clamp 到 0.9 防止爆音）。

## Linux 脚本（自检与虚拟麦）

说明：服务端状态查询虚拟麦克风时同样复用 `NETMIC_VIRTUAL_MIC_*` 环境变量。

- `NETMIC_SMOKE_ID`
  - 默认值：`$$`（当前 shell PID）
  - 用途：`audio_selfcheck.sh`/`virtual_mic_smoke.sh` 使用的临时资源后缀，便于并行执行。
- `NETMIC_SMOKE_DURATION_SEC`
  - 默认值：`30`
  - 用途：`virtual_mic_smoke.sh` 测试音时长（秒）。
- `NETMIC_SMOKE_PREFIX`
  - 默认值：`netmic_smoke`
  - 用途：`virtual_mic_smoke.sh` 生成的虚拟设备名前缀。
- `NETMIC_VIRTUAL_MIC_STATE`
  - 默认值：`/tmp/netmic_virtual_mic.env`
  - 用途：`virtual_mic.sh` 记录模块 id 与设备名的状态文件路径。
- `NETMIC_VIRTUAL_MIC_PREFIX`
  - 默认值：`netmic`
  - 用途：虚拟设备名前缀（影响 sink/source 名称）。
- `NETMIC_VIRTUAL_MIC_SINK_NAME`
  - 默认值：`<prefix>_sink`
  - 用途：虚拟 sink 名称。
- `NETMIC_VIRTUAL_MIC_SOURCE_NAME`
  - 默认值：`<prefix>_source`
  - 用途：虚拟 source 名称。
- `NETMIC_VIRTUAL_MIC_SINK_DESC`
  - 默认值：`NetMic_Virtual_Sink`
  - 用途：虚拟 sink 描述。
- `NETMIC_VIRTUAL_MIC_SOURCE_DESC`
  - 默认值：`NetMic_Virtual_Mic`
  - 用途：虚拟 source 描述。
