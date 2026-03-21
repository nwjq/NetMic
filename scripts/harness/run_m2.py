#!/usr/bin/env python3
"""NetMic M2 Harness runner."""

from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Tuple

import run_m0
import run_m1


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HOSTS_ENV = ROOT / ".harness" / "hosts.env"
DEFAULT_DURATION_SEC = 6
STARTUP_TIMEOUT_SEC = 90


@dataclass(frozen=True)
class MatrixCase:
    id: str
    title: str
    env_overrides: Dict[str, str]
    expect_fallbacks: bool
    expected_effective: Dict[str, object]


CASES: List[MatrixCase] = [
    MatrixCase(
        id="valid-opus",
        title="合法参数应原样生效",
        env_overrides={
            "NETMIC_CLIENT_CODEC": "opus",
            "NETMIC_CLIENT_SAMPLE_RATE_HZ": "32000",
            "NETMIC_CLIENT_CHUNK_MS": "40",
            "NETMIC_CLIENT_OPUS_BITRATE_KBPS": "64",
            "NETMIC_CLIENT_JITTER_BUFFER_MS": "150",
        },
        expect_fallbacks=False,
        expected_effective={
            "codec": "opus",
            "sample_rate_hz": 32000,
            "channels": 1,
            "chunk_ms": 40,
            "opus_bitrate_kbps": 64,
            "jitter_buffer_ms": 150,
        },
    ),
    MatrixCase(
        id="fallback-pcm16",
        title="越界参数应自动回退并记录 fallback",
        env_overrides={
            "NETMIC_CLIENT_CODEC": "pcm16",
            "NETMIC_CLIENT_SAMPLE_RATE_HZ": "12345",
            "NETMIC_CLIENT_CHUNK_MS": "15",
            "NETMIC_CLIENT_OPUS_BITRATE_KBPS": "999",
            "NETMIC_CLIENT_JITTER_BUFFER_MS": "999",
        },
        expect_fallbacks=True,
        expected_effective={
            "codec": "pcm16",
            "sample_rate_hz": 48000,
            "channels": 1,
            "chunk_ms": 20,
            "opus_bitrate_kbps": None,
            "jitter_buffer_ms": 100,
        },
    ),
]


def now_iso() -> str:
    return datetime.now().astimezone().isoformat(timespec="seconds")


