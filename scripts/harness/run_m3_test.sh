#!/usr/bin/env bash
# run_m3.py 最小自测：覆盖长时刷新窗口判定与稳定窗口查找。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

PYTHONPATH="$ROOT/scripts/harness" python3 - "$tmpdir" <<'PY'
import json
import pathlib
import sys

import run_m3

tmpdir = pathlib.Path(sys.argv[1])
event_log = tmpdir / "event-log.ndjson"

ok_events = [
    {
        "ts_ms": 1000,
        "snapshot": {"status": "streaming"},
        "visible": {"status_label": "推流中"},
    },
    {
        "ts_ms": 2000,
        "snapshot": {"status": "streaming"},
        "visible": {"status_label": "推流中"},
    },
    {
        "ts_ms": 3900,
        "snapshot": {"status": "streaming"},
        "visible": {"status_label": "推流中"},
    },
]
report = run_m3.analyze_refresh_window(ok_events, 0, len(ok_events) - 1, "streaming")
assert report["ok"] is True, report
assert report["max_gap_ms"] == 1900, report

reconnect_report = run_m3.analyze_reconnect_visibility(
    {
        "snapshot": {
            "status": "connecting",
            "status_note": "等待服务端重连",
            "runtime": {"reconnect_attempts": 1},
        },
        "visible": {
            "status_label": "连接中",
            "status_note": "等待服务端重连",
        },
    }
)
assert reconnect_report["ok"] is True, reconnect_report

unexpected_events = ok_events + [
    {
        "ts_ms": 4800,
        "snapshot": {"status": "connecting"},
        "visible": {"status_label": "连接中"},
    },
]
report = run_m3.analyze_refresh_window(
    unexpected_events,
    0,
    len(unexpected_events) - 1,
    "streaming",
)
assert report["ok"] is False, report
assert report["unexpected_statuses"] == ["connecting"], report

gap_events = [
    {
        "ts_ms": 1000,
        "snapshot": {"status": "streaming"},
        "visible": {"status_label": "推流中"},
    },
    {
        "ts_ms": 5005,
        "snapshot": {"status": "streaming"},
        "visible": {"status_label": "推流中"},
    },
]
report = run_m3.analyze_refresh_window(gap_events, 0, len(gap_events) - 1, "streaming")
assert report["ok"] is False, report
assert report["max_gap_ms"] == 4005, report

event_log.write_text(
    "\n".join(
        [
            json.dumps(
                {
                    "ts_ms": 1000,
                    "snapshot": {"status": "streaming"},
                    "visible": {"status_label": "推流中"},
                },
                ensure_ascii=False,
            ),
            "{bad json",
            json.dumps(
                {
                    "ts_ms": 2500,
                    "snapshot": {"status": "connecting"},
                    "visible": {"status_label": "连接中"},
                },
                ensure_ascii=False,
            ),
            json.dumps(
                {
                    "ts_ms": 4100,
                    "snapshot": {"status": "streaming"},
                    "visible": {"status_label": "推流中"},
                },
                ensure_ascii=False,
            ),
            "",
        ]
    ),
    encoding="utf-8",
)

index, event = run_m3.wait_for_event_span(
    event_log,
    start_index=0,
    base_ts_ms=1000,
    duration_sec=3,
    required_status="streaming",
)
assert index == 2, (index, event)
assert event["ts_ms"] == 4100, event

index, event = run_m3.wait_for_event_span(
    event_log,
    start_index=0,
    base_ts_ms=1000,
    duration_sec=0,
    required_status="streaming",
)
assert index == 2, (index, event)
assert event["ts_ms"] == 4100, event

manifest = run_m3.build_manifest(
    {
        "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
        "NETMIC_HARNESS_SERVER_PORT": "43000",
        "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
        "NETMIC_HARNESS_MAC_ROOT": "/Users/arc/code/NetMic",
        "NETMIC_HARNESS_CLIENT_INPUT_DEVICE": "",
    },
    pathlib.Path("/Users/arc/code/NetMic/.harness/hosts.env"),
    "m3-test",
    tmpdir / "run",
    run_m3.MIN_PASS_RUNTIME_SEC,
    run_m3.MIN_PASS_RUNTIME_SEC // 2,
)
runtime = manifest["runtime"]
assert runtime["app_runtime_sec"] == run_m3.MIN_PASS_RUNTIME_SEC, runtime
assert runtime["min_pass_runtime_sec"] == run_m3.MIN_PASS_RUNTIME_SEC, runtime
assert runtime["post_recover_sec"] == run_m3.MIN_PASS_RUNTIME_SEC // 2, runtime
PY

echo "[ok] run_m3.py 最小测试通过"
