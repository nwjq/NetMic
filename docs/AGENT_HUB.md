# Agent Hub（局域网协调器）

用于多 Agent 协作的最小 HTTP JSON 服务，刻意保持极简：
- 无鉴权
- 仅限局域网
- 事件日志有 TTL（短期保留）

## 运行

```bash
python3 agent-hub/agent_hub.py
```

环境变量：
- `AGENT_HUB_DB`（默认：`./agent_hub.db`）
- `AGENT_HUB_HOST`（默认：`0.0.0.0`）
- `AGENT_HUB_PORT`（默认：`7788`）
- `AGENT_HUB_EVENT_TTL_SECONDS`（默认：`86400`）

## 健康检查

```
GET /v1/health
```

## 事件轮询

```
GET /v1/events?since=<event_id>&limit=200
```

## 注册 / 心跳

```
POST /v1/register
POST /v1/heartbeat
```

示例：

```bash
curl -s http://<hub-ip>:7788/v1/register \
  -H 'content-type: application/json' \
  -d '{"agent_id":"linux-1","role":"integrator","capabilities":{"os":"linux"}}'
```

## 任务

```
POST /v1/claim_task
POST /v1/release_task
POST /v1/task_update
```

示例：

```bash
curl -s http://<hub-ip>:7788/v1/claim_task \
  -H 'content-type: application/json' \
  -d '{"agent_id":"linux-1","task_id":"ISSUE-1","meta":{"title":"Setup hub"}}'
```

## 测试会话

```
POST /v1/test_session_create
POST /v1/test_session_join
POST /v1/test_session_event
```

示例：

```bash
curl -s http://<hub-ip>:7788/v1/test_session_create \
  -H 'content-type: application/json' \
  -d '{"agent_id":"linux-1","params":{"server_ip":"192.168.11.2","client_ip":"192.168.11.1"}}'
```
