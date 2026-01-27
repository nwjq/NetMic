# TODO（MVP 自动驾驶看板）

当前优先级（与 Hub 任务队列配合）：
- 用户前置：安装 rustup/cargo，确保 `cargo check` 可运行。
- 用户前置：配置 `.autopilot/runner.env`（至少补全 `BUILDER_MAC_SSH` 或在 mac 端也运行监督器）。
- 环境前置：在可监听端口的真实机/Runner 上启动 Hub（受限沙箱可能无法绑定 `:7788`）。
- 自动驾驶：运行 `scripts/agent_bootstrap.sh`，观察置顶提示并清除阻塞项。
