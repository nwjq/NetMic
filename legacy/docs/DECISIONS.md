# DECISIONS

## 2026-01-27
- 背景：`netmic-proto` 已引用 `protocol` 模块，但仓库中缺少协议事实来源与最小结构定义，容易造成 client/server 漂移。
- 决策：新增 `docs/PROTO.md` 作为 MVP 协议事实来源，并在 `netmic-proto` 中补齐 `SessionParams / Handshake / Heartbeat / StatsSnapshot / AudioFrameHeader` 最小结构；握手阶段显式携带 `session_id`，服务端原样回显。
- 影响：后续协议字段调整必须先更新 `docs/PROTO.md`；builder 可直接围绕这些结构体落地最小 UDP 骨架。

## 2026-01-28（builder-linux / M1 UDP 接收骨架）
- 背景：M1 需要在服务端同一 UDP 端口上区分控制面与数据面，但协议尚未冻结。
- 决策：在 `netmic-proto` 中引入“首字节 kind + payload”的占位 framing，并约定 kind=0 为控制面 JSON、kind=1 为 PCM16 数据面；服务端锁定首个发送方为 active client，其余来源仅记录 busy 日志。
- 影响：后续冻结 wire format 时，必须同时更新 `docs/PROTO.md` 与 `crates/netmic-proto/src/datagram.rs` / `crates/netmic-server/src/main.rs` 的分流逻辑。
