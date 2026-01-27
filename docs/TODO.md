# TODO（MVP 自动驾驶看板）

当前优先级（与 Hub 任务队列配合）：
- 用户前置：安装 rustup/cargo，确保 `cargo check` 可运行。
- 用户前置：配置 `.autopilot/runner.env`（至少补全 `BUILDER_MAC_SSH` 或在 mac 端也运行监督器）。
- 环境前置：在可监听端口的真实机/Runner 上启动 Hub（受限沙箱可能无法绑定 `:7788`）。
- 里程碑优先：当 cargo/Hub 可用后，先推进 M0（pactl/pipewire 自检 → virtual source 创建 → sine wave 持续写入 5 分钟 smoke）。
- 自动驾驶：运行 `scripts/agent_bootstrap.sh`，观察置顶提示并清除阻塞项。
- 记录(20:19+08)：本机 `http://127.0.0.1:7788` 仍不可达；Hub 恢复后需补跑 `register` + `/v1/tasks` + `/v1/events` 并回写日志。
