# DECISIONS

## 2026-01-27
- 背景：期望用户只运行 `scripts/agent_bootstrap.sh` 即可持续推进 MVP 开发，并在阻塞时得到明确提示。
- 决策：将 `agent_bootstrap.sh` 设计为“前台监督器入口”，自动拉起 Hub/autopilot/任务种子/骨架初始化；在 agent 循环中使用 `--context` 退化为上下文摘要器，避免递归启动。
- 影响：自动驾驶流程统一入口，但会在后台启动多个 agent 循环；停止需显式执行 `scripts/agent_bootstrap.sh --stop`（或 `scripts/autopilot.sh stop`）。

- 背景：`netmic-proto` 已引用 `protocol` 模块，但仓库中缺少协议事实来源与最小结构定义，容易造成 client/server 漂移。
- 决策：新增 `docs/PROTO.md` 作为 MVP 协议事实来源，并在 `netmic-proto` 中补齐 `SessionParams / Handshake / Heartbeat / StatsSnapshot / AudioFrameHeader` 最小结构；握手阶段显式携带 `session_id`，服务端原样回显。
- 影响：后续协议字段调整必须先更新 `docs/PROTO.md`；builder 可直接围绕这些结构体落地最小 UDP 骨架。

## 2026-01-27（scribe/受限环境策略）
- 背景：当前 Codex 沙箱无法监听 TCP 端口，`agent-hub` 绑定 `:7788` 会失败，Hub 不可达。
- 决策：scribe 在受限环境不强依赖 Hub；以 `scripts/agent_bootstrap.sh --context` 与 MVP/ROADMAP 为准更新 `SESSION_LOG`/`TODO`，并显式记录 Hub 阻塞。
- 影响：日志与优先级可保持可读，但 tasks/events 需在真实 Runner 上补齐与回写。

## 2026-01-28（builder-linux / M1 UDP 接收骨架）
- 背景：M1 需要在服务端同一 UDP 端口上区分控制面与数据面，但协议尚未冻结。
- 决策：在 `netmic-proto` 中引入“首字节 kind + payload”的占位 framing，并约定 kind=0 为控制面 JSON、kind=1 为 PCM16 数据面；服务端锁定首个发送方为 active client，其余来源仅记录 busy 日志。
- 影响：后续冻结 wire format 时，必须同时更新 `docs/PROTO.md` 与 `crates/netmic-proto/src/datagram.rs` / `crates/netmic-server/src/main.rs` 的分流逻辑。
