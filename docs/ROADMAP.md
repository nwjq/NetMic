# NetMic 里程碑与任务拆解（M0 / M1 / M2）

本文档用于规划“先做什么、后做什么”，并标注建议的 Agent Skill 使用点。

---

## Milestone 0：参数与注入验证（服务端基础能力）

**目标**  
Linux 端能创建虚拟麦克风并持续写入 PCM。

**任务清单**
1) 环境自检（pactl / pipewire-pulse）  
   - 参考脚本：`scripts/linux/audio_selfcheck.sh --json`（需要实测时再加 `--smoke`）  
2) 创建虚拟麦克风（pactl + module-virtual-source）  
   - 参考脚本：`scripts/linux/virtual_mic_smoke.sh --cleanup-only`  
3) 持续写入测试音（sine wave）  
   - 参考脚本：`scripts/linux/virtual_mic_smoke.sh --duration 300`  
4) 运行 5 分钟无中断  

**Skill 建议**  
无（不使用 Notion，调研结论直接记录到文档/Issue）

---

## Milestone 1：端到端基本通路（Client → Server）

**目标**  
客户端采集 → 网络发送 → 服务端接收 → 注入成功。

**任务清单**
1) Client 采集（macOS/Linux）  
2) Client 重采样/转 mono（内部标准）  
3) UDP 发送/接收（最小协议）  
4) Server 解码/缓冲（先用 PCM）  
5) 写入虚拟麦克风  

**Skill 建议**  
无（任务直接在 GitHub Issues 中维护）

---

## Milestone 2：可调参数 + 回退机制

**目标**  
参数可调且在安全范围内自动回退。

**任务清单**
1) 参数校验（采样率/帧长/bitrate/buffer）  
2) 回退逻辑（不支持则自动回到默认）  
3) 状态回显与日志  

**当前实现对齐（2026-01-28）**  
`netmic-proto::config::normalize_session_params` 已提供“默认值为基线 + 回退事件记录”的骨架实现；安全范围以 `MVP.md` 第 3 节为准。

**Skill 建议**  
无（参数决策写入文档/Issue）

---

## Milestone 3：稳定性与可用性

**目标**  
30 分钟运行稳定，断线恢复 10 秒内完成。

**任务清单**
1) 抖动缓冲策略  
2) 断线重连逻辑  
3) 指标上报（丢包、延迟、缓冲深度）  

**Skill 建议**  
CI 失败修复：`gh-fix-ci`

---

## 任务模板（Issue 建议格式）

标题：`[模块] 简短描述`  
内容：
- 目标  
- 输入/输出  
- 依赖  
- 验收标准  

---

## 验收规则（简化版）

- M0：虚拟麦克风可见 + 写入稳定  
- M1：默认参数端到端跑通  
- M2：参数可调 + 自动回退  
- M3：稳定性与恢复达标
- 自动化验收入口：`scripts/verify_mvp.py`（以 `docs/MVP_GATES.yaml` 为机器可读标准）
