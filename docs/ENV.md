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

## Server（netmic-server）

- `NETMIC_SERVER_UDP_PORT`
  - 默认值：`43000`
  - 用途：服务端 UDP 监听端口；若解析失败会回退到默认值。
- `NETMIC_SERVER_AUDIO_DUMP`
  - 默认值：未设置（关闭）
  - 用途：设置为文件路径时，将收到的 PCM16 payload 追加写入文件，供排障使用。

## Linux 脚本（自检与虚拟麦）

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

## 自动化与监督器（agent_bootstrap.sh）

- `NETMIC_AUTO_INSTALL_RUST`
  - 默认值：`1`
  - 用途：是否允许监督器通过 rustup 自动安装 cargo；设置为 `0` 可关闭自动安装。
- `NETMIC_RUSTUP_PROFILE`
  - 默认值：`minimal`
  - 用途：rustup profile 选择，仅在自动安装 Rust 时生效。
