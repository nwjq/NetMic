# NetMic 多 Agent 自动迭代指南

目标：在 mac 与 linux 自托管 Runner 间，借助 Codex 多角色协作 + Agent Hub + tmux，实现持续开发与测试。

## 前置准备
- 已安装并登录 codex CLI；能 push 到仓库的 Git 凭据。
- Python 3（运行 Agent Hub）、tmux（用于布局），两台 Runner 互通同一内网。
- mac Runner 有音频采集权限；linux Runner 有 PipeWire/Pulse 与 pactl 可用。
- 可选：在仓库 Secrets 配置 `CODEX_TOKEN` 以便在 Runner 侧无提示启动。

## 组件与角色
- Agent Hub：轻量事件总线，跑在任一 Linux 节点。命令：`python3 agent-hub/agent_hub.py`
- 角色（均为 Codex 会话）：`orchestrator`（分配/监督），`builder-linux`，`builder-mac`，`scribe`（日志/PR 更新）。

### 角色职责速览
- orchestrator：读 ROADMAP/SESSION_LOG，分配 Issue/PR，触发跨机 E2E。
- builder-linux：服务端/虚拟麦相关编码与测试，跑 Linux 端 CI。
- builder-mac：客户端采集/GUI 编译与测试，跑 mac 端 CI。
- scribe：更新 SESSION_LOG / DECISIONS / PR 描述与测试结果。

## 首次初始化（一次性）
```bash
cd /path/to/NetMic
scripts/agent_bootstrap.sh --once
```
说明：
- `agent_bootstrap.sh` 会在缺失时自动创建 `docs/SESSION_LOG.md`、`docs/DECISIONS.md`、`docs/TODO.md`。
- `--once` 适合初始化/自检；日常推荐直接无参运行（前台监督模式）。

## 推荐入口（单命令自动驾驶）
```bash
cd /path/to/NetMic
scripts/agent_bootstrap.sh
```
该入口会自动完成以下动作（能做就做，做不到会置顶提示）：
- 解析/回退 Hub 地址（优先 `docs/HUB_URL.txt`，其次本机探测）。
- 在 Hub 不健康且为本机地址时自动拉起 `agent-hub/agent_hub.py`。
- 注入任务种子（`scripts/seed_tasks.py`），保证有活可干。
- 在 cargo 可用时自动创建 Rust 工作区骨架（`scripts/bootstrap/bootstrap_workspace.sh`）。
- 周期性执行 MVP Gate 验证（`scripts/verify_mvp.py`），以 `docs/MVP_GATES.yaml` 作为机器可读验收标准。
- 启动 `scripts/autopilot.sh start`（后台跑 agent 循环），并在前台持续刷新状态。
- Hub 地址会写入 `.autopilot/runtime_hub_url.txt`（避免污染 Git）；`.autopilot/hub_url.txt` 仅作为只读兼容输入。

重要行为说明：
- 监督器不会自动退出或 stop；退出用 `Ctrl+C`，停止后台进程用：
  - `scripts/agent_bootstrap.sh --stop`
- 如果出现权限/安全/依赖阻塞，监督器会在控制台顶部提示“需要用户处理的事项”，并持续轮询状态。
- 若存在关键阻塞项（例如缺少 cargo、codex 未登录），监督器会暂停启动 autopilot，避免产生不可控改动；解决后重跑即可。
- 若同一失败签名在 Gate 验证中重复出现，监督器会进入“研究模式”（写入 `.autopilot/research_mode.txt`），提示 agent 先调研再修复。

## Gate 验证（单命令真相）

核心文件：
- Gate 定义：`docs/MVP_GATES.yaml`
- 验证入口：`scripts/verify_mvp.py`
- 监督器状态：`.autopilot/verify_status.json` / `.autopilot/verify_status.txt`

手动运行：
```bash
cd /path/to/NetMic
scripts/verify_mvp.py
```

设计要点：
- Gate 以里程碑（M0/M1/M2/M3）组织，监督器只推进到“第一个未通过 Gate”。
- 验证结果统一为 `pass/fail/blocked`，其中 `blocked` 表示环境/权限/依赖阻塞，应优先修环境而不是盲改代码。
- 修改协议/参数/验收标准时，应在同一次变更中同步更新 `docs/MVP_GATES.yaml` 与相关文档。

## 研究模式（失败签名 → 调研 → 修复 → 再验证）

触发逻辑（由监督器自动执行）：
- 当同一失败签名连续重复达到阈值（默认 3 次），且不在冷却期内，进入研究模式。

研究模式产物：
- `.autopilot/research_mode.json`
- `.autopilot/research_mode.txt`

建议 agent 行为：
- 优先查官方文档/主仓库/primary sources，先解释根因假设，再提交最小可验证修复。
- 修复后必须重新运行 `scripts/verify_mvp.py`，以 Gate 结果作为是否继续推进的依据。

## 安全自更新与热重启（监督器护栏）

当 agent 修改了监督器/自动驾驶脚本（例如 `scripts/agent_bootstrap.sh` 或 `scripts/autopilot.sh`）时，建议写入重启请求：

```bash
printf '%s\n' \"reason: updated bootstrap/autopilot\" > .autopilot/restart.requested
```

