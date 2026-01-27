# NetMic 总体工作流（GitHub + Hub + 双机 Codex）

本文档定义：协作流程、职责分工、产出物与“何时使用哪些 Agent Skill”。  
目标：让两台机器上的 Codex 按同一规则协作，不把沟通记录留在 Git。

---

## 快速入口（推荐）

```bash
scripts/agent_bootstrap.sh
```

该脚本会以前台监督器的形式自动自检、启动 Hub/autopilot，并在阻塞时置顶提示需要用户执行的动作。
如需“单命令真相”的里程碑判定，请运行：`scripts/verify_mvp.py`（标准来自 `docs/MVP_GATES.yaml`）。

---

## 1. 基本原则

1) **GitHub 是最终事实来源**  
只保存代码、任务状态（Issue/Project）、CI 结果与发布产物。

2) **Hub 负责实时协调**  
Hub 只保存短期事件与状态（TTL），不存决策历史，不替代 GitHub。

3) **职责清晰，合并统一**  
所有补丁由 Integrator 合并；避免两台机器互相直接 push 主分支。

4) **跨机测试由 CI 统一触发**  
Linux + macOS 的 E2E 在 GitHub Actions 中执行，结果回到 CI。

---

## 2. 角色与职责

- **Integrator（建议 Linux）**
  - 合并补丁、处理冲突
  - 触发/监控 E2E
  - 决定里程碑通过与否

- **Builder（Linux/macOS）**
  - 领取任务 → 实现 → 提交补丁
  - 单元/本机测试

- **Tester（Linux/macOS）**
  - 复现问题、验证修复
  - 输出测试日志与指标

---

## 3. 协作主流程（端到端）

1) **需求拆解**  
从 MVP 规格 → 任务清单（Issues/Project）

2) **任务领取**  
Builder 在 Hub `claim_task`，标记正在进行

3) **实现与补丁提交**  
Builder 通过 Hub 提交补丁（`git diff` 或 `patch`）

4) **合并与 CI**  
Integrator 应用补丁 → push → CI/E2E 运行

5) **结果回写**  
CI 结果写入 GitHub；Hub 只存短期事件

6) **迭代进入下一任务**

---

## 4. Hub 使用边界

- 允许：注册、心跳、任务领取/释放、补丁提交、测试会话编排  
- 禁止：长期知识沉淀、需求讨论、决策记录

---

## 5. Agent Skill 触发点（Playbook）

> 不使用 Notion 时，以下仅保留与 GitHub 相关的技能触发点。

### 5.1 CI 失败自动修复
**使用**：`gh-fix-ci`  
**产出**：修复 PR 或直接提交修复 commit  
**写入**：GitHub PR/Commit

### 5.2 处理 PR 评论
**使用**：`gh-address-comments`  
**产出**：逐条回应与修复提交  
**写入**：PR 评论与 commit

---

## 6. 产出物规范

1) **每个任务必须对应 Issue**  
Issue 标题简明，带模块前缀（例如：`client/采集`）。

2) **补丁提交格式**  
通过 Hub 提交 `patch`，Integrator 统一合并。

3) **日志与测试结果**  
E2E 结果记录在 GitHub Actions，日志作为 artifact 存档。

---

## 7. 建议的标签体系

- `client` / `server` / `network` / `gui` / `infra`
- `m0` / `m1` / `m2`
- `bug` / `enhancement` / `doc`

---

## 8. 节奏建议

- 每日固定一个 “合并窗口”（Integrator）
- 每次合并必须有最小测试（即使是 smoke）
- 每周复盘：延迟/丢包/稳定性目标是否满足
