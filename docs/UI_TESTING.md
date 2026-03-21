# NetMic UI 测试与可见性验收

本文档定义 NetMic UI 的正式测试面。

UI 不是附属层。对用户可见的内容是否正确、是否及时刷新，属于正式验收范围。

## 1. 验收目标

UI 测试必须同时保证：

- 可见内容正确
- 可见内容与运行态一致
- 可见内容能及时刷新
- 刷新异常时能明确暴露问题

## 2. 真相来源

UI 可见内容只应来源于：

- `UiSnapshot`
- `UiWaveform`

因此 UI 验证必须覆盖三件事：

1. 输入数据是否正确进入前端
2. 前端是否正确渲染输入
3. 渲染是否在合理时间内反映到可见区域

## 3. 测试分层

### A. 渲染层测试

目标：

- 给定一个 `UiSnapshot`，DOM 必须显示正确

当前入口：

- `apps/netmic-ui/ui/app.dom.test.mjs`

### B. 交互层测试

目标：

- 用户操作必须触发正确命令

当前入口：

- `apps/netmic-ui/ui/app.interactions.test.mjs`

### C. IPC 集成层测试

目标：

- IPC 与事件进入前端后，页面必须完成正确更新

当前入口：

- `apps/netmic-ui/ui/app.ipc.e2e.test.mjs`

### D. 真实 App 验收

目标：

- Tauri App 实际跑起来时，用户看到的内容正确，且刷新及时
- Harness 必须以“前端完成渲染后的 ack”为准，不能只看后端 snapshot 已写出

这层不是可选项。

规则：

- 单元测试和 jsdom 测试只能证明局部正确
- M3 不允许只靠单元测试给出结论
- 到 M3 时，必须有“整个 App 实际运行”的 UI 验收

## 4. 显示正确性要求

至少要验证以下映射：

- `status` -> 状态标签、状态颜色、主按钮文案
- `status_note` -> 状态说明文本
- `effective` -> 当前生效参数
- `runtime.last_error` -> 错误提示
- `runtime.virtual_mic_ready` / `runtime.virtual_mic_error` -> 虚拟麦状态
- `runtime.peer_addr` -> 对端地址
- `metrics.*` -> 指标展示
- `fallbacks` -> 回退提示
- `logs` -> 日志列表
- `devices.input` -> 输入设备列表

## 5. 刷新及时性要求

UI 刷新必须满足：

- `netmic://snapshot` 到达后，页面应在一次正常渲染周期内更新
- Server 状态轮询结果必须在当前轮询窗口内反映到页面
- 波形更新必须持续进行，不能长时间停住且无提示

当前已知目标：

- Server 状态轮询周期：1000 ms
- 波形目标频率：20 FPS

## 6. Harness 中的 UI 验收

Harness 对 UI 的要求：

1. 每个里程碑都要定义 UI 可见结果
2. `verdict` 前必须有 UI 验收结论
3. UI 验收不通过，整体 run 不算通过

建议产物：

```text
.harness/runs/<run_id>/ui/
  snapshot.json
  render-log.ndjson
  visible-status.json
  visible-config.json
  visible-logs.json
  refresh-check.json
```

## 7. 里程碑重点

### M0

- Server UI 能显示虚拟麦状态与相关错误

### M1

- Client / Server UI 能正确显示连接、监听、推流状态
- 状态变化后页面及时刷新

### M2

- 参数页、状态页、fallback 展示一致

### M3

- 长时间运行时 UI 持续刷新
- 断线与恢复状态可见
- 断线期间 render ack 必须出现可见的重连/过期提示；`snapshot.status_note` 与可见 `status_note` 需一致
- 前端 render ack 持续产生，且时间窗口满足刷新要求；其中 `snapshot.status` 与可见状态标签必须一致
- 实际 App 运行过程中，没有预期之外的可见 bug

## 8. 结论

NetMic 的 UI 验收标准不是“页面能打开”，而是：

- 显示内容正确
- 刷新时机正确
- 真实 App 运行时可见行为正确
- 这些都能进入 Harness 的统一结论
