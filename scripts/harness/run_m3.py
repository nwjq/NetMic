#!/usr/bin/env python3
"""NetMic M3 Harness runner."""

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
from typing import Callable, Dict, List, Optional, Tuple

import run_m0
import run_m1


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HOSTS_ENV = ROOT / ".harness" / "hosts.env"
APP_RUNTIME_SEC = 30 * 60
MIN_PASS_RUNTIME_SEC = APP_RUNTIME_SEC
DEFAULT_DISCONNECT_AFTER_SEC = APP_RUNTIME_SEC // 2
STREAM_TIMEOUT_SEC = 45
RECONNECT_TIMEOUT_SEC = 15
RECOVER_TIMEOUT_SEC = 15
RECOVERY_TARGET_MS = 10_000
REFRESH_GAP_TARGET_MS = 3_000
STABILITY_WAIT_PAD_SEC = 30
STATUS_LABELS = {
    "idle": "空闲",
    "connecting": "连接中",
    "listening": "监听中",
    "connected": "已连接",
    "streaming": "推流中",
    "error": "错误",
}
PROCESS_STOP_TIMEOUT_SEC = 5


def now_iso() -> str:
    return datetime.now().astimezone().isoformat(timespec="seconds")


def load_events(path: Path) -> List[Dict[str, object]]:
    events: List[Dict[str, object]] = []
    if not path.exists():
        return events
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def wait_for_event(
    event_log: Path,
    start_index: int,
    timeout_sec: int,
    predicate: Callable[[Dict[str, object]], bool],
) -> Tuple[Optional[int], Optional[Dict[str, object]]]:
    deadline = time.time() + timeout_sec
    while time.time() < deadline:
        events = load_events(event_log)
        for index in range(start_index, len(events)):
            event = events[index]
            if predicate(event):
                return index, event
        time.sleep(1)
    return None, None


def event_ts_ms(event: Dict[str, object]) -> int:
    return int(event.get("ts_ms") or 0)


def event_status(event: Dict[str, object]) -> str:
    return str((event.get("snapshot") or {}).get("status") or "")


def event_visible_label(event: Dict[str, object]) -> str:
    return str((event.get("visible") or {}).get("status_label") or "")


def wait_for_event_span(
    event_log: Path,
    start_index: int,
    base_ts_ms: int,
    duration_sec: int,
    required_status: str,
) -> Tuple[Optional[int], Optional[Dict[str, object]]]:
    if duration_sec <= 0:
        events = load_events(event_log)
        if not events:
            return None, None
        return len(events) - 1, events[-1]

    deadline = time.time() + duration_sec + STABILITY_WAIT_PAD_SEC
    target_span_ms = duration_sec * 1000
    while time.time() < deadline:
        events = load_events(event_log)
        for index in range(start_index, len(events)):
            event = events[index]
            if required_status and event_status(event) != required_status:
                continue
            if event_ts_ms(event) - base_ts_ms >= target_span_ms:
                return index, event
        time.sleep(1)
    return None, None


def analyze_refresh_window(
    events: List[Dict[str, object]],
    start_index: int,
    end_index: int,
    expected_status: str,
) -> Dict[str, object]:
    window = events[start_index : end_index + 1]
    timestamps = [event_ts_ms(event) for event in window if event_ts_ms(event) > 0]
    statuses = [event_status(event) for event in window if event_status(event)]
    visible_labels = [event_visible_label(event) for event in window if event_visible_label(event)]
    max_gap_ms = 0
    if len(timestamps) >= 2:
        max_gap_ms = max(
            later - earlier for earlier, later in zip(timestamps, timestamps[1:])
        )
    unexpected_statuses = sorted({status for status in statuses if status != expected_status})
    expected_visible_label = STATUS_LABELS.get(expected_status, "")
    missing_status_count = sum(1 for event in window if not event_status(event))
    missing_visible_label_count = sum(1 for event in window if not event_visible_label(event))
    unexpected_visible_labels = sorted(
        {label for label in visible_labels if label != expected_visible_label}
    )
    return {
        "ok": len(timestamps) >= 2
        and not missing_status_count
        and not missing_visible_label_count
        and not unexpected_statuses
        and not unexpected_visible_labels
        and max_gap_ms <= REFRESH_GAP_TARGET_MS,
        "event_count": len(window),
        "span_ms": (timestamps[-1] - timestamps[0]) if len(timestamps) >= 2 else 0,
        "max_gap_ms": max_gap_ms,
        "expected_status": expected_status,
        "expected_visible_label": expected_visible_label,
        "statuses": sorted(set(statuses)),
        "unexpected_statuses": unexpected_statuses,
        "visible_labels": sorted(set(visible_labels)),
        "unexpected_visible_labels": unexpected_visible_labels,
        "missing_status_count": missing_status_count,
        "missing_visible_label_count": missing_visible_label_count,
    }