def build_manifest(
    env: Dict[str, str],
    hosts_env: Path,
    run_id: str,
    run_dir: Path,
    duration_sec: int,
) -> Dict[str, object]:
    return {
        "run_id": run_id,
        "milestone": "M2",
        "work_package": "[harness] 参数矩阵 + fallback + UI 一致性",
        "topology": "macOS Client + Linux Server",
        "hosts_env": run_m0.display_path(hosts_env),
        "artifact_dir": run_m0.display_path(run_dir),
        "runtime": {
            "duration_sec": duration_sec,
            "startup_timeout_sec": STARTUP_TIMEOUT_SEC,
            "cases": [
                {
                    "id": case.id,
                    "title": case.title,
                    "env_overrides": case.env_overrides,
                    "expect_fallbacks": case.expect_fallbacks,
                    "expected_effective": case.expected_effective,
                }
                for case in CASES
            ],
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


def build_client_config(
    env: Dict[str, str],
    requested: Dict[str, object],
) -> Dict[str, object]:
    codec = str(requested.get("codec") or "opus")
    opus_bitrate = requested.get("opus_bitrate_kbps")
    return {
        "server_addr": env.get("NETMIC_HARNESS_SERVER_HOST", ""),
        "server_port": int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000"),
        "input_device": env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", ""),
        "codec": codec,
        "sample_rate_hz": int(requested.get("sample_rate_hz") or 48000),
        "channels": int(requested.get("channels") or 1),
        "chunk_ms": int(requested.get("chunk_ms") or 20),
        "opus_bitrate_kbps": int(opus_bitrate or 48),
        "jitter_buffer_ms": int(requested.get("jitter_buffer_ms") or 100),
        "auto_reconnect": True,
        "pairing_token": "",
    }


def build_snapshot(
    env: Dict[str, str],
    case: MatrixCase,
    session_report: Dict[str, object],
    status_payload: Dict[str, object],
    verdict: str,
    detail: str,
    logs: List[Dict[str, object]],
) -> Dict[str, object]:
    requested = session_report.get("requested") or {}
    effective = session_report.get("handshake_effective") or session_report.get("effective") or {}
    fallbacks = session_report.get("fallbacks") or []
    stats = status_payload.get("stats") or {}
    snapshot_status = "streaming" if verdict == "pass" else "error"
    return {
        "mode": "client",
        "status": snapshot_status,
        "status_note": (
            f"{case.title}（Harness 观测）" if snapshot_status == "streaming" else detail
        ),
        "client_config": build_client_config(env, requested),
        "server_config": {
            "listen_port": int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000"),
            "force_takeover": False,
            "virtual_mic_enabled": True,
        },
        "effective": effective,
        "fallbacks": fallbacks,
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
            "peer_addr": f"{env.get('NETMIC_HARNESS_SERVER_HOST', '')}:{env.get('NETMIC_HARNESS_SERVER_PORT', '43000')}",
            "connected_seconds": int(status_payload.get("active_client_seconds") or 0),
            "reconnect_attempts": 0,
            "mic_permission": "已授权",
            "virtual_mic_name": str(status_payload.get("virtual_mic_name") or "未设置"),
            "virtual_mic_ready": bool(status_payload.get("virtual_mic_ready", True)),
            "virtual_mic_error": status_payload.get("virtual_mic_error"),
            "server_status_updated_ms": run_m0.now_ms(),
            "last_error": None if verdict == "pass" else detail,
        },
        "devices": {
            "input": [
                env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", "") or "系统默认",
            ]
        },
        "logs": logs,
    }


def load_json_or_empty(path: Path) -> Dict[str, object]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def wait_for_server_ready(env: Dict[str, str], port: int, request_id: str) -> Dict[str, object]:
    deadline = time.time() + STARTUP_TIMEOUT_SEC
    while time.time() < deadline:
        result = run_m1.remote_status(env, port, request_id)
        if result.returncode == 0 and (result.stdout or "").strip():
            return json.loads(result.stdout)
        time.sleep(1)
    return {}


def poll_streaming_status(
    env: Dict[str, str],
    port: int,
    request_id: str,
    proc: subprocess.Popen[str],
) -> Dict[str, object]:
    while proc.poll() is None:
        result = run_m1.remote_status(env, port, request_id)
        if result.returncode == 0 and (result.stdout or "").strip():
            current = json.loads(result.stdout)
            if current.get("state") == "streaming" and current.get("active_client"):
                return current
        time.sleep(1)
    return {}


def verify_effective(
    actual: Dict[str, object],
    expected: Dict[str, object],
) -> Optional[str]:
    for key, expected_value in expected.items():
        if actual.get(key) != expected_value:
            return f"生效参数不匹配：{key}={actual.get(key)!r}，期望 {expected_value!r}"
    return None


def verify_ui_artifacts(
    ui_dir: Path,
    requested: Dict[str, object],
    effective: Dict[str, object],
    fallbacks: List[Dict[str, object]],
) -> Optional[str]:
    visible_status = load_json_or_empty(ui_dir / "visible-status.json")
    visible_config = load_json_or_empty(ui_dir / "visible-config.json")
    if visible_status.get("effective_values") != effective:
        return "UI 状态页的 effective_values 与生效参数不一致"

    client_config_values = visible_config.get("client_config_values") or {}
    for key in (
        "codec",
        "sample_rate_hz",
        "channels",
        "chunk_ms",
        "opus_bitrate_kbps",
        "jitter_buffer_ms",
    ):
        requested_value = requested.get(key)
        rendered_value = client_config_values.get(key)
        if requested_value != rendered_value:
            return f"UI 配置页字段不一致：{key}={rendered_value!r}，期望 {requested_value!r}"

    if visible_config.get("fallback_items") != fallbacks:
        return "UI 配置页的 fallback_items 与 session report 不一致"

    fallback_lines = [str(line) for line in visible_config.get("fallback_lines") or []]
    for item in fallbacks:
        field = str(item.get("field") or "")
        requested_value = str(item.get("requested") or "")
        applied_value = str(item.get("applied") or "")
        if not any(field in line and requested_value in line and applied_value in line for line in fallback_lines):
            return f"UI fallback 文案未覆盖字段：{field}"

    params_lines = [str(line) for line in visible_status.get("params_lines") or []]
    expected_lines = [
        f"Codec：{effective.get('codec')}",
        f"采样率：{effective.get('sample_rate_hz')} Hz",
        f"声道：{effective.get('channels')}",
        f"Chunk：{effective.get('chunk_ms')} ms",
        f"Opus Bitrate：{effective.get('opus_bitrate_kbps') if effective.get('opus_bitrate_kbps') is not None else '--'}",
        f"Buffer：{effective.get('jitter_buffer_ms')} ms",
    ]
    for line in expected_lines:
        if line not in params_lines:
            return f"UI 状态页缺少参数文案：{line}"
    return None


def run_case(
    env: Dict[str, str],
    run_dir: Path,
    case: MatrixCase,
    duration_sec: int,
) -> Tuple[run_m0.StepResult, Dict[str, object]]:
    case_root = run_dir / "cases" / case.id
    client_dir = case_root / "client"
    server_dir = case_root / "server"
    ui_dir = case_root / "ui"
    for path in (client_dir, server_dir, ui_dir):
        run_m0.ensure_dir(path)

    remote_dir = f"{env['NETMIC_HARNESS_LINUX_ROOT']}/.harness/runs/{run_dir.name}/cases/{case.id}/server"
    port = int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000")
    bootstrap_log = server_dir / "bootstrap.log"
    status_json_path = server_dir / "status.json"
    server_runtime_log = server_dir / "runtime.log"
    client_runtime_log = client_dir / "runtime.log"
    session_report_path = client_dir / "session-report.json"
    audio_dump_path = server_dir / "audio_dump.pcm"

    logs: List[Dict[str, object]] = [run_m0.step_log("info", f"开始 M2 case：{case.id}")]
    status_payload: Dict[str, object] = {}
    session_report: Dict[str, object] = {}
    verdict = "pass"
    summary = f"{case.id} 通过"

    server_start = run_m1.remote_start_server(env, remote_dir, port)
    run_m0.append_section(bootstrap_log, "remote-start-server", server_start.stdout, server_start.stderr)
    if server_start.returncode != 0:
        output = "\n".join(part for part in (server_start.stdout, server_start.stderr) if part).strip()
        verdict = run_m0.classify_output(output)
        summary = run_m0.build_command_failure_summary("远端服务端启动失败", output)
    else:
        startup_status = wait_for_server_ready(env, port, f"{run_dir.name}-{case.id}-startup")
        run_m0.write_json(status_json_path, {"startup": startup_status})
        if not startup_status:
            verdict = "fail"
            summary = "服务端未在启动窗口内进入可查询状态"
        else:
            client_env = os.environ.copy()
            client_env["RUST_LOG"] = "info"
            client_env["NETMIC_SERVER_ADDR"] = f"{env['NETMIC_HARNESS_SERVER_HOST']}:{port}"
            client_env["NETMIC_CLIENT_DEMO_SEND"] = "1"
            client_env["NETMIC_CLIENT_STREAM_SECS"] = str(duration_sec)
            client_env["NETMIC_CLIENT_SESSION_REPORT"] = str(session_report_path)
            if env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", "").strip():
                client_env["NETMIC_CLIENT_INPUT_DEVICE"] = env["NETMIC_HARNESS_CLIENT_INPUT_DEVICE"].strip()
            client_env.update(case.env_overrides)

            with client_runtime_log.open("w", encoding="utf-8") as handle:
                proc = subprocess.Popen(
                    ["cargo", "run", "-p", "netmic-client", "--quiet"],
                    cwd=str(ROOT),
                    env=client_env,
                    stdout=handle,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                streaming_status = poll_streaming_status(env, port, f"{run_dir.name}-{case.id}-active", proc)
                client_rc = proc.wait()

            final_status_result = run_m1.remote_status(env, port, f"{run_dir.name}-{case.id}-final")
            final_status = (
                json.loads(final_status_result.stdout)
                if final_status_result.returncode == 0 and (final_status_result.stdout or "").strip()
                else {}
            )
            status_payload = streaming_status or final_status or startup_status
            run_m0.write_json(
                status_json_path,
                {"startup": startup_status, "active": streaming_status, "final": final_status},
            )

            remote_runtime = run_m1.remote_fetch_text(env, f"{remote_dir}/runtime.log")
            run_m0.write_text(server_runtime_log, remote_runtime.stdout or "")

            audio_dump_result = run_m1.remote_fetch_binary_base64(env, f"{remote_dir}/audio_dump.pcm")
            if audio_dump_result.returncode == 0 and (audio_dump_result.stdout or "").strip():
                audio_dump_path.write_bytes(base64.b64decode(audio_dump_result.stdout.encode("ascii")))
            else:
                audio_dump_path.write_bytes(b"")

            client_output = client_runtime_log.read_text(encoding="utf-8")
            server_output = server_runtime_log.read_text(encoding="utf-8")
            session_report = load_json_or_empty(session_report_path)
            audio_dump_size = audio_dump_path.stat().st_size if audio_dump_path.exists() else 0

            requested = session_report.get("requested") or {}
            effective = session_report.get("effective") or {}
            handshake_effective = session_report.get("handshake_effective") or {}
            fallbacks = session_report.get("fallbacks") or []

            if client_rc != 0:
                verdict = run_m1.classify_client_failure(client_output)
                summary = "客户端推流失败"
            elif "client params resolved from env" not in client_output:
                verdict = "fail"
                summary = "客户端未记录参数解析结果"
            elif "handshake accepted with effective params" not in client_output:
                verdict = "fail"
                summary = "客户端未记录握手成功"
            elif "handled handshake request" not in server_output:
                verdict = "fail"
                summary = "服务端未记录握手处理"
            elif not session_report:
                verdict = "fail"
                summary = "缺少 session report 产物"
            elif effective != handshake_effective:
                verdict = "fail"
                summary = "本地归一化参数与握手生效参数不一致"
            elif (not case.expect_fallbacks) and fallbacks:
                verdict = "fail"
                summary = "合法参数 case 不应产生 fallback"
            elif case.expect_fallbacks and not fallbacks:
                verdict = "fail"
                summary = "越界参数 case 未产生 fallback"
            else:
                mismatch = verify_effective(handshake_effective, case.expected_effective)
                if mismatch:
                    verdict = "fail"
                    summary = mismatch
                elif not streaming_status or streaming_status.get("state") != "streaming":
                    verdict = "fail"
                    summary = "Harness 未在运行中观测到 streaming 状态"
                elif audio_dump_size <= 0:
                    verdict = "fail"
                    summary = "服务端 audio dump 为空"
                else:
                    snapshot = build_snapshot(env, case, session_report, status_payload, "pass", summary, logs)
                    ui_step = run_m0.run_ui_verify(snapshot, ui_dir)
                    if ui_step.status != "pass":
                        verdict = ui_step.status
                        summary = ui_step.summary
                    else:
                        ui_error = verify_ui_artifacts(ui_dir, requested, handshake_effective, fallbacks)
                        if ui_error:
                            verdict = "fail"
                            summary = ui_error
                    if verdict == "pass":
                        logs.append(run_m0.step_log("info", f"{case.id} 参数矩阵与 UI 对齐通过"))
            if verdict != "pass":
                snapshot = build_snapshot(
                    env,
                    case,
                    session_report if session_report else {},
                    status_payload,
                    verdict,
                    summary,
                    logs,
                )
                ui_step = run_m0.run_ui_verify(snapshot, ui_dir)
                if ui_step.status != "pass" and verdict == "pass":
                    verdict = ui_step.status
                    summary = ui_step.summary

    run_m1.remote_stop_server(env, remote_dir)

    artifacts = [
        run_m0.relative_artifact(client_runtime_log),
        run_m0.relative_artifact(session_report_path),
        run_m0.relative_artifact(server_runtime_log),
        run_m0.relative_artifact(audio_dump_path),
        run_m0.relative_artifact(status_json_path),
        run_m0.relative_artifact(ui_dir / "snapshot.json"),
        run_m0.relative_artifact(ui_dir / "visible-status.json"),
        run_m0.relative_artifact(ui_dir / "visible-config.json"),
        run_m0.relative_artifact(ui_dir / "visible-logs.json"),
        run_m0.relative_artifact(ui_dir / "refresh-check.json"),
    ]
    step = run_m0.StepResult(f"run-{case.id}", verdict, summary, artifacts)
    case_report = {
        "case_id": case.id,
        "title": case.title,
        "status": verdict,
        "summary": summary,
        "artifacts": artifacts,
    }
    run_m0.write_json(case_root / "case-report.json", case_report)
    return step, case_report


def main() -> int:
    parser = argparse.ArgumentParser(description="Run NetMic M2 harness")
    parser.add_argument("--hosts-env", default=str(DEFAULT_HOSTS_ENV))
    parser.add_argument("--run-id", default="")
    parser.add_argument("--duration", type=int, default=DEFAULT_DURATION_SEC)
    args = parser.parse_args()

    hosts_env = Path(args.hosts_env).expanduser().resolve()
    run_id = args.run_id or datetime.now().astimezone().strftime("m2-%Y%m%dT%H%M%S")
    duration_sec = max(3, int(args.duration))
    env = run_m0.parse_env_file(hosts_env)
    artifact_root_raw = env.get("NETMIC_HARNESS_ARTIFACT_DIR", ".harness/runs")
    artifact_root = Path(artifact_root_raw) if os.path.isabs(artifact_root_raw) else ROOT / artifact_root_raw
    run_dir = artifact_root / run_id
    for path in (run_dir / "client", run_dir / "server", run_dir / "ui"):
        run_m0.ensure_dir(path)

    manifest = build_manifest(env, hosts_env, run_id, run_dir, duration_sec)
    run_m0.write_json(run_dir / "manifest.json", manifest)

    steps: List[run_m0.StepResult] = []
    prepare_status, prepare_summary = run_m0.require_local_tools(
        password_auth=bool(env.get("NETMIC_HARNESS_LINUX_PASSWORD", "") and not env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")),
        needs_node=True,
        needs_rsync=True,
    )
    if shutil.which("cargo") is None:
        prepare_status = "blocked"
        prepare_summary = "缺少本地依赖：cargo"
    missing_summary = run_m1.require_keys(env)
    if missing_summary:
        prepare_status = "blocked"
        prepare_summary = missing_summary
    steps.append(
        run_m0.StepResult(
            "prepare",
            prepare_status,
            prepare_summary,
            [run_m0.relative_artifact(run_dir / "manifest.json")],
        )
    )
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
    if sync_step.status != "pass":
        run_m0.write_json(
            run_dir / "report.json",
            {
                "run_id": run_id,
                "status": sync_step.status,
                "summary": sync_step.summary,
                "started_at": manifest["started_at"],
                "finished_at": now_iso(),
                "steps": [run_m0.asdict(step) for step in steps],
            },
        )
        print(sync_step.summary)
        return 2 if sync_step.status == "blocked" else 1

    case_reports: List[Dict[str, object]] = []
    final_verdict = "pass"
    final_summary = "M2 Harness 完成：参数矩阵、fallback、UI 产物全部通过"
    for case in CASES:
        step, case_report = run_case(env, run_dir, case, duration_sec)
        steps.append(step)
        case_reports.append(case_report)
        if step.status != "pass":
            final_verdict = step.status
            final_summary = f"{case.id} 失败：{step.summary}"
            break

    run_m0.write_json(run_dir / "matrix.json", case_reports)
    run_m0.write_json(
        run_dir / "report.json",
        {
            "run_id": run_id,
            "status": final_verdict,
            "summary": final_summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [run_m0.asdict(step) for step in steps],
            "cases": case_reports,
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
