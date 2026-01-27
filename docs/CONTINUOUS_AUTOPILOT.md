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
touch docs/SESSION_LOG.md docs/DECISIONS.md docs/TODO.md
scripts/agent_bootstrap.sh   # 确认 git/日志/issue 概况
```

## 快速启动（建议在 Linux 节点执行）
1) 启 Hub：`python3 agent-hub/agent_hub.py`（默认 0.0.0.0:7788）。
2) 设定内网 Hub 地址（可选）：`export HUB_URL=http://<linux-ip>:7788`  
   - 若不设置，`scripts/autopilot.sh` 会自动探测本机默认 IP，并写入 `.autopilot/hub_url.txt`。
   - 若存在 `docs/HUB_URL.txt`，autopilot 会优先读取它（适合跨机器共享同一个 Hub 地址）。
   - 手动启动角色前，可用：`export HUB_URL=${HUB_URL:-$(cat .autopilot/hub_url.txt 2>/dev/null || echo http://127.0.0.1:7788)}`
3) tmux 一键布局：`scripts/launch_agents.sh`  
   - 会创建 tmux session `netmic-agents`：hub / orchestrator / builder-linux / builder-mac / scribe 窗口。
4) 在各窗口（或各机器）启动 Codex：
   - Linux: `HUB=$HUB_URL ROLE=orchestrator scripts/start_agent.sh`
   - Linux: `HUB=$HUB_URL ROLE=builder-linux scripts/start_agent.sh`
   - Mac   : `HUB=$HUB_URL ROLE=builder-mac scripts/start_agent.sh`
   - 任意 : `HUB=$HUB_URL ROLE=scribe scripts/start_agent.sh`
   （若 `codex` 已登录同一账号，可直接运行；否则先登录。）

## 一键自动驾驶（推荐）
当你不想手动起多个窗口时，直接用单脚本：

```bash
cd /path/to/NetMic
scripts/autopilot.sh start
```

常用命令：
- 查看整体状态（hub/agents/tasks/pids）：`scripts/autopilot.sh status`
- 停止本机 autopilot 进程：`scripts/autopilot.sh stop`

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
- 若某角色崩溃，直接在对应 tmux 窗口重跑 `HUB=... ROLE=... scripts/start_agent.sh`。

## 故障与重启
- 若某角色崩溃，直接在对应 tmux 窗口重新执行 `HUB=... ROLE=... scripts/start_agent.sh`。
- Agent Hub 无鉴权，局域网内部署即可；如需安全性，可在外层做防火墙/ACL。

## 自定义
- 修改 `scripts/launch_agents.sh` 的 `SESSION`、`HUB_URL` 环境变量适配你的网络。
- 不想用 tmux 时，可按上述启动命令手动在各机执行。
- 不想启 Hub 的简化模式：手动在 mac/linux 各开一个 Codex（builder 角色），用 GitHub Actions/SSH 直接触发客户端/服务端，无事件协调；适用于小批量实验。