def stop_ui_process(proc: subprocess.Popen[str]) -> bool:
    if proc.poll() is not None:
        return False
    proc.terminate()
    try:
        proc.wait(timeout=PROCESS_STOP_TIMEOUT_SEC)
        return False
    except subprocess.TimeoutExpired:
        proc.kill()
        try:
            proc.wait(timeout=PROCESS_STOP_TIMEOUT_SEC)
        except subprocess.TimeoutExpired:
            pass
        return True


def evaluate_final_verdict(
    ui_ok: bool,
    stable_before_report: Dict[str, object],
    stable_after_report: Dict[str, object],
    app_runtime_sec: int,
    wall_runtime_sec: float,
    recovery_ms: int,
    phase2_audio_dump: Path,
) -> Tuple[str, str]:
    if not ui_ok:
        return ("fail", "M3 UI 产物渲染失败")

    if not stable_before_report["ok"]:
        return (
            "fail",
            "M3 断线前长时刷新不达标："
            f"event_count={stable_before_report['event_count']}, "
            f"max_gap_ms={stable_before_report['max_gap_ms']}, "
            f"unexpected={stable_before_report['unexpected_statuses']}",
        )

    if not stable_after_report["ok"]:
        return (
            "fail",
            "M3 恢复后长时刷新不达标："
            f"event_count={stable_after_report['event_count']}, "
            f"max_gap_ms={stable_after_report['max_gap_ms']}, "
            f"unexpected={stable_after_report['unexpected_statuses']}",
        )

    if wall_runtime_sec < float(app_runtime_sec):
        return (
            "fail",
            "M3 实际运行时长不达标："
            f"{wall_runtime_sec:.1f}s（要求至少 {app_runtime_sec}s）",
        )

    if app_runtime_sec < MIN_PASS_RUNTIME_SEC:
        return (
            "fail",
            "M3 运行时长不达标："
            f"{app_runtime_sec}s（要求至少 {MIN_PASS_RUNTIME_SEC}s）",
        )

    if recovery_ms <= 0 or recovery_ms > RECOVERY_TARGET_MS:
        return ("fail", f"M3 恢复时长不达标：{recovery_ms} ms")

    if not phase2_audio_dump.exists() or phase2_audio_dump.stat().st_size <= 0:
        return ("fail", "M3 phase2 audio dump 为空，恢复后未形成音频流")

    return (
        "pass",
        "M3 Harness 完成：真实 netmic-ui 长测通过，"
        f"总运行 {wall_runtime_sec:.1f}s，断线后 {recovery_ms} ms 内恢复",
    )


def fetch_remote_artifacts(env: Dict[str, str], remote_dir: str, server_dir: Path) -> None:
    run_m0.ensure_dir(server_dir)
    runtime = run_m1.remote_fetch_text(env, f"{remote_dir}/runtime.log")
    run_m0.write_text(server_dir / "runtime.log", runtime.stdout or "")

    audio_dump_path = server_dir / "audio_dump.pcm"
    audio_dump = run_m1.remote_fetch_binary_base64(env, f"{remote_dir}/audio_dump.pcm")
    if audio_dump.returncode == 0 and (audio_dump.stdout or "").strip():
        audio_dump_path.write_bytes(base64.b64decode(audio_dump.stdout.encode("ascii")))
    else:
        audio_dump_path.write_bytes(b"")


