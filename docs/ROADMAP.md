# NetMic 里程碑与任务拆解（M0 / M1 / M2 / M3）

本文档用于定义里程碑顺序、阶段目标与出口条件。

总控机制见 `docs/OPERATING_MODEL.md`，统一执行面见 `docs/HARNESS.md`。

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

**Harness 交付**
- Linux 端 bootstrap 入口明确
- 产物中至少包含环境自检结果、虚拟麦状态、smoke 日志
- 结果可判定为 `pass/fail/blocked`

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

**Harness 交付**
- 支持 `macOS Client + Linux Server` 默认链路联调
- 产物中至少包含 client/server 日志、握手结果、服务端状态与音频 dump
- 默认参数作为 golden path 固定下来

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

**Harness 交付**
- 参数矩阵可通过 Harness 驱动
- 回退事件进入 run 产物
- UI/日志/报告对同一组生效参数达成一致

---

## Milestone 3：稳定性与可用性

**目标**  
30 分钟运行稳定，断线恢复 10 秒内完成。

**任务清单**
1) 抖动缓冲策略  
2) 断线重连逻辑  
3) 指标上报（丢包、延迟、缓冲深度）  

**Harness 交付**
- 提供长时运行入口
- 提供网络中断/恢复场景入口
- 输出统一稳定性报告

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

- M0：虚拟麦克风可见 + 写入稳定 + 具备 Harness 自检产物
- M1：默认参数端到端跑通 + 具备双机 run 产物
- M2：参数可调 + 自动回退 + 回退结果进入报告
- M3：稳定性与恢复达标 + 长测结果进入报告

## 推进规则

- 当前主线始终推进第一个未完成里程碑
- 后续里程碑允许预留骨架，但不抢占当前主线
- 里程碑是否前进，以 Harness 结论和产物为准
- 所有里程碑完成之前，项目默认持续推进
- 只有现场配置缺失、且必须由用户补充时，才允许暂停
