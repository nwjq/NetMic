---
name: netmic-rust-tauri
description: NetMic 的 Rust/Tauri 开发规范与最佳实践。用于编写/重构 Rust 模块、异步网络、音频管线或 Tauri 前后端交互时。
---

# NetMic Rust/Tauri 开发规范

## 适用场景
- 新增或重构 Rust 模块
- 设计异步网络或音频处理管线
- Tauri 命令与状态管理

## 核心约定
- **内部音频格式**：PCM16 / mono / 48k  
- **异步优先**：网络与控制逻辑使用 `tokio`  
- **日志统一**：使用 `tracing`，避免 `println!`  
- **错误处理**：库层 `thiserror`，应用层 `anyhow`

## 编码与结构建议
1) **分层清晰**：采集/处理/传输/注入拆分成独立模块  
2) **避免阻塞**：音频回调中只做轻量操作，重计算移到后台  
3) **统一配置**：参数校验集中在单一模块  
4) **小步集成**：先 CLI 可运行，再接 GUI

## Tauri 交互原则
- 通过 `command` 暴露 start/stop/status  
- 使用全局 `State` 存储连接与配置  
- 指标用事件流推送 UI（减少轮询）

## 测试要点
- 参数校验必须单测  
- 协议编码/解码必须单测  
- 端到端联调用 E2E workflow 验证