def build_manifest(
    env: Dict[str, str],
    hosts_env: Path,
    run_id: str,
    run_dir: Path,
    app_runtime_sec: int,
    disconnect_after_sec: int,
) -> Dict[str, object]:
    return {
        "run_id": run_id,
        "milestone": "M3",
        "work_package": "[harness] real netmic-ui long run + reconnect recovery",
        "topology": "macOS netmic-ui + Linux netmic-server",
        "hosts_env": run_m0.display_path(hosts_env),
        "artifact_dir": run_m0.display_path(run_dir),
        "runtime": {
            "app_runtime_sec": app_runtime_sec,
            "min_pass_runtime_sec": MIN_PASS_RUNTIME_SEC,
            "disconnect_after_sec": disconnect_after_sec,
            "post_recover_sec": max(0, app_runtime_sec - disconnect_after_sec),
            "stream_timeout_sec": STREAM_TIMEOUT_SEC,
            "reconnect_timeout_sec": RECONNECT_TIMEOUT_SEC,
            "recover_timeout_sec": RECOVER_TIMEOUT_SEC,
            "recovery_target_ms": RECOVERY_TARGET_MS,
            "refresh_gap_target_ms": REFRESH_GAP_TARGET_MS,
        },
        "server": {
            "host": env.get("NETMIC_HARNESS_LINUX_HOST", ""),
            "port": env.get("NETMIC_HARNESS_SERVER_PORT", "43000"),
            "root": env.get("NETMIC_HARNESS_LINUX_ROOT", ""),
        },
        "client": {
            "root": env.get("NETMIC_HARNESS_MAC_ROOT", str(ROOT)),
            "input_device": env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", ""),
            "app": "netmic-ui",
        },
        "repo": run_m0.collect_repo_state(),
        "started_at": now_iso(),
    }


def select_snapshot(event: Dict[str, object]) -> Dict[str, object]:
    return dict(event.get("snapshot") or {})


