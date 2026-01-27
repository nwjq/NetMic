# TODO（MVP 自动驾驶看板）

当前优先级（与 Hub 任务队列配合）：
- 用户前置：配置 `.autopilot/runner.env`（至少补全 `BUILDER_MAC_SSH` 或在 mac 端也运行监督器）。
- 编排优先：Hub 已可达（`http://192.168.11.1:7788`），orchestrator 应开始 claim/编排并推动任务状态从 queued 前进。
- 里程碑优先：推进 M0 任务链（pactl/pipewire 自检 → virtual source 创建 → sine wave 持续写入 5 分钟 smoke），并及时回写 SESSION_LOG。
- 自动驾驶：运行 `scripts/agent_bootstrap.sh`，观察置顶提示并清除阻塞项。
- 记录(01:16+08)：`scripts/verify_mvp.py` 仍为 M0–M3 gates 全 pass（active=M3）；Hub 任务状态未变更，`M0-virtual-mic-20260128-{1,2,3}` 仍 queued；当前应优先推进并认领 `M0-virtual-mic-20260128-1`。
