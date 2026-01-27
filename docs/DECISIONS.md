# DECISIONS

## 2026-01-27
- 背景：期望用户只运行 `scripts/agent_bootstrap.sh` 即可持续推进 MVP 开发，并在阻塞时得到明确提示。
- 决策：将 `agent_bootstrap.sh` 设计为“前台监督器入口”，自动拉起 Hub/autopilot/任务种子/骨架初始化；在 agent 循环中使用 `--context` 退化为上下文摘要器，避免递归启动。
- 影响：自动驾驶流程统一入口，但会在后台启动多个 agent 循环；停止需显式执行 `scripts/agent_bootstrap.sh --stop`（或 `scripts/autopilot.sh stop`）。

## 2026-01-27（scribe/受限环境策略）
- 背景：当前 Codex 沙箱无法监听 TCP 端口，`agent-hub` 绑定 `:7788` 会失败，Hub 不可达。
- 决策：scribe 在受限环境不强依赖 Hub；以 `scripts/agent_bootstrap.sh --context` 与 MVP/ROADMAP 为准更新 `SESSION_LOG`/`TODO`，并显式记录 Hub 阻塞。
- 影响：日志与优先级可保持可读，但 tasks/events 需在真实 Runner 上补齐与回写。
