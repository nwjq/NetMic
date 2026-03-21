#!/usr/bin/env python3
"""NetMic M1 Harness runner."""

from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional

import run_m0


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HOSTS_ENV = ROOT / ".harness" / "hosts.env"
DEFAULT_DURATION_SEC = 10
STARTUP_TIMEOUT_SEC = 90


def now_iso() -> str:
    return datetime.now().astimezone().isoformat(timespec="seconds")


def build_manifest(env: Dict[str, str], hosts_env: Path, run_id: str, run_dir: Path, duration_sec: int) -> Dict[str, object]:
    return {
        "run_id": run_id,
        "milestone": "M1",
        "work_package": "[harness] macOS client -> Linux server default path",
        "topology": "macOS Client + Linux Server",
        "hosts_env": run_m0.display_path(hosts_env),
        "artifact_dir": run_m0.display_path(run_dir),
        "runtime": {
            "duration_sec": duration_sec,
            "startup_timeout_sec": STARTUP_TIMEOUT_SEC,
        },
        "server": {
            "host": env.get("NETMIC_HARNESS_LINUX_HOST", ""),
            "port": env.get("NETMIC_HARNESS_SERVER_PORT", "43000"),
            "root": env.get("NETMIC_HARNESS_LINUX_ROOT", ""),
        },
        "client": {
            "root": env.get("NETMIC_HARNESS_MAC_ROOT", str(ROOT)),
            "input_device": env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", ""),
        },
        "repo": run_m0.collect_repo_state(),
        "started_at": now_iso(),
    }


def require_keys(env: Dict[str, str]) -> Optional[str]:
    keys = [
        "NETMIC_HARNESS_LINUX_HOST",
        "NETMIC_HARNESS_LINUX_USER",
        "NETMIC_HARNESS_LINUX_ROOT",
        "NETMIC_HARNESS_SERVER_HOST",
        "NETMIC_HARNESS_SERVER_PORT",
    ]
    missing = [key for key in keys if not env.get(key, "").strip()]
    if missing:
        return "缺少现场配置：" + ", ".join(missing)
    return None


def remote_status_command(port: int, request_id: str) -> str:
    snippet = f"""
import json
import socket
import sys

sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.settimeout(1.5)
req = {{
    "type": "server_status_request",
    "payload": {{"request_id": "{request_id}"}}
}}
sock.sendto(bytes([0]) + json.dumps(req, ensure_ascii=False).encode("utf-8"), ("127.0.0.1", {port}))
data, _ = sock.recvfrom(65535)
if not data or data[0] != 0:
    raise SystemExit("invalid status response")
msg = json.loads(data[1:].decode("utf-8"))
print(json.dumps(msg.get("payload", {{}}), ensure_ascii=False))
"""
    return "python3 - <<'PY'\n" + snippet + "\nPY"


def remote_start_server(env: Dict[str, str], remote_dir: str, port: int) -> run_m0.CommandResult:
    script = f"""
set -euo pipefail
mkdir -p {remote_dir}
if [ -f {remote_dir}/server.pid ]; then
  pid="$(cat {remote_dir}/server.pid || true)"
  if [ -n "$pid" ] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid" >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5; do
      if ! kill -0 "$pid" >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -9 "$pid" >/dev/null 2>&1 || true
      sleep 1
    fi
  fi
fi
rm -f {remote_dir}/server.pid {remote_dir}/runtime.log {remote_dir}/audio_dump.pcm
nohup env \
  RUST_LOG=info \
  NETMIC_SERVER_UDP_PORT={port} \
  NETMIC_SERVER_VIRTUAL_MIC_AUTO_CREATE=1 \
  NETMIC_SERVER_AUDIO_SINK=pulse \
  NETMIC_SERVER_AUDIO_DUMP={remote_dir}/audio_dump.pcm \
  cargo run -p netmic-server --quiet \
  > {remote_dir}/runtime.log 2>&1 < /dev/null &
echo $! > {remote_dir}/server.pid
sleep 1
cat {remote_dir}/server.pid
"""
    return run_m0.run_remote(env, script)


def remote_stop_server(env: Dict[str, str], remote_dir: str) -> run_m0.CommandResult:
    script = f"""
set +e
if [ -f {remote_dir}/server.pid ]; then
  pid="$(cat {remote_dir}/server.pid)"
  if [ -n "$pid" ]; then
    kill "$pid" >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5; do
      if ! kill -0 "$pid" >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill -9 "$pid" >/dev/null 2>&1 || true
      sleep 1
    fi
  fi
  rm -f {remote_dir}/server.pid
fi
"""
    return run_m0.run_remote(env, script)


