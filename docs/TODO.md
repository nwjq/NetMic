# TODO（MVP 自动驾驶看板）

当前优先级（与 Hub 任务队列配合）：
- 用户前置：配置 `.autopilot/runner.env`（至少补全 `BUILDER_MAC_SSH` 或在 mac 端也运行监督器）。
- 编排优先：Hub 已可达（`http://192.168.11.1:7788`），orchestrator 应开始 claim/编排并推动任务状态从 queued 前进。
- 里程碑优先：推进 M0 任务链（pactl/pipewire 自检 → virtual source 创建 → sine wave 持续写入 5 分钟 smoke），并及时回写 SESSION_LOG。
- 自动驾驶：运行 `scripts/agent_bootstrap.sh`，观察置顶提示并清除阻塞项。
- 记录(03:05+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），`M1-003-client-capture-sender` 仍 queued。
- 记录(02:59+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox，仍无 sudo 权限），`M1-003-client-capture-sender` 仍 queued。
- 记录(02:55+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），`M1-003-client-capture-sender` 仍 queued。
- 记录(02:48+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），`M1-001-protocol-doc`/`M1-003-client-capture-sender` 仍 queued。
- 记录(02:44+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:39+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），`M1-001-protocol-doc`/`M1-003-client-capture-sender` 仍 queued。
- 记录(02:33+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:28+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:23+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:16+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:11+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:07+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(02:02+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），其余任务无新变化。
- 记录(01:56+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 仍 blocked（真实 Runner 缺少 sox），`M3-001-metrics-and-reconnect` 已 done。
- 记录(01:51+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-3` 继续 blocked（真实 Runner 缺少 sox），`M3-001-metrics-and-reconnect` 已由 builder-linux 认领推进。
- 记录(01:44+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-2` 已 done，`M0-virtual-mic-20260128-3` 为 blocked（真实 Runner 缺少 sox），需补依赖后重测 5 分钟 smoke。
- 记录(01:39+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-2` 已 done，`M0-virtual-mic-20260128-3` 仍 queued，需继续推进真实 Runner 5 分钟 smoke 验证与回写。
- 记录(01:22+08)：`scripts/verify_mvp.py` 仍为 M0–M3 gates 全 pass（active=M3）；Hub 显示 `M0-virtual-mic-20260128-1` 已 done，当前应优先推进 `M0-virtual-mic-20260128-2/-3` 的真实 Runner 验证与回写。
- 记录(01:28+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M2-001-config-validation` 已被 `builder-linux` 认领，M0 主线仍应优先推进 `M0-virtual-mic-20260128-2/-3` 的真实 Runner 验证与回写。
- 记录(01:29+08)：Hub 任务已收敛：`M0-001-linux-audio-selfcheck` 与 `M0-virtual-mic-20260127-{1,2,3}` 均为 superseded；`M0-virtual-mic-20260128-2/-3` 仍 queued 且 priority=high，应继续作为主线推进。
- 记录(01:35+08)：门禁仍为 M0–M3 全 pass（active=M3）；Hub 显示 `M2-001-config-validation` 已 done，`M0-virtual-mic-20260128-2/-3` 仍 queued，需继续推进真实 Runner 验证与回写。
