# NetMic 模块责任矩阵（双机协作）

本文档定义 Linux 与 macOS 两台机器上的模块边界、默认负责人和交付边界。
目标：让持续开发过程中，工作包有明确归属且不互相污染。

---

## 1. 角色与机器分工

### Linux 侧（Server Agent）
- 负责服务端核心功能与系统注入
- 负责服务端自检与运维脚本
- 负责服务端稳定性与性能验证

### macOS 侧（Client Agent）
- 负责客户端采集与权限处理
- 负责客户端重采样/单声道化
- 负责客户端连接/重连逻辑

### 共享模块（Integrator 统筹）
- 协议与消息结构
- 配置与参数校验
- 日志与指标格式
- 项目运行模型与文档体系
- Harness 编排、Gate 验收与产物结构
- 项目私有 skills 与技巧沉淀机制

---

## 2. 模块划分与负责人

| 模块 | 说明 | 负责人（默认） | 产出 |
| --- | --- | --- | --- |
| server/audio-inject | 虚拟麦克风创建与 PCM 写入 | Linux | 自检脚本 + 注入模块 |
| server/receiver | UDP 接收、抖动缓冲 | Linux | 接收模块 + 缓冲策略 |
| server/metrics | 丢包/缓冲/延迟统计 | Linux | 指标输出格式 |
| client/capture | 麦克风采集 | macOS | 采集模块 + 设备选择 |
| client/resample | 重采样/单声道化 | macOS | 处理链路 |
| client/sender | UDP 发送与心跳 | macOS | 发送模块 |
| gui | Tauri UI + IPC | 共享 | UI 与 IPC 文档 |
| proto | 协议结构与握手 | 共享 | 消息定义文档 |
| config | 参数校验与回退规则 | 共享 | 参数规则文档 |
| logging | 日志与错误分类 | 共享 | 日志规范 |
| operating-model | 项目总控、文档分层、持续开发机制 | 共享（Integrator 主导） | `docs/OPERATING_MODEL.md` |
| harness | 双机编排、产物归档、Gate 验收 | 共享（Integrator 主导） | Harness 文档 + 配置模板 + 验收规则 |
| skills | 项目私有 skill、技巧复用与维护 | 共享（Integrator 主导） | `.codex/skills/` 下的 skill 实体 |

---

## 3. 交付与合并规则

1) 每个任务必须对应 Issue  
2) 子模块修改尽量只改自己负责的目录  
3) 所有补丁通过 Integrator 合并  
4) 变更协议/配置必须先更新文档再改代码
5) 涉及双机地址、SSH、设备名、路径等现场信息时，不写入 Git；统一通过 `.harness/hosts.env` 由用户补充
6) 工作包跨模块时，先以 `docs/OPERATING_MODEL.md` 判定主线目标，再以本文件判定归属

---

## 4. 冲突处理规则

- 两侧同时修改同一模块时，由 Integrator 判定主导方  
- 若涉及协议变更，优先冻结两侧实现，先达成文档一致
- 若涉及 Harness 入口、Gate 口径或产物结构，由 Integrator 统一口径

---

## 5. 里程碑对齐（简要）

- **M0**：Linux 侧为主（虚拟麦克风/写入验证）  
- **M1**：Client + Server 并行（采集/发送/接收/注入）  
- **M2/M3**：共享模块为主（参数回退、稳定性、指标、Harness 验收）
