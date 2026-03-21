# NetMic Harness 执行面

本文档只定义 NetMic 的统一执行面。

总控规则见 `docs/OPERATING_MODEL.md`；本文只回答“如何执行一次 run，并产出统一结果”。

## 1. 执行目标

- 为当前工作包提供统一执行链路
- 为当前里程碑提供统一验收入口
- 为每次 run 生成统一产物与统一结论

## 2. 标准拓扑

- `Coordinator`：通常在本地发起一次 run，负责读配置、触发双机动作、收集产物、给出结论。
- `macOS Client`：负责音频采集、重采样、发送。
- `Linux Server`：负责 UDP 接收、虚拟麦克风注入、状态与指标输出。

默认目标拓扑：

```text
macOS (client/coordinator)  --LAN/UDP-->  Linux (server/inject)
```

## 3. 上游输入

Harness 运行前，默认已由上游文档确定：

1. 当前目标里程碑
2. 当前工作包范围
3. 当前模块归属
4. 当前协议、UI、配置约束
5. 当前现场配置

对应来源通常是：

- `MVP.md`
- `docs/OPERATING_MODEL.md`
- `docs/ROADMAP.md`
- `docs/MODULE_OWNERS.md`
- `docs/PROTO.md`
- `docs/UI.md`
- `docs/ENV.md`
- `.harness/hosts.env`

## 4. 标准运行阶段

每次双机联调 run 都按以下阶段组织：

1. `prepare`
   - 读取 `.harness/hosts.env`
   - 校验本地与远端工作目录、端口、设备名等信息
   - 先比对本地工作区与 Linux 远端仓库；若未同步则先同步，再进入后续阶段
2. `bootstrap-linux`
   - 自检 PipeWire/Pulse 环境
   - 创建或确认虚拟麦克风
   - 启动或确认 `netmic-server`
3. `bootstrap-macos`
   - 校验麦克风权限与输入设备
   - 启动或确认 `netmic-client`
4. `run`
   - 按里程碑执行默认参数链路或参数矩阵
5. `collect`
   - 收集 client/server 日志、状态、音频 dump、设备信息
6. `ui-verify`
   - 校验可见状态、可见参数、可见日志与刷新时效
7. `verdict`
   - 输出 `pass` / `fail` / `blocked`
8. `promote-skill`（按需）
   - 从本次 run 的产物、日志、修复路径中提炼可复用技巧
   - 若满足复用条件，则新增或更新 `.codex/skills/<skill-name>/SKILL.md`

当前已落地入口：

- 总控：`scripts/harness/coordinator.py`
  - 扫描 `.harness/runs/*/report.json`
  - 每次启动先检查本地工作区是否已同步到 Linux 远端仓库，必要时先执行同步
  - 判断每个里程碑是否已有 `pass`
  - 从第一个未完成里程碑继续循环调度 runner，直到目标里程碑或真实 `fail/blocked`
  - runner 缺失时输出 `fail`，而不是把“未实现”误判成“已完成”
- `M0`：`scripts/harness/run_m0.py`
  - 读取 `.harness/hosts.env`
  - 先把本地工作区同步到 Linux 侧仓库
  - 通过 SSH 进入 Linux 侧仓库
  - 执行 `audio_selfcheck.sh` / `virtual_mic.sh create|status` / `virtual_mic_smoke.sh`
  - 生成 `manifest.json` / `report.json` / `server/*` / `ui/*`
  - 输出统一 `pass` / `fail` / `blocked`
- `M1`：`scripts/harness/run_m1.py`
  - 先把本地工作区同步到 Linux 侧仓库
  - 远端启动 `netmic-server`
  - 本地启动 `netmic-client`
  - 通过远端 loopback 查询 `server_status`
  - 回收 `client/server` 日志、`status.json`、`audio_dump.pcm`
  - 生成 `ui/*` 产物并输出统一 verdict
