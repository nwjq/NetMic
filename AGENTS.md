# NetMic Agent 指南

本仓库当前以“文档 + 自动化脚本”为主。请把文档视为事实来源（source of truth），所有变更尽量与文档保持一致。

## 语言约定（重要）
- 面向开发者的文档、说明与注释，尽量使用中文。
- 代码标识符（变量名、函数名、类型名、文件名）保持英文为主。
- 如必须引用英文资料或术语，建议同时给出中文解释或总结。

## 优先阅读（按顺序）
- `MVP.md`：产品范围、核心约束与参数安全范围。
- `docs/ROADMAP.md`：里程碑顺序与验收规则。
- `docs/WORKFLOW.md`：协作模型与合并边界。
- `docs/MODULE_OWNERS.md`：模块责任边界与冲突处理规则。
- `docs/CONTINUOUS_AUTOPILOT.md`：多 Agent 与 Runner 的工作流。

## 本地 Skills（按需触发）
- `netmic-architecture`：系统边界、协议/数据流、跨模块重构；接口变更要同步更新文档。
- `netmic-ui-tauri`：UI/UX、IPC 契约、状态机、配置/指标 UI；在本仓库内优先于通用 Tauri 指南。
- `tauri`：官方框架细节（配置项、插件、分发/签名、安全硬化）。
- `rust-engineer`：Rust 实现模式与错误处理。

## 仓库结构速览
- `agent-hub/`：局域网协作协调服务（极简 HTTP JSON）。
- `scripts/`：Hub、角色启动与 autopilot 自动化脚本。
- `docs/`：协作方式、里程碑、角色与流程规范。

## 常用命令
- 启动 Hub：`python3 agent-hub/agent_hub.py`
- 加载上下文：`scripts/agent_bootstrap.sh`
- 启动角色会话：`HUB=http://<ip>:7788 ROLE=orchestrator scripts/start_agent.sh`
- Autopilot：`scripts/autopilot.sh start|status|stop`
- Hub 健康检查：`python3 scripts/e2e_smoke.py --mode health --hub http://<ip>:7788`

## 工作约定
- 若修改协议/配置/IPC 契约，需在同一次变更中更新相关文档。
- 优先做小而明确的改动，并尊重 `docs/MODULE_OWNERS.md` 的模块边界。
- 不要引入新的长期知识/协调存储；GitHub 与现有 docs 才是长期记录。
