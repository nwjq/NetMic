#!/usr/bin/env python3
"""NetMic M0 Harness coordinator.

目标：
- 读取 `.harness/hosts.env`
- 通过 SSH 触发 Linux 侧 M0 自检 / 虚拟麦创建 / smoke
- 统一落盘 `.harness/runs/<run_id>/`
- 输出 `pass` / `fail` / `blocked`
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Tuple


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HOSTS_ENV = ROOT / ".harness" / "hosts.env"
DEFAULT_DURATION_SEC = 300
DEFAULT_SERVER_PORT = 43000
DEFAULT_ARTIFACT_DIR = ROOT / ".harness" / "runs"
UI_POLL_INTERVAL_MS = 1000
DEFAULT_SYNC_EXCLUDES = (
    ".git/",
    ".harness/hosts.env",
    ".harness/runs/",
    ".codex/",
    "target/",
    "node_modules/",
)

BLOCKED_PATTERNS = (
    "permission denied",
    "operation not permitted",
    "host key verification failed",
    "connection refused",
    "connection timed out",
    "no route to host",
    "could not resolve hostname",
    "network is unreachable",
    "缺少命令",
    "command not found",
    "可能未运行",
    "not installed",
    "sshpass",
    "timed out",
)


@dataclass
class StepResult:
    id: str
    status: str
    summary: str
    artifacts: List[str] = field(default_factory=list)


@dataclass
class CommandResult:
    command: List[str]
    returncode: int
    stdout: str
    stderr: str


def now_iso() -> str:
    return datetime.now().astimezone().isoformat(timespec="seconds")


def now_ms() -> int:
    return int(datetime.now().timestamp() * 1000)


def parse_env_file(path: Path) -> Dict[str, str]:
    env: Dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        env[key.strip()] = value.strip().strip('"').strip("'")
    return env


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def write_json(path: Path, payload: object) -> None:
    ensure_dir(path.parent)
    path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def write_text(path: Path, text: str) -> None:
    ensure_dir(path.parent)
    path.write_text(text, encoding="utf-8")


def append_section(path: Path, title: str, stdout: str, stderr: str) -> None:
    ensure_dir(path.parent)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(f"== {title} ==\n")
        if stdout:
            handle.write("[stdout]\n")
            handle.write(stdout.rstrip() + "\n")
        if stderr:
            handle.write("[stderr]\n")
            handle.write(stderr.rstrip() + "\n")
        handle.write("\n")


def classify_output(output: str) -> str:
    lower = output.lower()
    for pattern in BLOCKED_PATTERNS:
        if pattern in lower:
            return "blocked"
    return "fail"


def summarize_command_issue(output: str) -> str:
    for raw_line in output.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("rsync ") or line.startswith("sending incremental file list"):
            continue
        return line
    return ""


def require_local_tools(password_auth: bool, needs_node: bool, needs_rsync: bool = False) -> Tuple[str, str]:
    missing: List[str] = []
    if shutil.which("ssh") is None:
        missing.append("ssh")
    if password_auth and shutil.which("sshpass") is None:
        missing.append("sshpass")
    if needs_rsync and shutil.which("rsync") is None:
        missing.append("rsync")
    if needs_node and shutil.which("node") is None:
        missing.append("node")
    if missing:
        return (
            "blocked",
            "缺少本地依赖：" + ", ".join(missing),
        )
    return ("pass", "本地依赖检查通过")


def build_ssh_transport(env: Dict[str, str]) -> List[str]:
    port = env.get("NETMIC_HARNESS_LINUX_PORT", "22") or "22"
    ssh_key = env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")
    password = env.get("NETMIC_HARNESS_LINUX_PASSWORD", "")
    ssh_opts = env.get("NETMIC_HARNESS_SSH_OPTS", "")

    command: List[str] = ["ssh", "-p", port]
    if password and not ssh_key:
        command.extend(
            [
                "-o",
                "PreferredAuthentications=password",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "NumberOfPasswordPrompts=1",
            ]
        )
    if ssh_key:
        command.extend(["-i", ssh_key])
    if ssh_opts:
        command.extend(shlex.split(ssh_opts))
    return command


def build_ssh_base(env: Dict[str, str]) -> List[str]:
    user = env["NETMIC_HARNESS_LINUX_USER"]
    host = env["NETMIC_HARNESS_LINUX_HOST"]
    ssh_key = env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")
    password = env.get("NETMIC_HARNESS_LINUX_PASSWORD", "")

    command: List[str] = []
    if password and not ssh_key:
        command.extend(["sshpass", "-p", password])
    command.extend(build_ssh_transport(env))
    command.append(f"{user}@{host}")
    return command


def coordinator_root_from_env(env: Dict[str, str]) -> Path:
    raw = (
        env.get("NETMIC_HARNESS_COORDINATOR_ROOT", "")
        or env.get("NETMIC_HARNESS_MAC_ROOT", "")
        or str(ROOT)
    )
    path = Path(raw).expanduser()
    if not path.is_absolute():
        path = ROOT / path
    return path.resolve()


def build_rsync_command(env: Dict[str, str], local_root: Path, dry_run: bool) -> List[str]:
    user = env["NETMIC_HARNESS_LINUX_USER"]
    host = env["NETMIC_HARNESS_LINUX_HOST"]
    remote_root = env["NETMIC_HARNESS_LINUX_ROOT"].rstrip("/")
    ssh_key = env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")
    password = env.get("NETMIC_HARNESS_LINUX_PASSWORD", "")

    command: List[str] = []
    if password and not ssh_key:
        command.extend(["sshpass", "-p", password])
    command.extend(["rsync", "-az", "--delete", "--itemize-changes"])
    if dry_run:
        command.append("--dry-run")
    for pattern in DEFAULT_SYNC_EXCLUDES:
        command.extend(["--exclude", pattern])
    ssh_transport = " ".join(shlex.quote(part) for part in build_ssh_transport(env))
    command.extend(
        [
            "-e",
            ssh_transport,
            f"{str(local_root).rstrip('/')}/",
            f"{user}@{host}:{remote_root}/",
        ]
    )
    return command


def sync_remote_workspace(env: Dict[str, str]) -> Tuple[str, str, List[CommandResult]]:
    local_root = coordinator_root_from_env(env)
    if not local_root.exists():
        return ("blocked", f"本地 Coordinator 根目录不存在：{local_root}", [])

    dry_run_result = run_command(build_rsync_command(env, local_root, dry_run=True))
    combined = "\n".join(part for part in (dry_run_result.stdout, dry_run_result.stderr) if part).strip()
    if dry_run_result.returncode != 0:
        detail = summarize_command_issue(combined)
        summary = "远端工作区同步预检失败"
        if detail:
            summary = f"{summary}：{detail}"
        return (classify_output(combined), summary, [dry_run_result])
    if not combined:
        return ("pass", "远端工作区已与本地同步", [dry_run_result])

    apply_result = run_command(build_rsync_command(env, local_root, dry_run=False))
    combined = "\n".join(part for part in (apply_result.stdout, apply_result.stderr) if part).strip()
    if apply_result.returncode != 0:
        detail = summarize_command_issue(combined)
        summary = "远端工作区同步失败"
        if detail:
            summary = f"{summary}：{detail}"
        return (
            classify_output(combined),
            summary,
            [dry_run_result, apply_result],
        )
    return (
        "pass",
        "已将本地工作区同步到远端",
        [dry_run_result, apply_result],
    )


def run_remote_sync_step(env: Dict[str, str], server_dir: Path) -> StepResult:
    log_path = server_dir / "remote-sync.log"
    state_path = server_dir / "remote-sync.json"
    status, summary, results = sync_remote_workspace(env)
    titles = ["rsync-dry-run", "rsync-apply"]
    for index, result in enumerate(results):
        title = titles[index] if index < len(titles) else f"rsync-step-{index + 1}"
        append_section(log_path, title, result.stdout, result.stderr)
    write_json(
        state_path,
        {
            "status": status,
            "summary": summary,
            "commands": [
                {
                    "command": result.command,
                    "returncode": result.returncode,
                }
                for result in results
            ],
            "updated_at": now_iso(),
        },
    )
    return StepResult(
        "sync-remote",
        status,
        summary,
        [relative_artifact(log_path), relative_artifact(state_path)],
    )


def run_command(command: List[str]) -> CommandResult:
    proc = subprocess.run(
        command,
        cwd=str(ROOT),
        text=True,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    return CommandResult(
        command=command,
        returncode=proc.returncode,
        stdout=proc.stdout,
        stderr=proc.stderr,
    )


def run_remote(env: Dict[str, str], script: str) -> CommandResult:
    linux_root = env["NETMIC_HARNESS_LINUX_ROOT"]
    ssh_base = build_ssh_base(env)
    remote_script = f"cd {shlex.quote(linux_root)} && {script}"
    command = ssh_base + [f"bash -lc {shlex.quote(remote_script)}"]
    return run_command(command)


def build_snapshot(
    env: Dict[str, str],
    status_json: Dict[str, object],
    verdict: str,
    detail: str,
    logs: List[Dict[str, object]],
) -> Dict[str, object]:
    ready = bool(status_json.get("ready"))
    source_name = str(status_json.get("source_name") or "未设置")
    port = int(env.get("NETMIC_HARNESS_SERVER_PORT") or DEFAULT_SERVER_PORT)
    snapshot_status = "listening" if ready and verdict == "pass" else "error"
    status_note = "等待客户端连接" if snapshot_status == "listening" else detail
    last_error = None if snapshot_status == "listening" else detail
    virtual_mic_error = None if ready else detail

    return {
        "mode": "server",
        "status": snapshot_status,
        "status_note": status_note,
        "client_config": {
            "server_addr": env.get("NETMIC_HARNESS_SERVER_HOST", ""),
            "server_port": port,
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
            "listen_port": port,
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
            "buffer_depth_ms": 0.0,
            "jitter_buffer_depth_ms": 0.0,
            "estimated_e2e_latency_ms": 0.0,
            "audio_rms": 0.0,
            "audio_peak": 0,
            "uplink_kbps": 0.0,
        },
        "runtime": {
            "peer_addr": None,
            "connected_seconds": 0,
            "reconnect_attempts": 0,
            "mic_permission": "不适用",
            "virtual_mic_name": source_name,
            "virtual_mic_ready": ready,
            "virtual_mic_error": virtual_mic_error,
            "server_status_updated_ms": now_ms(),
            "last_error": last_error,
        },
        "devices": {
            "input": [],
        },
        "logs": logs,
    }


def step_log(level: str, message: str) -> Dict[str, object]:
    return {
        "ts_ms": now_ms(),
        "level": level,
        "message": message,
    }


def summarize_runtime(result: CommandResult) -> Tuple[str, str]:
    combined = "\n".join(part for part in (result.stdout, result.stderr) if part).strip()
    lowered = combined.lower()
    if result.returncode == 0 and "跳过音频写入" not in combined and "缺少 paplay" not in combined:
        return ("pass", "虚拟麦 smoke 完成，已执行测试音写入")
    if "跳过音频写入" in combined or "缺少 paplay" in combined:
        return ("blocked", "虚拟麦 smoke 未实际写入测试音：缺少 paplay 或生成链路不可用")
    if result.returncode == 0:
        return ("pass", "虚拟麦 smoke 完成")
    return (classify_output(lowered), "虚拟麦 smoke 失败")


def parse_json_or_empty(path: Path) -> Dict[str, object]:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def prepare_manifest(
    env: Dict[str, str],
    hosts_env: Path,
    run_id: str,
    run_dir: Path,
    duration_sec: int,
) -> Dict[str, object]:
    return {
        "run_id": run_id,
        "milestone": "M0",
        "work_package": "[harness] Linux M0 bootstrap + smoke + UI artifacts",
        "topology": "macOS Coordinator -> Linux Server",
        "hosts_env": display_path(hosts_env),
        "artifact_dir": display_path(run_dir),
        "server": {
            "host": env.get("NETMIC_HARNESS_LINUX_HOST", ""),
            "port": env.get("NETMIC_HARNESS_LINUX_PORT", "22"),
            "user": env.get("NETMIC_HARNESS_LINUX_USER", ""),
            "root": env.get("NETMIC_HARNESS_LINUX_ROOT", ""),
        },
        "client": {
            "server_host": env.get("NETMIC_HARNESS_SERVER_HOST", ""),
            "server_port": env.get("NETMIC_HARNESS_SERVER_PORT", str(DEFAULT_SERVER_PORT)),
            "input_device_configured": bool(env.get("NETMIC_HARNESS_CLIENT_INPUT_DEVICE", "")),
        },
        "runtime": {
            "duration_sec": duration_sec,
            "server_status_poll_ms": UI_POLL_INTERVAL_MS,
        },
        "started_at": now_iso(),
    }


def relative_artifact(path: Path) -> str:
    return display_path(path)


def run_ui_verify(
    snapshot: Dict[str, object],
    ui_dir: Path,
) -> StepResult:
    snapshot_path = ui_dir / "snapshot.json"
    write_json(snapshot_path, snapshot)
    render_log = ui_dir / "render.log"
    render_result = run_command(
        [
            "node",
            str(ROOT / "scripts" / "harness" / "render_ui_artifacts.mjs"),
            "--snapshot",
            str(snapshot_path),
            "--out-dir",
            str(ui_dir),
        ]
    )
    append_section(render_log, "render-ui-artifacts", render_result.stdout, render_result.stderr)
    ui_status = (
        "pass"
        if render_result.returncode == 0
        else classify_output(render_result.stderr or render_result.stdout)
    )
    ui_summary = (
        "M0 UI 产物已生成，虚拟麦状态可见"
        if ui_status == "pass"
        else "UI 产物生成失败"
    )
    return StepResult(
        "ui-verify",
        ui_status,
        ui_summary,
        [
            relative_artifact(snapshot_path),
            relative_artifact(ui_dir / "visible-status.json"),
            relative_artifact(ui_dir / "visible-config.json"),
            relative_artifact(ui_dir / "visible-logs.json"),
            relative_artifact(ui_dir / "refresh-check.json"),
            relative_artifact(render_log),
        ],
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Run NetMic M0 harness")
    parser.add_argument("--hosts-env", default=str(DEFAULT_HOSTS_ENV))
    parser.add_argument("--run-id", default="")
    parser.add_argument("--duration", type=int, default=DEFAULT_DURATION_SEC)
    args = parser.parse_args()

    hosts_env = Path(args.hosts_env).expanduser().resolve()
    run_id = args.run_id or datetime.now().astimezone().strftime("m0-%Y%m%dT%H%M%S")
    duration_sec = max(1, int(args.duration))

    if not hosts_env.exists():
        print(f"blocked: hosts env not found: {hosts_env}", file=sys.stderr)
        return 2

    env = parse_env_file(hosts_env)
    artifact_root_raw = env.get("NETMIC_HARNESS_ARTIFACT_DIR", ".harness/runs")
    artifact_root = (
        Path(artifact_root_raw).expanduser()
        if os.path.isabs(artifact_root_raw)
        else ROOT / artifact_root_raw
    )
    run_dir = artifact_root / run_id
    server_dir = run_dir / "server"
    ui_dir = run_dir / "ui"
    client_dir = run_dir / "client"
    ensure_dir(server_dir)
    ensure_dir(ui_dir)
    ensure_dir(client_dir)

    steps: List[StepResult] = []
    logs: List[Dict[str, object]] = []
    manifest = prepare_manifest(env, hosts_env, run_id, run_dir, duration_sec)
    write_json(run_dir / "manifest.json", manifest)

    required_keys = [
        "NETMIC_HARNESS_LINUX_HOST",
        "NETMIC_HARNESS_LINUX_USER",
        "NETMIC_HARNESS_LINUX_ROOT",
        "NETMIC_HARNESS_SERVER_HOST",
    ]
    missing_keys = [key for key in required_keys if not env.get(key, "").strip()]
    if missing_keys:
        summary = "缺少现场配置：" + ", ".join(missing_keys)
        steps.append(StepResult("prepare", "blocked", summary))
        logs.append(step_log("error", summary))
        report = {
            "run_id": run_id,
            "status": "blocked",
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(summary)
        return 2

    prepare_status, prepare_summary = require_local_tools(
        password_auth=bool(env.get("NETMIC_HARNESS_LINUX_PASSWORD", "") and not env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")),
        needs_node=True,
        needs_rsync=True,
    )
    prepare_artifacts = [relative_artifact(run_dir / "manifest.json")]
    steps.append(StepResult("prepare", prepare_status, prepare_summary, prepare_artifacts))
    logs.append(step_log("info" if prepare_status == "pass" else "error", prepare_summary))
    if prepare_status != "pass":
        report = {
            "run_id": run_id,
            "status": prepare_status,
            "summary": prepare_summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(prepare_summary)
        return 2

    sync_step = run_remote_sync_step(env, server_dir)
    steps.append(sync_step)
    logs.append(step_log("info" if sync_step.status == "pass" else "error", sync_step.summary))
    if sync_step.status != "pass":
        summary = sync_step.summary
        status = sync_step.status
        ui_step = run_ui_verify(build_snapshot(env, {}, status, summary, logs), ui_dir)
        steps.append(ui_step)
        logs.append(step_log("info" if ui_step.status == "pass" else "error", ui_step.summary))
        report = {
            "run_id": run_id,
            "status": status,
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(summary)
        return 2 if status == "blocked" else 1

    bootstrap_log = server_dir / "bootstrap.log"
    runtime_log = server_dir / "runtime.log"

    probe = run_remote(env, "pwd")
    append_section(bootstrap_log, "prepare-probe", probe.stdout, probe.stderr)
    if probe.returncode != 0:
        status = classify_output("\n".join([probe.stdout, probe.stderr]))
        summary = "无法连接 Linux 主机或进入远端仓库目录"
        steps.append(
            StepResult(
                "bootstrap-linux",
                status,
                summary,
                [relative_artifact(bootstrap_log)],
            )
        )
        logs.append(step_log("error", summary))
        ui_step = run_ui_verify(build_snapshot(env, {}, status, summary, logs), ui_dir)
        steps.append(ui_step)
        logs.append(step_log("info" if ui_step.status == "pass" else "error", ui_step.summary))
        if status == "pass" and ui_step.status != "pass":
            status = ui_step.status
            summary = ui_step.summary
        report = {
            "run_id": run_id,
            "status": status,
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(summary)
        return 2 if status == "blocked" else 1

    selfcheck = run_remote(env, "scripts/linux/audio_selfcheck.sh --json")
    append_section(bootstrap_log, "audio-selfcheck", selfcheck.stdout, selfcheck.stderr)
    selfcheck_json_path = server_dir / "selfcheck.json"
    write_text(selfcheck_json_path, selfcheck.stdout or "{}\n")
    selfcheck_json = parse_json_or_empty(selfcheck_json_path)
    if selfcheck.returncode != 0:
        summary = str(selfcheck_json.get("reason") or "Linux 音频自检失败")
        status = classify_output("\n".join([selfcheck.stdout, selfcheck.stderr, summary]))
        steps.append(
            StepResult(
                "bootstrap-linux",
                status,
                summary,
                [relative_artifact(bootstrap_log), relative_artifact(selfcheck_json_path)],
            )
        )
        logs.append(step_log("error", summary))
        ui_step = run_ui_verify(build_snapshot(env, {}, status, summary, logs), ui_dir)
        steps.append(ui_step)
        logs.append(step_log("info" if ui_step.status == "pass" else "error", ui_step.summary))
        if status == "pass" and ui_step.status != "pass":
            status = ui_step.status
            summary = ui_step.summary
        report = {
            "run_id": run_id,
            "status": status,
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(summary)
        return 2 if status == "blocked" else 1

    create = run_remote(env, "scripts/linux/virtual_mic.sh create")
    append_section(bootstrap_log, "virtual-mic-create", create.stdout, create.stderr)
    status_json_path = server_dir / "status.json"
    status_result = run_remote(env, "scripts/linux/virtual_mic.sh status --json")
    append_section(bootstrap_log, "virtual-mic-status", status_result.stdout, status_result.stderr)
    write_text(status_json_path, status_result.stdout or "{}\n")
    status_json = parse_json_or_empty(status_json_path)
    if create.returncode != 0 or status_result.returncode != 0 or not status_json.get("ready"):
        summary = "虚拟麦克风创建后未就绪"
        status = classify_output(
            "\n".join([create.stdout, create.stderr, status_result.stdout, status_result.stderr, summary])
        )
        steps.append(
            StepResult(
                "bootstrap-linux",
                status,
                summary,
                [
                    relative_artifact(bootstrap_log),
                    relative_artifact(selfcheck_json_path),
                    relative_artifact(status_json_path),
                ],
            )
        )
        logs.append(step_log("error", summary))
        ui_step = run_ui_verify(build_snapshot(env, status_json, status, summary, logs), ui_dir)
        steps.append(ui_step)
        logs.append(step_log("info" if ui_step.status == "pass" else "error", ui_step.summary))
        if status == "pass" and ui_step.status != "pass":
            status = ui_step.status
            summary = ui_step.summary
        report = {
            "run_id": run_id,
            "status": status,
            "summary": summary,
            "started_at": manifest["started_at"],
            "finished_at": now_iso(),
            "steps": [asdict(step) for step in steps],
        }
        write_json(run_dir / "report.json", report)
        print(summary)
        return 2 if status == "blocked" else 1

    bootstrap_summary = f"Linux 自检通过，虚拟麦已就绪：{status_json.get('source_name', '')}"
    steps.append(
        StepResult(
            "bootstrap-linux",
            "pass",
            bootstrap_summary,
            [
                relative_artifact(bootstrap_log),
                relative_artifact(selfcheck_json_path),
                relative_artifact(status_json_path),
            ],
        )
    )
    logs.append(step_log("info", bootstrap_summary))

    smoke = run_remote(
        env,
        f"scripts/linux/virtual_mic_smoke.sh --duration {duration_sec}",
    )
    append_section(runtime_log, "virtual-mic-smoke", smoke.stdout, smoke.stderr)
    run_status, run_summary = summarize_runtime(smoke)
    steps.append(
        StepResult(
            "run",
            run_status,
            run_summary,
            [relative_artifact(runtime_log)],
        )
    )
    logs.append(step_log("info" if run_status == "pass" else "error", run_summary))

    verdict = "pass"
    if any(step.status == "blocked" for step in steps):
        verdict = "blocked"
    elif any(step.status == "fail" for step in steps):
        verdict = "fail"

    detail = run_summary if verdict != "pass" else bootstrap_summary
    ui_step = run_ui_verify(build_snapshot(env, status_json, verdict, detail, logs), ui_dir)
    steps.append(ui_step)
    logs.append(step_log("info" if ui_step.status == "pass" else "error", ui_step.summary))

    if verdict == "pass" and ui_step.status != "pass":
        verdict = ui_step.status
        detail = ui_step.summary
    elif verdict == "pass":
        detail = "M0 Harness 完成：自检、虚拟麦创建、smoke、UI 产物全部通过"

    report = {
        "run_id": run_id,
        "status": verdict,
        "summary": detail,
        "started_at": manifest["started_at"],
        "finished_at": now_iso(),
        "steps": [asdict(step) for step in steps],
    }
    write_json(run_dir / "report.json", report)
    print(f"{verdict}: {detail}")
    if verdict == "pass":
        return 0
    if verdict == "blocked":
        return 2
    return 1


if __name__ == "__main__":
    sys.exit(main())