- `M2`：`scripts/harness/run_m2.py`
  - 先把本地工作区同步到 Linux 侧仓库
  - 以参数矩阵驱动 `netmic-client`
  - 回收 `session-report.json`、`audio_dump.pcm` 与 UI 可见产物
  - 校验请求参数 / 生效参数 / fallback / UI 展示一致
- `M3`：`scripts/harness/run_m3.py`
  - 先把本地工作区同步到 Linux 侧仓库
  - 本地启动真实 `netmic-ui`（Harness 自动拉起）
  - 默认执行 30 分钟真实 App 长测，并在中途打断/恢复远端 `netmic-server`
  - 校验断线前/恢复后的 snapshot 刷新连续性与稳定窗口
  - 回收真实 App 的 snapshot/event log、phase1/phase2 音频 dump 与 UI 恢复产物

## 5. 用户补充信息入口

所有需要用户补充、且不应写死在仓库中的信息，统一放在：

- 模板：`.harness/hosts.env.example`
- 本地填写文件：`.harness/hosts.env`

典型字段包括：

- Linux 主机地址、SSH 用户、SSH 端口
- Linux 侧仓库路径
- macOS 本地仓库路径
- 服务端监听端口
- 客户端输入设备名
- 本地产物目录

安全规则：

- 连接与认证方式由用户决定。
- 可使用密码、SSH key 或当前局域网内可用的其他方式。
- 需要保密的连接信息只放 `.harness/hosts.env`。

## 6. 产物约定

每次 run 的产物统一放在：

```text
.harness/runs/<run_id>/
```

建议最小结构：

```text
.harness/runs/<run_id>/
  manifest.json
  report.json
  client/
    bootstrap.log
    runtime.log
    devices.json
  server/
    bootstrap.log
    runtime.log
    selfcheck.json
    status.json
    audio_dump.pcm
  ui/
    snapshot.json
    visible-status.json
    visible-config.json
    visible-logs.json
    refresh-check.json
```

原则：

- 先保证“有统一产物”，再追求格式复杂度。
- 所有 verdict 必须能追溯到对应 run 的产物。
- 若当前里程碑暂未要求音频 dump，可先不生成 `audio_dump.pcm`，但必须补齐对应阶段的关键日志与状态文件。

## 7. 里程碑与 Harness 对齐

- `M0`：Linux 本机自检、虚拟麦创建、测试音/PCM 写入验证。
- `M1`：macOS 采集 -> 网络发送 -> Linux 接收 -> 注入，默认参数跑通。
- `M2`：参数安全范围、回退提示、状态回显进入 Harness。
- `M3`：长时间稳定性、断线恢复、指标采集进入 Harness。

UI 对齐要求：

- M0：Server UI 能显示虚拟麦状态与错误
- M1：连接/监听/推流状态在 UI 可见且及时刷新
- M2：生效参数与 fallback 在 UI 可见且正确
- M3：真实 App 运行时，UI 长时刷新、断线恢复与过期提示都正确

规则：

- 只存在代码路径、但没有进入 Harness 的能力，不算真正完成。
- 只存在手工验证、但没有明确 `pass/fail/blocked` 结果的能力，不算真正验收完成。
- 一个工作包是否结束，以 Harness 结论为准。
- Harness 给出 `pass` 后，应继续进入当前里程碑下一个工作包，或进入下一个未完成里程碑。
- Harness 给出 `fail` 后，应继续进入修复循环，而不是停下。
- 后台状态正确但 UI 显示错误、缺失或刷新过慢，Harness 不应给出 `pass`。
- 单元测试通过但真实 App 运行未验证，M3 不应给出 `pass`。

### M0 当前 run 口径

`scripts/harness/run_m0.py` 当前将 M0 切成以下检查：

1. `prepare`
   - 校验 `.harness/hosts.env`
   - 校验本机依赖（`ssh`、必要时 `sshpass`、`node`）