def render_phase_snapshot(snapshot: Dict[str, object], ui_dir: Path, phase: str) -> run_m0.StepResult:
    phase_dir = ui_dir / phase
    return run_m0.run_ui_verify(
        snapshot,
        phase_dir,
        step_id=f"ui-verify-{phase}",
        success_summary=f"M3 {phase} 阶段 UI 可见产物已生成",
        failure_summary=f"M3 {phase} 阶段 UI 产物生成失败",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Run NetMic M3 harness")
    parser.add_argument("--hosts-env", default=str(DEFAULT_HOSTS_ENV))
    parser.add_argument("--run-id", default="")
    parser.add_argument("--app-runtime-sec", type=int, default=APP_RUNTIME_SEC)
    parser.add_argument("--disconnect-after-sec", type=int, default=DEFAULT_DISCONNECT_AFTER_SEC)
    args = parser.parse_args()

    hosts_env = Path(args.hosts_env).expanduser().resolve()
    run_id = args.run_id or datetime.now().astimezone().strftime("m3-%Y%m%dT%H%M%S")
    app_runtime_sec = max(30, int(args.app_runtime_sec))
    disconnect_after_sec = max(5, int(args.disconnect_after_sec))
    disconnect_after_sec = min(disconnect_after_sec, max(5, app_runtime_sec - 5))
    env = run_m0.parse_env_file(hosts_env)
    artifact_root_raw = env.get("NETMIC_HARNESS_ARTIFACT_DIR", ".harness/runs")
    artifact_root = Path(artifact_root_raw) if os.path.isabs(artifact_root_raw) else ROOT / artifact_root_raw
    run_dir = artifact_root / run_id
    client_dir = run_dir / "client"
    server_dir = run_dir / "server"
    ui_dir = run_dir / "ui"
    for path in (client_dir, server_dir, ui_dir):
        run_m0.ensure_dir(path)

    manifest = build_manifest(
        env,
        hosts_env,
        run_id,
        run_dir,
        app_runtime_sec,
        disconnect_after_sec,
    )
    run_m0.write_json(run_dir / "manifest.json", manifest)

    steps: List[run_m0.StepResult] = []
    missing_summary = run_m1.require_keys(env)
    if missing_summary:
        steps.append(run_m0.StepResult("prepare", "blocked", missing_summary))
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

    port = int(env.get("NETMIC_HARNESS_SERVER_PORT", "43000") or "43000")
    remote_phase1 = f"{env['NETMIC_HARNESS_LINUX_ROOT']}/.harness/runs/{run_id}/server-phase1"
    remote_phase2 = f"{env['NETMIC_HARNESS_LINUX_ROOT']}/.harness/runs/{run_id}/server-phase2"
    bootstrap_log = server_dir / "bootstrap.log"

    server_start = run_m1.remote_start_server(env, remote_phase1, port)
    run_m0.append_section(bootstrap_log, "remote-start-server-phase1", server_start.stdout, server_start.stderr)
    if server_start.returncode != 0:
        output = "\n".join(part for part in (server_start.stdout, server_start.stderr) if part).strip()
        verdict = run_m0.classify_output(output)
        summary = run_m0.build_command_failure_summary("远端服务端 phase1 启动失败", output)
        steps.append(run_m0.StepResult("bootstrap-linux", verdict, summary, [run_m0.relative_artifact(bootstrap_log)]))
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

    startup_status = run_m1.remote_status(env, port, f"{run_id}-startup")
    if startup_status.returncode != 0:
        time.sleep(2)
        startup_status = run_m1.remote_status(env, port, f"{run_id}-startup")
    status_json_path = server_dir / "status.json"
    status_payload = json.loads(startup_status.stdout) if startup_status.returncode == 0 and (startup_status.stdout or "").strip() else {}
    run_m0.write_json(status_json_path, {"phase1_startup": status_payload})
    if not status_payload:
        run_m1.remote_stop_server(env, remote_phase1)
        summary = "服务端 phase1 未进入可查询状态"
        steps.append(run_m0.StepResult("bootstrap-linux", "fail", summary, [run_m0.relative_artifact(status_json_path)]))
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

    steps.append(
        run_m0.StepResult(
            "bootstrap-linux",
            "pass",
            "远端服务端 phase1 已启动",
            [run_m0.relative_artifact(bootstrap_log), run_m0.relative_artifact(status_json_path)],
        )
    )

    snapshot_path = ui_dir / "live-snapshot.json"
    event_log_path = ui_dir / "event-log.ndjson"
    render_log_path = ui_dir / "render-log.ndjson"
    client_runtime_log = client_dir / "runtime.log"
    client_env = os.environ.copy()
    client_env["RUST_LOG"] = "info"
    client_env["NETMIC_UI_HARNESS_AUTOSTART"] = "1"
    client_env["NETMIC_UI_HARNESS_SERVER_ADDR"] = env["NETMIC_HARNESS_SERVER_HOST"]
    client_env["NETMIC_UI_HARNESS_SERVER_PORT"] = str(port)
    client_env["NETMIC_UI_HARNESS_SNAPSHOT_PATH"] = str(snapshot_path)
    client_env["NETMIC_UI_HARNESS_EVENT_LOG"] = str(event_log_path)
    client_env["NETMIC_UI_HARNESS_RENDER_LOG"] = str(render_log_path)
    if env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", "").strip():
        client_env["NETMIC_UI_HARNESS_INPUT_DEVICE"] = env["NETMIC_HARNESS_CLIENT_INPUT_DEVICE"].strip()

    app_wall_start = time.monotonic()
    wall_runtime_sec = 0.0
    phase2_started = False
    with client_runtime_log.open("w", encoding="utf-8") as handle:
        proc = subprocess.Popen(
            ["cargo", "run", "-p", "netmic-ui", "--quiet"],
            cwd=str(ROOT),
            env=client_env,
            stdout=handle,
            stderr=subprocess.STDOUT,
            text=True,
        )

        stream_index, stream_event = wait_for_event(
            render_log_path,
            0,
            STREAM_TIMEOUT_SEC,
            lambda event: (event.get("snapshot") or {}).get("status") == "streaming",
        )
        if stream_event is None:
            stop_ui_process(proc)
            run_m1.remote_stop_server(env, remote_phase1)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            summary = "真实 netmic-ui 未在窗口内完成前端 streaming 渲染"
            steps.append(run_m0.StepResult("bootstrap-ui", "fail", summary, [run_m0.relative_artifact(client_runtime_log), run_m0.relative_artifact(event_log_path), run_m0.relative_artifact(render_log_path)]))
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

        steps.append(
            run_m0.StepResult(
                "bootstrap-ui",
                "pass",
                "真实 netmic-ui 已进入 streaming",
                [
                    run_m0.relative_artifact(client_runtime_log),
                    run_m0.relative_artifact(snapshot_path),
                    run_m0.relative_artifact(event_log_path),
                    run_m0.relative_artifact(render_log_path),
                ],
            )
        )

        stable_before_index, stable_before_event = wait_for_event_span(
            render_log_path,
            stream_index or 0,
            event_ts_ms(stream_event),
            disconnect_after_sec,
            "streaming",
        )
        if stable_before_event is None:
            stop_ui_process(proc)
            run_m1.remote_stop_server(env, remote_phase1)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            summary = f"真实 netmic-ui 未达到断线前稳定运行窗口：{disconnect_after_sec}s"
            steps.append(
                run_m0.StepResult(
                    "steady-before-disconnect",
                    "fail",
                    summary,
                    [
                        run_m0.relative_artifact(client_runtime_log),
                        run_m0.relative_artifact(event_log_path),
                        run_m0.relative_artifact(render_log_path),
                    ],
                )
            )
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

        steps.append(
            run_m0.StepResult(
                "steady-before-disconnect",
                "pass",
                f"断线前已稳定运行 {disconnect_after_sec}s",
                [
                    run_m0.relative_artifact(client_runtime_log),
                    run_m0.relative_artifact(event_log_path),
                    run_m0.relative_artifact(render_log_path),
                ],
            )
        )

        run_m1.remote_stop_server(env, remote_phase1)
        reconnect_index, reconnect_event = wait_for_event(
            render_log_path,
            (stable_before_index or stream_index or 0) + 1,
            RECONNECT_TIMEOUT_SEC,
            lambda event: (event.get("snapshot") or {}).get("status") == "connecting"
            and ((event.get("snapshot") or {}).get("runtime") or {}).get("reconnect_attempts", 0) >= 1,
        )
        if reconnect_event is None:
            stop_ui_process(proc)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            summary = "真实 netmic-ui 未在断线后渲染 reconnecting/connecting"
            steps.append(run_m0.StepResult("disconnect-recover", "fail", summary, [run_m0.relative_artifact(event_log_path), run_m0.relative_artifact(render_log_path)]))
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

        server_restart = run_m1.remote_start_server(env, remote_phase2, port)
        run_m0.append_section(bootstrap_log, "remote-start-server-phase2", server_restart.stdout, server_restart.stderr)
        if server_restart.returncode != 0:
            stop_ui_process(proc)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            output = "\n".join(part for part in (server_restart.stdout, server_restart.stderr) if part).strip()
            verdict = run_m0.classify_output(output)
            summary = run_m0.build_command_failure_summary("远端服务端 phase2 重启失败", output)
            steps.append(run_m0.StepResult("disconnect-recover", verdict, summary, [run_m0.relative_artifact(bootstrap_log)]))
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
        phase2_started = True

        recovered_index, recovered_event = wait_for_event(
            render_log_path,
            (reconnect_index or 0) + 1,
            RECOVER_TIMEOUT_SEC,
            lambda event: (event.get("snapshot") or {}).get("status") == "streaming",
        )
        if recovered_event is None:
            stop_ui_process(proc)
            run_m1.remote_stop_server(env, remote_phase2)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            fetch_remote_artifacts(env, remote_phase2, server_dir / "phase2")
            summary = "真实 netmic-ui 未在恢复窗口内重新渲染 streaming"
            steps.append(run_m0.StepResult("disconnect-recover", "fail", summary, [run_m0.relative_artifact(event_log_path), run_m0.relative_artifact(render_log_path)]))
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

        post_recover_sec = max(0, app_runtime_sec - disconnect_after_sec)
        steady_after_index, steady_after_event = wait_for_event_span(
            render_log_path,
            recovered_index or 0,
            event_ts_ms(recovered_event),
            post_recover_sec,
            "streaming",
        )
        if steady_after_event is None:
            stop_ui_process(proc)
            run_m1.remote_stop_server(env, remote_phase2)
            fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
            fetch_remote_artifacts(env, remote_phase2, server_dir / "phase2")
            summary = f"真实 netmic-ui 未达到恢复后稳定运行窗口：{post_recover_sec}s"
            steps.append(run_m0.StepResult("steady-after-recover", "fail", summary, [run_m0.relative_artifact(event_log_path), run_m0.relative_artifact(render_log_path)]))
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

        steps.append(
            run_m0.StepResult(
                "steady-after-recover",
                "pass",
                f"恢复后已稳定运行 {post_recover_sec}s",
                [run_m0.relative_artifact(event_log_path), run_m0.relative_artifact(render_log_path)],
            )
        )

        time.sleep(3)
        stop_ui_process(proc)
        wall_runtime_sec = time.monotonic() - app_wall_start

    fetch_remote_artifacts(env, remote_phase1, server_dir / "phase1")
    if phase2_started:
        fetch_remote_artifacts(env, remote_phase2, server_dir / "phase2")
        run_m1.remote_stop_server(env, remote_phase2)

    backend_events = load_events(event_log_path)
    events = load_events(render_log_path)
    before_snapshot = select_snapshot(stream_event or {})
    reconnect_snapshot = select_snapshot(reconnect_event or {})
    recovered_snapshot = select_snapshot(recovered_event or {})
    steady_snapshot = select_snapshot(steady_after_event or recovered_event or {})
    recovery_ms = int((recovered_event or {}).get("ts_ms") or 0) - int((reconnect_event or {}).get("ts_ms") or 0)
    stable_before_report = analyze_refresh_window(
        events,
        stream_index or 0,
        stable_before_index or stream_index or 0,
        "streaming",
    )
    stable_after_report = analyze_refresh_window(
        events,
        recovered_index or 0,
        steady_after_index or recovered_index or 0,
        "streaming",
    )

    ui_steps = [
        render_phase_snapshot(before_snapshot, ui_dir, "before"),
        render_phase_snapshot(reconnect_snapshot, ui_dir, "reconnecting"),
        render_phase_snapshot(recovered_snapshot, ui_dir, "recovered"),
        render_phase_snapshot(steady_snapshot, ui_dir, "steady"),
    ]
    for step in ui_steps:
        steps.append(step)
    final_status, summary = evaluate_final_verdict(
        ui_ok=all(step.status == "pass" for step in ui_steps),
        stable_before_report=stable_before_report,
        stable_after_report=stable_after_report,
        app_runtime_sec=app_runtime_sec,
        wall_runtime_sec=wall_runtime_sec,
        recovery_ms=recovery_ms,
        phase2_audio_dump=server_dir / "phase2" / "audio_dump.pcm",
    )

    steps.append(
        run_m0.StepResult(
            "disconnect-recover",
            final_status,
            summary,
            [
                run_m0.relative_artifact(event_log_path),
                run_m0.relative_artifact(render_log_path),
                run_m0.relative_artifact(snapshot_path),
                run_m0.relative_artifact(server_dir / "phase1" / "audio_dump.pcm"),
                run_m0.relative_artifact(server_dir / "phase2" / "audio_dump.pcm"),
            ],
        )
    )

    run_m0.write_json(
        run_dir / "recovery.json",
        {
            "streaming_event": stream_event,
            "steady_before_event": stable_before_event,
            "reconnecting_event": reconnect_event,
            "recovered_event": recovered_event,
            "steady_after_event": steady_after_event,
            "recovery_ms": recovery_ms,
            "event_count": len(backend_events),
            "render_event_count": len(events),
            "app_runtime_sec": app_runtime_sec,
            "disconnect_after_sec": disconnect_after_sec,
            "post_recover_sec": post_recover_sec,
            "wall_runtime_sec": wall_runtime_sec,
            "refresh_gap_target_ms": REFRESH_GAP_TARGET_MS,
            "stable_before": stable_before_report,
            "stable_after": stable_after_report,
        },
    )
    if (ui_dir / "recovered").exists():
        if (ui_dir / "snapshot.json").exists():
            (ui_dir / "snapshot.json").unlink()
        shutil.copy2(ui_dir / "recovered" / "snapshot.json", ui_dir / "snapshot.json")

    run_m0.write_json(
        run_dir / "report.json",
        {
            "run_id": run_id,
            "status": final_status,
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [run_m0.asdict(step) for step in steps],
            "recovery_ms": recovery_ms,
            "wall_runtime_sec": wall_runtime_sec,
        },
    )
    print(f"{final_status}: {summary}")
    if final_status == "pass":
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