def remote_fetch_text(env: Dict[str, str], remote_path: str) -> run_m0.CommandResult:
    return run_m0.run_remote(env, f"test -f {remote_path} && cat {remote_path}")


def remote_fetch_binary_base64(env: Dict[str, str], remote_path: str) -> run_m0.CommandResult:
    script = f"""
python3 - <<'PY'
import base64
import pathlib
import sys
path = pathlib.Path("{remote_path}")
if not path.exists():
    sys.exit(1)
sys.stdout.write(base64.b64encode(path.read_bytes()).decode("ascii"))
PY
"""
    return run_m0.run_remote(env, script)


def remote_status(env: Dict[str, str], port: int, request_id: str) -> run_m0.CommandResult:
    return run_m0.run_remote(env, remote_status_command(port, request_id))


def classify_client_failure(output: str) -> str:
    lowered = output.lower()
    blocked_patterns = (
        "permission",
        "microphone",
        "device unavailable",
        "stream config failed",
        "default input config",
    )
    for pattern in blocked_patterns:
        if pattern in lowered:
            return "blocked"
    return "fail"


def build_snapshot(
    env: Dict[str, str],
    status_payload: Dict[str, object],
    verdict: str,
    detail: str,
    logs: List[Dict[str, object]],
) -> Dict[str, object]:
    state = str(status_payload.get("state") or ("streaming" if verdict == "pass" else "error"))
    snapshot_status = "streaming" if state == "streaming" and verdict == "pass" else ("listening" if verdict == "pass" else "error")
    stats = status_payload.get("stats") or {}
    return {
        "mode": "server",
        "status": snapshot_status,
        "status_note": "客户端推流中（Harness 观测）" if snapshot_status == "streaming" else detail,
        "client_config": {
            "server_addr": env.get("NETMIC_HARNESS_SERVER_HOST", ""),
            "server_port": int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000"),
            "input_device": env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", ""),
            "codec": "opus",
            "sample_rate_hz": 48000,
            "channels": 1,
            "chunk_ms": 20,
            "opus_bitrate_kbps": 48,
            "jitter_buffer_ms": 100,
            "auto_reconnect": True,
            "pairing_token": "",
        },
        "server_config": {
            "listen_port": int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000"),
            "force_takeover": False,
            "virtual_mic_enabled": True,
        },
        "effective": {
            "codec": "opus",
            "sample_rate_hz": 48000,
            "channels": 1,
            "chunk_ms": 20,
            "opus_bitrate_kbps": 48,
            "jitter_buffer_ms": 100,
        },
        "fallbacks": [],
        "metrics": {
            "rtt_ms": 0.0,
            "packet_loss_pct": 0.0,
            "buffer_depth_ms": float(stats.get("buffer_depth_ms") or 0.0),
            "jitter_buffer_depth_ms": float(stats.get("jitter_buffer_depth_ms") or 0.0),
            "estimated_e2e_latency_ms": float(stats.get("estimated_e2e_latency_ms") or 0.0),
            "audio_rms": float(stats.get("audio_rms") or 0.0),
            "audio_peak": int(stats.get("audio_peak") or 0),
            "uplink_kbps": 0.0,
        },
        "runtime": {
            "peer_addr": status_payload.get("active_client"),
            "connected_seconds": int(status_payload.get("active_client_seconds") or 0),
            "reconnect_attempts": 0,
            "mic_permission": "不适用",
            "virtual_mic_name": str(status_payload.get("virtual_mic_name") or "未设置"),
            "virtual_mic_ready": bool(status_payload.get("virtual_mic_ready")),
            "virtual_mic_error": status_payload.get("virtual_mic_error"),
            "server_status_updated_ms": run_m0.now_ms(),
            "last_error": None if verdict == "pass" else detail,
        },
        "devices": {"input": []},
        "logs": logs,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Run NetMic M1 harness")
    parser.add_argument("--hosts-env", default=str(DEFAULT_HOSTS_ENV))
    parser.add_argument("--run-id", default="")
    parser.add_argument("--duration", type=int, default=None)
    args = parser.parse_args()

    hosts_env = Path(args.hosts_env).expanduser().resolve()
    run_id = args.run_id or datetime.now().astimezone().strftime("m1-%Y%m%dT%H%M%S")
    env = run_m0.parse_env_file(hosts_env)
    duration_sec = (
        max(3, int(args.duration))
        if args.duration is not None
        else run_m0.env_int(env, "NETMIC_HARNESS_M1_DURATION_SEC", DEFAULT_DURATION_SEC, 3)
    )
    artifact_root_raw = env.get("NETMIC_HARNESS_ARTIFACT_DIR", ".harness/runs")
    artifact_root = Path(artifact_root_raw) if os.path.isabs(artifact_root_raw) else ROOT / artifact_root_raw
    run_dir = artifact_root / run_id
    client_dir = run_dir / "client"
    server_dir = run_dir / "server"
    ui_dir = run_dir / "ui"
    for path in (client_dir, server_dir, ui_dir):
        run_m0.ensure_dir(path)

    manifest = build_manifest(env, hosts_env, run_id, run_dir, duration_sec)
    run_m0.write_json(run_dir / "manifest.json", manifest)

    steps: List[run_m0.StepResult] = []
    logs: List[Dict[str, object]] = []

    missing_summary = require_keys(env)
    if missing_summary:
        steps.append(run_m0.StepResult("prepare", "blocked", missing_summary))
        logs.append(run_m0.step_log("error", missing_summary))
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": "blocked",
                "summary": missing_summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(missing_summary)
        return 2

    prepare_status, prepare_summary = run_m0.require_local_tools(
        password_auth=bool(env.get("NETMIC_HARNESS_LINUX_PASSWORD", "") and not env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")),
        needs_node=True,
        needs_rsync=True,
    )
    if shutil.which("cargo") is None:
        prepare_status = "blocked"
        prepare_summary = "缺少本地依赖：cargo"
    steps.append(
        run_m0.StepResult(
            "prepare",
            prepare_status,
            prepare_summary,
            [run_m0.relative_artifact(run_dir / "manifest.json")],
        )
    )
    logs.append(run_m0.step_log("info" if prepare_status == "pass" else "error", prepare_summary))
    if prepare_status != "pass":
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": prepare_status,
                "summary": prepare_summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(prepare_summary)
        return 2

    sync_step = run_m0.run_remote_sync_step(env, server_dir)
    steps.append(sync_step)
    logs.append(run_m0.step_log("info" if sync_step.status == "pass" else "error", sync_step.summary))
    if sync_step.status != "pass":
        verdict = sync_step.status
        summary = sync_step.summary
        ui_step = run_m0.run_ui_verify(build_snapshot(env, {}, verdict, summary, logs), ui_dir)
        steps.append(ui_step)
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": verdict,
                "summary": summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(summary)
        return 2 if verdict == "blocked" else 1

    port = int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000")
    remote_dir = f"{env['NETMIC_HARNESS_LINUX_ROOT']}/.harness/runs/{run_id}/server"
    bootstrap_log = server_dir / "bootstrap.log"
    server_start = remote_start_server(env, remote_dir, port)
    run_m0.append_section(bootstrap_log, "remote-start-server", server_start.stdout, server_start.stderr)
    if server_start.returncode != 0:
        output = "\n".join(part for part in (server_start.stdout, server_start.stderr) if part).strip()
        summary = run_m0.build_command_failure_summary("远端服务端启动失败", output)
        verdict = run_m0.classify_output(output)
        steps.append(run_m0.StepResult("bootstrap-linux", verdict, summary, [run_m0.relative_artifact(bootstrap_log)]))
        logs.append(run_m0.step_log("error", summary))
        ui_step = run_m0.run_ui_verify(build_snapshot(env, {}, verdict, summary, logs), ui_dir)
        steps.append(ui_step)
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": verdict,
                "summary": summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(summary)
        return 2 if verdict == "blocked" else 1

    startup_status: Dict[str, object] = {}
    startup_deadline = time.time() + STARTUP_TIMEOUT_SEC
    while time.time() < startup_deadline:
        result = remote_status(env, port, f"{run_id}-startup")
        if result.returncode == 0 and (result.stdout or "").strip():
            startup_status = json.loads(result.stdout)
            break
        time.sleep(1)
    status_json_path = server_dir / "status.json"
    run_m0.write_json(status_json_path, {"startup": startup_status})
    if not startup_status:
        summary = "服务端未在启动窗口内进入可查询状态"
        steps.append(
            run_m0.StepResult(
                "bootstrap-linux",
                "fail",
                summary,
                [run_m0.relative_artifact(bootstrap_log), run_m0.relative_artifact(status_json_path)],
            )
        )
        logs.append(run_m0.step_log("error", summary))
        remote_stop_server(env, remote_dir)
        ui_step = run_m0.run_ui_verify(build_snapshot(env, {}, "fail", summary, logs), ui_dir)
        steps.append(ui_step)
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": "fail",
                "summary": summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(summary)
        return 1

    bootstrap_summary = f"远端服务端已启动，状态={startup_status.get('state', 'unknown')}"
    steps.append(
        run_m0.StepResult(
            "bootstrap-linux",
            "pass",
            bootstrap_summary,
            [run_m0.relative_artifact(bootstrap_log), run_m0.relative_artifact(status_json_path)],
        )
    )
    logs.append(run_m0.step_log("info", bootstrap_summary))

    client_runtime_log = client_dir / "runtime.log"
    client_env = os.environ.copy()
    client_env["RUST_LOG"] = "info"
    client_env["NETMIC_SERVER_ADDR"] = f"{env['NETMIC_HARNESS_SERVER_HOST']}:{port}"
    client_env["NETMIC_CLIENT_DEMO_SEND"] = "1"
    client_env["NETMIC_CLIENT_STREAM_SECS"] = str(duration_sec)
    if env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", "").strip():
        client_env["NETMIC_CLIENT_INPUT_DEVICE"] = env["NETMIC_HARNESS_CLIENT_INPUT_DEVICE"].strip()

    streaming_status: Dict[str, object] = {}
    with client_runtime_log.open("w", encoding="utf-8") as handle:
        proc = subprocess.Popen(
            ["cargo", "run", "-p", "netmic-client", "--quiet"],
            cwd=str(ROOT),
            env=client_env,
            stdout=handle,
            stderr=subprocess.STDOUT,
            text=True,
        )
        while proc.poll() is None:
            result = remote_status(env, port, f"{run_id}-active")
            if result.returncode == 0 and (result.stdout or "").strip():
                current = json.loads(result.stdout)
                if current.get("state") == "streaming" and current.get("active_client"):
                    streaming_status = current
                    break
            time.sleep(1)
        client_rc = proc.wait()

    final_status_result = remote_status(env, port, f"{run_id}-final")
    final_status = json.loads(final_status_result.stdout) if final_status_result.returncode == 0 and (final_status_result.stdout or "").strip() else {}
    status_payload = streaming_status or final_status or startup_status
    run_m0.write_json(status_json_path, {"startup": startup_status, "active": streaming_status, "final": final_status})

    remote_runtime = remote_fetch_text(env, f"{remote_dir}/runtime.log")
    server_runtime_log = server_dir / "runtime.log"
    run_m0.write_text(server_runtime_log, remote_runtime.stdout or "")

    audio_dump_path = server_dir / "audio_dump.pcm"
    audio_dump_result = remote_fetch_binary_base64(env, f"{remote_dir}/audio_dump.pcm")
    if audio_dump_result.returncode == 0 and (audio_dump_result.stdout or "").strip():
        audio_dump_path.write_bytes(base64.b64decode(audio_dump_result.stdout.encode("ascii")))
    else:
        audio_dump_path.write_bytes(b"")

    remote_stop_server(env, remote_dir)

    client_output = client_runtime_log.read_text(encoding="utf-8")
    server_output = server_runtime_log.read_text(encoding="utf-8")
    audio_dump_size = audio_dump_path.stat().st_size if audio_dump_path.exists() else 0

    verdict = "pass"
    summary = "M1 默认链路通过"
    if client_rc != 0:
        verdict = classify_client_failure(client_output)
        summary = "客户端推流失败"
    elif "handshake accepted" not in client_output:
        verdict = "fail"
        summary = "客户端未记录握手成功"
    elif "handled handshake request" not in server_output:
        verdict = "fail"
        summary = "服务端未记录握手处理"
    elif not streaming_status or streaming_status.get("state") != "streaming":
        verdict = "fail"
        summary = "Harness 未在运行中观测到 streaming 状态"
    elif audio_dump_size <= 0:
        verdict = "fail"
        summary = "服务端 audio dump 为空"

    steps.append(
        run_m0.StepResult(
            "run",
            verdict,
            summary,
            [
                run_m0.relative_artifact(client_runtime_log),
                run_m0.relative_artifact(server_runtime_log),
                run_m0.relative_artifact(audio_dump_path),
                run_m0.relative_artifact(status_json_path),
            ],
        )
    )
    logs.append(run_m0.step_log("info" if verdict == "pass" else "error", summary))

    ui_step = run_m0.run_ui_verify(build_snapshot(env, status_payload, verdict, summary, logs), ui_dir)
    steps.append(ui_step)
    final_verdict = verdict if verdict != "pass" or ui_step.status == "pass" else ui_step.status
    final_summary = (
        "M1 Harness 完成：默认参数端到端、audio dump、UI 产物全部通过"
        if final_verdict == "pass"
        else (ui_step.summary if verdict == "pass" else summary)
    )

    run_m0.write_json(
        run_dir / "report.json",
        {
            "run_id": run_id,
            "status": final_verdict,
            "summary": final_summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [run_m0.asdict(step) for step in steps],
        },
    )
    print(f"{final_verdict}: {final_summary}")
    if final_verdict == "pass":
        return 0
    if final_verdict == "blocked":
        return 2
    return 1


if __name__ == "__main__":
    sys.exit(main())
