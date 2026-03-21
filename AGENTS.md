# NetMic Agent 指南

本仓库当前采用 Harness-first 工作方式。目标不是先堆实现，而是先把“双机局域网联调 + 自动化验收 + 产物收集”编排清楚，再持续补实现。

文档是事实来源（source of truth）。若代码、脚本、注释与文档冲突，以文档为准，并在同一次变更中修正文档或实现。

## 语言约定（重要）
- 面向开发者的文档、说明与注释，尽量使用中文。
- 代码标识符（变量名、函数名、类型名、文件名）保持英文为主。
- 如必须引用英文资料或术语，建议同时给出中文解释或总结。

## 优先阅读（按顺序）
- `MVP.md`：产品范围、核心约束与参数安全范围。
- `docs/OPERATING_MODEL.md`：项目总控、持续开发主线、文档分层与 skill 机制。
- `docs/HARNESS.md`：双机 Harness 执行链路、产物与结论口径。
- `docs/ROADMAP.md`：里程碑顺序与验收规则。
- `docs/MODULE_OWNERS.md`：模块责任边界与冲突处理规则。
- `docs/ENV.md`：运行时变量与 Harness 本地配置约定。
- `docs/PROTO.md` / `docs/UI.md`：协议、状态机、UI/IPC 契约（按需阅读）。

## 全局原则
- 先看全局控制面，再看局部执行面。
- 当前主目标始终是“第一个未完成的里程碑”。
- 每次工作都必须落到一个明确工作包，而不是散点改动。
- 每次工作都必须能进入 Harness，并生成统一产物与结论。
- 事实写入 `docs/`，方法写入 `.codex/skills/`。
- 除非 `docs/ROADMAP.md` 全部里程碑完成，或当前主线被真实配置阻塞，否则不能自行停下。

## Harness 原则
- 默认目标拓扑：`macOS Client + Linux Server`，通过局域网完成 M0-M3。
- 所有缺失的主机地址、SSH 用户、远端仓库路径、输入设备名等，统一写入 `.harness/hosts.env`，由用户补充；不要猜测或硬编码。
- 需要保密的连接信息只放 `.harness/hosts.env`。
- 每个新增能力都应回答两个问题：
  - 它服务于哪个里程碑（M0/M1/M2/M3）？
  - 它如何进入 Harness 验收链路？
- 对双机联调相关改动，优先补齐文档、配置模板、产物结构与阻塞提示，再补实现。

## 本地 Skills
- 当前仓库未内置项目私有 skills。
- 需要时可以新增项目私有 skill。
- 新增 skill 必须满足：
  - 来自真实工作
  - 直接服务当前主线里程碑或重复性工作流
  - 不替代 source-of-truth 文档
  - 与 Harness 工作流保持一致
- 项目私有 skill 使用 Codex 标准结构，放在 `.codex/skills/<skill-name>/SKILL.md`。
- 可按需包含 `agents/openai.yaml`、`scripts/`、`references/`、`assets/`。
- 当某个技巧被重复验证、且能稳定减少排障或实现成本时，应将其升级为项目私有 skill。

## 仓库结构速览
- `crates/`：核心代码（client/server/proto）。
- `scripts/`：开发自检/验证脚本（当前以 `scripts/linux` 为主）。
- `docs/`：规范、Harness、里程碑与接口文档。
- `.harness/`：本地双机配置模板与未来的联调产物目录。

## 常用命令
- 复制本地配置模板：`cp .harness/hosts.env.example .harness/hosts.env`
- Linux 环境自检：`scripts/linux/audio_selfcheck.sh --json`
- 虚拟麦克风管理：`scripts/linux/virtual_mic.sh create|remove|status`
- 虚拟麦 smoke 验证：`scripts/linux/virtual_mic_smoke.sh --duration 300`

> 说明：`scripts/linux` 当前仅用于开发验证，不作为运行时依赖；双机 Harness 主入口以 `docs/HARNESS.md` 约定为准，后续脚本应向该文档对齐。

## 工作约定
- 所有工作优先服从 `docs/OPERATING_MODEL.md` 的主线。
- 若修改协议/配置/IPC 契约，需在同一次变更中更新相关文档。
- 优先做小而明确的改动，并尊重 `docs/MODULE_OWNERS.md` 的模块边界。
- 不要引入新的长期知识/协调存储；GitHub 与现有 docs 才是长期记录。
- 若因缺少远端信息而无法继续，应明确标记为 `blocked`，并指出由用户补充 `.harness/hosts.env` 的哪些字段。
- `pass` 不表示停下，只表示进入下一个工作包；`fail` 不表示停下，只表示继续修复。