2. `sync-remote`
   - 对比本地工作区与 Linux 侧仓库
   - 若不一致，则通过 `rsync` 先把远端工作区同步到当前本地状态
   - `.harness/hosts.env`、`.harness/runs/`、`.codex/`、`target/`、`node_modules/` 不进入同步范围
3. `bootstrap-linux`
   - `scripts/linux/audio_selfcheck.sh --json`
   - `scripts/linux/virtual_mic.sh create`
   - `scripts/linux/virtual_mic.sh status --json`
4. `run`
   - `scripts/linux/virtual_mic_smoke.sh --duration <sec>`
   - 若日志显示“跳过音频写入”，该 run 不能判为 `pass`
5. `ui-verify`
   - 根据服务端状态生成 `ui/snapshot.json`
   - 用 `scripts/harness/render_ui_artifacts.mjs` 生成可见产物

判定规则：

- 自检、虚拟麦就绪、测试音实际写入、UI 产物齐备时，才可判定 `pass`
- 缺少 SSH / `sshpass` / `rsync` / `node`、缺少 PipeWire/Pulse 依赖、Linux 不可达、或本机/沙箱禁止 SSH 连接等，判定 `blocked`
- 其余执行错误判定为 `fail`

### 顶层 coordinator 口径

`scripts/harness/coordinator.py` 不直接代替各里程碑 runner，它负责：

1. 扫描 `.harness/runs/*/report.json`
2. 结合 `manifest.json` / `recovery.json` 判断 `M0 -> M1 -> M2 -> M3` 中第一个未完成里程碑
3. 调用对应 runner
4. 若 runner 返回 `pass`，继续推进下一里程碑
5. 若 runner 返回 `blocked`，只在真实现场阻塞时暂停
6. 若 runner 缺失或代码路径不存在，判定 `fail`

补充口径：

- `M3` 旧产物只有在满足“真实 `netmic-ui` 长测 30 分钟、恢复时长达标、前后稳定窗口刷新达标”时，才可被 coordinator 视为 `pass`
- coordinator 自身的远端同步预检同样会生成产物；若 `rsync/ssh` 失败，`sync/remote-sync.log` 与 `sync/remote-sync.json` 必须保留首个底层错误，避免 report 只剩泛化摘要

当前已存在的专门 runner：

- `M0`：`scripts/harness/run_m0.py`
- `M1`：`scripts/harness/run_m1.py`
- `M2`：`scripts/harness/run_m2.py`
- `M3`：`scripts/harness/run_m3.py`

## 8. 阻塞分类

Harness 只接受三类结果：

- `pass`：达到当前阶段验收目标。
- `fail`：代码或脚本执行后结果不符合预期。
- `blocked`：缺少用户信息、权限、系统依赖或环境前置条件。

补充规则：

- 只有 `blocked` 且阻塞确实需要用户补充配置时，才允许暂停等待。
- `pass` 与 `fail` 都属于继续推进信号。

典型 `blocked`：

- 未提供 Linux 地址或 SSH 用户
- 未提供可用的 Linux 连接方式
- macOS 未授予麦克风权限
- Linux 未安装 PipeWire/Pulse 兼容层
- 输入设备名未知

## 9. 与其他文档的关系

以下情况应同步更新本文：

- 执行阶段变更
- 产物结构变更
- verdict 口径变更
- UI 可见性检查规则变更
- `.harness/hosts.env` 入口字段变更
- 双机执行拓扑变更

协议、UI、参数、安全范围等事实不在本文定义，分别以对应主线文档为准。

## 10. 复用机会

run 结束后，如果产物和修复过程里出现了稳定可复用技巧，可以将其升级为项目私有 skill。

skill 机制的总规则由 `docs/OPERATING_MODEL.md` 和 `AGENTS.md` 约束。
UI 验收规则由 `docs/UI_TESTING.md` 约束。