行为说明：
- 监督器会检测 `.autopilot/restart.requested`，并在冷却时间与每小时次数护栏内执行 `exec scripts/agent_bootstrap.sh` 热重启。
- 相关运行态文件位于 `.autopilot/`，已在 `.gitignore` 中忽略，不应进入版本控制。

## 跨机配置（可选，但想全自动建议配置）
为避免每次手动 export，监督器会自动读取：
- `.autopilot/runner.env`（本机私有配置）

建议做法：
```bash
cp .autopilot/runner.env.example .autopilot/runner.env
```
然后按需填写：
- `HUB_URL=http://<linux-ip>:7788`
- `BUILDER_MAC_SSH=user@mac-runner-host`
- `BUILDER_MAC_ROOT=/path/to/NetMic`
- `NETMIC_AUTO_INSTALL_RUST=0`（可选：关闭监督器通过 rustup 自动安装 cargo）
- `NETMIC_RUSTUP_PROFILE=minimal`（可选：rustup profile，默认 minimal）

关于自动安装 Rust：
- 默认开启；如需关闭可在 `.autopilot/runner.env` 设为 `NETMIC_AUTO_INSTALL_RUST=0`。
- 开启需确认 Runner 允许联网且允许写入 `$HOME/.cargo`。
- 启用后，监督器会在检测到缺少 `cargo` 时自动执行 rustup 官方安装脚本，并尝试 `source $HOME/.cargo/env`。

## 手动模式（需要更细控制时）
若你不想使用监督器入口，也可以按传统方式手动启动：
1) 启 Hub：`python3 agent-hub/agent_hub.py`
2) 启 autopilot：`scripts/autopilot.sh start|status|stop`
3) 或使用 tmux 布局：`scripts/launch_agents.sh`
4) 在各窗口（或各机器）启动 Codex：
   - Linux: `HUB=$HUB_URL ROLE=orchestrator scripts/start_agent.sh`
   - Linux: `HUB=$HUB_URL ROLE=builder-linux scripts/start_agent.sh`
   - Mac   : `HUB=$HUB_URL ROLE=builder-mac scripts/start_agent.sh`
   - 任意 : `HUB=$HUB_URL ROLE=scribe scripts/start_agent.sh`
（监督器与手动模式可以混用，但请避免重复拉起过多 agent 循环。）

## 一键自动驾驶（推荐）
当你不想手动起多个窗口时，直接用单脚本：

```bash
cd /path/to/NetMic
scripts/autopilot.sh start
```

常用命令：
- 查看整体状态（hub/agents/tasks/pids）：`scripts/autopilot.sh status`
- 停止本机 autopilot 进程：`scripts/autopilot.sh stop`
- 危险模式：`AUTOPILOT_DANGEROUS=1 scripts/autopilot.sh start`
  说明：codex CLI 的 `--full-auto` 与 `--dangerously-bypass-approvals-and-sandbox` 互斥；开启危险模式时会自动关闭 `--full-auto`。

可选远程启动 mac builder（需 ssh 免密或可用凭据）：

```bash
export BUILDER_MAC_SSH=user@mac-runner-host
export BUILDER_MAC_ROOT=/path/to/NetMic   # 若远端路径不同
scripts/autopilot.sh start
```

## 会话接续与低上下文技巧
- 每次工作前运行 `scripts/agent_bootstrap.sh`：输出分支、git 状态、SESSION_LOG/DECISIONS/TODO 摘要、Issue 列表。
- 退出前在 `docs/SESSION_LOG.md` 写当日摘要：DONE/BLOCKER/NEXT/TEST。
- 重大取舍记录在 `docs/DECISIONS.md`。

## CI/Runner 协同
- mac Runner 承担客户端构建与音频采集相关测试；linux Runner 承担服务端、虚拟麦与端到端接收。
- 跨机 E2E 可用 `scripts/e2e_smoke.py`（后续改造为真实音频链路），由 orchestrator 分配 session_id，两个 builder 按角色上报 node_ready。

## 健康检查与排障
- Hub 健康：`curl "$HUB_URL/v1/health"` 应返回 ok/agents/tasks。
- Hub agents：`curl "$HUB_URL/v1/agents?active_within=900&limit=50"`
- Hub tasks：`curl "$HUB_URL/v1/tasks?limit=50"`
- 事件流：`curl "$HUB_URL/v1/events?since=0&limit=5"` 查看最近事件。
- agent 前置信息：`scripts/agent_bootstrap.sh`。
- 若当前环境无法访问/监听 `:7788`，监督器会在顶部提示阻塞项；`--context` 模式会跳过 Hub 详情，但仍输出本地文档摘要。
- 若某角色崩溃，直接在对应 tmux 窗口重跑 `HUB=... ROLE=... scripts/start_agent.sh`。

## 故障与重启
- 若某角色崩溃，直接在对应 tmux 窗口重新执行 `HUB=... ROLE=... scripts/start_agent.sh`。
- Agent Hub 无鉴权，局域网内部署即可；如需安全性，可在外层做防火墙/ACL。

## 自定义
- 修改 `scripts/launch_agents.sh` 的 `SESSION`、`HUB_URL` 环境变量适配你的网络。
- 不想用 tmux 时，可按上述启动命令手动在各机执行。
- 不想启 Hub 的简化模式：手动在 mac/linux 各开一个 Codex（builder 角色），用 GitHub Actions/SSH 直接触发客户端/服务端，无事件协调；适用于小批量实验。
