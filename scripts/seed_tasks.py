#!/usr/bin/env python3
"""为 NetMic MVP 自动驾驶注入任务种子（幂等）。"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, Iterable, List, Tuple

SEED_AGENT_ID = "bootstrap-seed"


SeedTask = Dict[str, Any]


SEED_TASKS: List[SeedTask] = [
    {
        "task_id": "BOOT-001-workspace-skeleton",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "bootstrap",
            "objective": "运行 scripts/bootstrap/bootstrap_workspace.sh 创建 Rust 工作区骨架，并确保 cargo check 通过。",
            "acceptance": [
                "根目录存在 Cargo.toml workspace 配置",
                "crates/netmic-{proto,server,client} 存在且 cargo check 通过",
                "docs/SESSION_LOG.md 记录本次骨架初始化结果",
            ],
            "doc_refs": ["MVP.md#4", "docs/ROADMAP.md"],
        },
    },
    {
        "task_id": "BOOT-002-session-docs-init",
        "meta": {
            "target_role": "scribe",
            "milestone": "bootstrap",
            "objective": "初始化 docs/SESSION_LOG.md 与 docs/DECISIONS.md 的最小结构，并记录自动驾驶入口的运行方式。",
            "acceptance": [
                "docs/SESSION_LOG.md 包含 DONE/BLOCKER/NEXT/TEST 模板",
                "docs/DECISIONS.md 包含日期与决策模板",
                "文档明确 agent_bootstrap.sh 是入口",
            ],
            "doc_refs": ["docs/CONTINUOUS_AUTOPILOT.md", "docs/WORKFLOW.md"],
        },
    },
    {
        "task_id": "M0-001-linux-audio-selfcheck",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M0",
            "objective": "实现 Linux 音频注入自检脚本（pactl/pipewire-pulse），并提供最小 smoke test。",
            "acceptance": [
                "存在 scripts/linux/audio_selfcheck.sh 或 .py",
                "脚本能输出是否可创建虚拟麦克风的结论",
                "结果写入 docs/SESSION_LOG.md",
            ],
            "doc_refs": ["MVP.md#6", "docs/ROADMAP.md#milestone-0"],
        },
    },
    {
        "task_id": "M0-virtual-mic-20260127-1",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M0",
            "objective": "在现有 audio_selfcheck 基础上补齐‘虚拟麦克风前置条件’的结构化结论（便于后续脚本复用）。",
            "acceptance": [
                "audio_selfcheck 在 --json 或等价模式下输出 machine-readable 字段",
                "至少包含 pactl 可用性、server type、默认 sink/source 关键字段",
                "为后续 virtual mic create 脚本提供可复用的检查入口（函数或脚本接口）",
            ],
            "doc_refs": ["MVP.md#6", "docs/ROADMAP.md#milestone-0"],
        },
    },
    {
        "task_id": "M0-virtual-mic-20260127-2",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M0",
            "objective": "新增虚拟麦克风创建/移除脚本（pactl + module-null-sink + module-remap-source），输出 source 名称与 id。",
            "acceptance": [
                "存在 scripts/linux/virtual_mic.sh（或同等脚本）支持 create/remove/status",
                "create 后能用 pactl list short sources 看到目标虚拟 source",
                "脚本对重复 create/remove 幂等，并在日志中明确模块 id/source 名称",
            ],
            "doc_refs": ["MVP.md#6", "docs/ROADMAP.md#milestone-0"],
        },
    },
    {
        "task_id": "M0-virtual-mic-20260127-3",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M0",
            "objective": "实现测试音持续写入链路（sine wave → 虚拟麦克风），并提供 5 分钟 smoke 入口。",
            "acceptance": [
                "存在可配置时长的写入脚本（例如 scripts/linux/virtual_mic_smoke.sh）",
                "默认测试音参数为 48k/mono/16-bit，贴近内部标准格式",
                "提供最小 stub/自测路径（无 pactl 时能明确返回 NOT_READY）并记录到 SESSION_LOG",
            ],
            "doc_refs": ["MVP.md#9", "docs/ROADMAP.md#milestone-0"],
        },
    },
    {
        "task_id": "M1-001-protocol-doc",
        "meta": {
            "target_role": "scribe",
            "milestone": "M1",
            "objective": "根据 MVP.md 的边界，产出 docs/PROTO.md（握手/心跳/统计/音频帧最小结构）。",
            "acceptance": [
                "docs/PROTO.md 描述握手请求/响应字段",
                "定义最小音频帧头结构与统计字段",
                "明确单客户端占用与 BUSY 行为",
            ],
            "doc_refs": ["MVP.md#5", "docs/MODULE_OWNERS.md"],
        },
    },
    {
        "task_id": "M1-002-server-udp-receiver",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M1",
            "objective": "在 netmic-server 中落地 UDP 接收骨架（握手/音频包分流占位），先用 PCM 跑通链路。",
            "acceptance": [
                "netmic-server 能监听 UDP 端口",
                "能区分控制面与数据面消息类型（占位实现可）",
                "cargo check 通过且写入 SESSION_LOG",
            ],
            "doc_refs": ["MVP.md#5", "docs/ROADMAP.md#milestone-1"],
        },
    },
    {
        "task_id": "M1-003-client-capture-sender",
        "meta": {
            "target_role": "builder-mac",
            "milestone": "M1",
            "objective": "在 netmic-client 中实现采集/重采样/发送的最小骨架（可先用假数据/测试音）。",
            "acceptance": [
                "netmic-client 具备发送循环骨架",
                "为后续接入 cpal/rubato 预留清晰模块结构",
                "cargo check 通过且写入 SESSION_LOG",
            ],
            "doc_refs": ["MVP.md#4", "docs/ROADMAP.md#milestone-1"],
        },
    },
    {
        "task_id": "M2-001-config-validation",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M2",
            "objective": "在 netmic-proto 中加入参数校验与回退规则骨架（安全范围以 MVP.md 为准）。",
            "acceptance": [
                "存在参数校验入口（validate 或 normalize）",
                "能表达回退到默认值的结果结构",
                "关键范围写入注释并引用 MVP.md",
            ],
            "doc_refs": ["MVP.md#3", "docs/ROADMAP.md#milestone-2"],
        },
    },
    {
        "task_id": "M3-001-metrics-and-reconnect",
        "meta": {
            "target_role": "builder-linux",
            "milestone": "M3",
            "objective": "在 server/client 侧补齐指标与重连状态机骨架（日志可观测优先）。",
            "acceptance": [
                "日志中可见状态机转换",
                "存在丢包/缓冲深度等指标结构占位",
                "文档或注释解释恢复窗口目标（10 秒内）",
            ],
            "doc_refs": ["MVP.md#8", "docs/ROADMAP.md#milestone-3"],
        },
    },
]


def request_json(method: str, url: str, payload: Dict[str, Any] | None = None, timeout: int = 10) -> Dict[str, Any]:
    data = None
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=True).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as exc:  # pragma: no cover - 网络环境依赖
        return {"ok": False, "error": f"http_{exc.code}", "detail": exc.read().decode("utf-8", "ignore")}
    except Exception as exc:  # pragma: no cover - 网络环境依赖
        return {"ok": False, "error": str(exc)}
    try:
        return json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError:
        return {"ok": False, "error": "bad_json", "raw": raw.decode("utf-8", "ignore")}


def get_tasks(hub: str) -> Dict[str, Dict[str, Any]]:
    resp = request_json("GET", f"{hub}/v1/tasks?limit=1000")
    tasks: Dict[str, Dict[str, Any]] = {}
    if not resp.get("ok"):
        return tasks
    for task in resp.get("tasks", []):
        task_id = task.get("task_id")
        if task_id:
            tasks[task_id] = task
    return tasks


def claim_then_queue(hub: str, task_id: str, meta: Dict[str, Any]) -> Tuple[bool, str]:
    claim_payload = {"agent_id": SEED_AGENT_ID, "task_id": task_id, "meta": meta}
    claim_resp = request_json("POST", f"{hub}/v1/claim_task", claim_payload)
    if not claim_resp.get("ok"):
        return False, claim_resp.get("error", "claim_failed")
    release_payload = {"agent_id": SEED_AGENT_ID, "task_id": task_id, "status": "queued"}
    release_resp = request_json("POST", f"{hub}/v1/release_task", release_payload)
    if not release_resp.get("ok"):
        return False, release_resp.get("error", "release_failed")
    return True, "queued"


def seed_tasks(hub: str, tasks: Iterable[SeedTask]) -> Dict[str, List[str]]:
    existing = get_tasks(hub)
    created: List[str] = []
    skipped: List[str] = []
    failed: List[str] = []

    for task in tasks:
        task_id = task["task_id"]
        meta = task["meta"]
        current = existing.get(task_id)
        if current:
            status = str(current.get("status", ""))
            if status in {"done", "closed"}:
                skipped.append(f"{task_id}:status={status}")
                continue
            # 已存在的未完成任务，避免抢占，保守跳过。
            skipped.append(f"{task_id}:status={status or 'existing'}")
            continue
        ok, detail = claim_then_queue(hub, task_id, meta)
        if ok:
            created.append(task_id)
        else:
            failed.append(f"{task_id}:{detail}")

    return {"created": created, "skipped": skipped, "failed": failed}


def parse_args(argv: List[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Seed NetMic MVP tasks into Agent Hub")
    parser.add_argument("--hub", required=True, help="Hub base URL, e.g. http://127.0.0.1:7788")
    return parser.parse_args(argv)


def main(argv: List[str]) -> int:
    args = parse_args(argv)
    hub = args.hub.rstrip("/")
    summary = seed_tasks(hub, SEED_TASKS)
    print(json.dumps({"ok": True, "hub": hub, **summary}, ensure_ascii=True))
    return 0 if not summary["failed"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
