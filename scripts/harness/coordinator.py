#!/usr/bin/env python3
"""NetMic 顶层 Harness coordinator。

职责：
- 读取 `.harness/hosts.env`
- 扫描已有 run 产物，判断每个里程碑是否已完成
- 从第一个未完成里程碑继续推进，直到目标里程碑或真实 `fail/blocked`
- runner 缺失视为 `fail`，不能假装项目“已经做完”
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Tuple

import run_m0


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_HOSTS_ENV = ROOT / ".harness" / "hosts.env"
DEFAULT_ARTIFACT_DIR = ROOT / ".harness" / "runs"
STATE_PATH = ROOT / ".harness" / "coordinator_state.json"
M3_MIN_APP_RUNTIME_SEC = 30 * 60
M3_MAX_RECOVERY_MS = 10_000


@dataclass(frozen=True)
class MilestoneSpec:
    id: str
    title: str
    runner: str
    default_args: Tuple[str, ...] = ()


WORKFLOW: List[MilestoneSpec] = [
    MilestoneSpec(
        id="M0",
        title="参数与注入验证",
        runner="scripts/harness/run_m0.py",
    ),
    MilestoneSpec(
        id="M1",
        title="端到端基本通路",
        runner="scripts/harness/run_m1.py",
    ),
    MilestoneSpec(
        id="M2",
        title="可调参数 + 回退机制",
        runner="scripts/harness/run_m2.py",
    ),
    MilestoneSpec(
        id="M3",
        title="稳定性与可用性",
        runner="scripts/harness/run_m3.py",
    ),
]


def now_iso() -> str:
    return datetime.now().astimezone().isoformat(timespec="seconds")


def display_path(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path.resolve())


def parse_env_file(path: Path) -> Dict[str, str]:
    env: Dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        env[key.strip()] = value.strip().strip('"').strip("'")
    return env


def artifact_root_from_env(env: Dict[str, str]) -> Path:
    raw = env.get("NETMIC_HARNESS_ARTIFACT_DIR", ".harness/runs")
    path = Path(raw).expanduser()
    if path.is_absolute():
        return path
    return ROOT / path


def load_run_reports(artifact_root: Path) -> List[Dict[str, object]]:
    reports: List[Dict[str, object]] = []
    if not artifact_root.exists():
        return reports
    for report_path in sorted(artifact_root.glob("*/report.json")):
        try:
            report = json.loads(report_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        manifest_path = report_path.with_name("manifest.json")
        manifest = {}
        if manifest_path.exists():
            try:
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                manifest = {}
        recovery_path = report_path.with_name("recovery.json")
        recovery = {}
        if recovery_path.exists():
            try:
                recovery = json.loads(recovery_path.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                recovery = {}
        report["_report_path"] = str(report_path)
        report["_manifest"] = manifest
        report["_recovery"] = recovery
        reports.append(report)
    return reports


def report_counts_as_pass(report: Dict[str, object], milestone_id: str) -> bool:
    if report.get("status") != "pass":
        return False

    if milestone_id != "M3":
        return True

    manifest = report.get("_manifest", {})
    recovery = report.get("_recovery", {})
    if not isinstance(manifest, dict) or not isinstance(recovery, dict):
        return False

    runtime = manifest.get("runtime", {})
    if not isinstance(runtime, dict):
        return False

    app_runtime_sec = int(runtime.get("app_runtime_sec") or 0)
    recovery_ms = int(recovery.get("recovery_ms") or report.get("recovery_ms") or 0)
    wall_runtime_sec = extract_wall_runtime_sec(report, recovery)
    stable_before = recovery.get("stable_before", {})
    stable_after = recovery.get("stable_after", {})

    return (
        app_runtime_sec >= M3_MIN_APP_RUNTIME_SEC
        and wall_runtime_sec is not None
        and wall_runtime_sec >= M3_MIN_APP_RUNTIME_SEC
        and recovery_ms > 0
        and recovery_ms <= M3_MAX_RECOVERY_MS
        and isinstance(stable_before, dict)
        and bool(stable_before.get("ok"))
        and isinstance(stable_after, dict)
        and bool(stable_after.get("ok"))
    )


def report_completion_note(report: Dict[str, object], milestone_id: str) -> Optional[str]:
    if report_counts_as_pass(report, milestone_id):
        return None

    status = str(report.get("status") or "")
    summary = str(report.get("summary") or "").strip()
    if status and status != "pass":
        if summary:
            return f"最近一次 run 为 {status}：{summary}"
        return f"最近一次 run 为 {status}"
    if milestone_id != "M3":
        return "最近一次 run 未形成有效 pass"

    manifest = report.get("_manifest", {})
    recovery = report.get("_recovery", {})
    runtime = manifest.get("runtime", {}) if isinstance(manifest, dict) else {}
    app_runtime_sec = int(runtime.get("app_runtime_sec") or 0) if isinstance(runtime, dict) else 0
    wall_runtime_sec = extract_wall_runtime_sec(
        report,
        recovery if isinstance(recovery, dict) else {},
    )
    recovery_ms = int(
        (recovery.get("recovery_ms") if isinstance(recovery, dict) else 0)
        or report.get("recovery_ms")
        or 0
    )
    stable_before = recovery.get("stable_before", {}) if isinstance(recovery, dict) else {}
    stable_after = recovery.get("stable_after", {}) if isinstance(recovery, dict) else {}

    if app_runtime_sec < M3_MIN_APP_RUNTIME_SEC:
        return (
            "最近一次 run 虽报告 pass，但只记录了 "
            f"{app_runtime_sec}s，未达到 M3 要求的 {M3_MIN_APP_RUNTIME_SEC}s"
        )
    if wall_runtime_sec is None:
        return "最近一次 run 虽报告 pass，但缺少真实 wall-clock 运行时长"
    if wall_runtime_sec < M3_MIN_APP_RUNTIME_SEC:
        return (
            "最近一次 run 虽报告 pass，但真实 wall-clock 仅 "
            f"{wall_runtime_sec:.1f}s，未达到 M3 要求的 {M3_MIN_APP_RUNTIME_SEC}s"
        )
    if recovery_ms <= 0 or recovery_ms > M3_MAX_RECOVERY_MS:
        return f"最近一次 run 虽报告 pass，但恢复时长 {recovery_ms}ms 不满足 M3 门槛"
    if not isinstance(stable_before, dict) or not stable_before.get("ok"):
        return "最近一次 run 虽报告 pass，但缺少断线前稳定刷新窗口"
    if not isinstance(stable_after, dict) or not stable_after.get("ok"):
        return "最近一次 run 虽报告 pass，但缺少恢复后稳定刷新窗口"
    return "最近一次 run 虽报告 pass，但未满足 coordinator 的 M3 通过条件"


def parse_iso_datetime(value: object) -> Optional[datetime]:
    if not isinstance(value, str) or not value.strip():
        return None
    try:
        return datetime.fromisoformat(value)
    except ValueError:
        return None


def extract_wall_runtime_sec(
    report: Dict[str, object],
    recovery: Dict[str, object],
) -> Optional[float]:
    for container in (report, recovery):
        raw_value = container.get("wall_runtime_sec")
        if isinstance(raw_value, (int, float)):
            return float(raw_value)

    started_at = parse_iso_datetime(report.get("started_at"))
    finished_at = parse_iso_datetime(report.get("finished_at"))
    if started_at is None or finished_at is None:
        return None

    elapsed = (finished_at - started_at).total_seconds()
    if elapsed < 0:
        return None
    return elapsed


def latest_pass_for_milestone(
    reports: List[Dict[str, object]],
    milestone_id: str,
) -> Optional[Dict[str, object]]:
    candidates: List[Dict[str, object]] = []
    for report in reports:
        manifest = report.get("_manifest", {})
        if not isinstance(manifest, dict):
            continue
        if manifest.get("milestone") != milestone_id:
            continue
        if not report_counts_as_pass(report, milestone_id):
            continue
        candidates.append(report)
    if not candidates:
        return None
    candidates.sort(key=lambda item: str(item.get("finished_at", "")))
    return candidates[-1]


def latest_report_for_milestone(
    reports: List[Dict[str, object]],
    milestone_id: str,
) -> Optional[Dict[str, object]]:
    candidates: List[Dict[str, object]] = []
    for report in reports:
        manifest = report.get("_manifest", {})
        if not isinstance(manifest, dict):
            continue
        if manifest.get("milestone") != milestone_id:
            continue
        candidates.append(report)
    if not candidates:
        return None
    candidates.sort(key=lambda item: str(item.get("finished_at", "")))
    return candidates[-1]


def build_state(
    artifact_root: Path,
    reports: List[Dict[str, object]],
    target: str,
) -> Dict[str, object]:
    milestones = []
    for spec in WORKFLOW:
        latest = latest_pass_for_milestone(reports, spec.id)
        latest_attempt = latest_report_for_milestone(reports, spec.id)
        milestones.append(
            {
                "id": spec.id,
                "title": spec.title,
                "runner": spec.runner,
                "status": "pass" if latest else "pending",
                "latest_pass_run": latest.get("run_id") if latest else None,
                "latest_report": display_path(Path(str(latest["_report_path"])))
                if latest
                else None,
                "latest_attempt_run": latest_attempt.get("run_id") if latest_attempt else None,
                "latest_attempt_status": latest_attempt.get("status") if latest_attempt else None,
                "latest_attempt_summary": latest_attempt.get("summary") if latest_attempt else None,
                "latest_attempt_report": display_path(Path(str(latest_attempt["_report_path"])))
                if latest_attempt
                else None,
                "note": report_completion_note(latest_attempt, spec.id)
                if latest is None and latest_attempt
                else None,
            }
        )
    return {
        "updated_at": now_iso(),
        "target": target,
        "artifact_root": display_path(artifact_root),
        "milestones": milestones,
    }


def find_first_unfinished(
    reports: List[Dict[str, object]],
    target: str,
) -> Optional[MilestoneSpec]:
    for spec in WORKFLOW:
        if latest_pass_for_milestone(reports, spec.id) is None:
            return spec
        if spec.id == target:
            return None
    return None


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def write_json(path: Path, payload: object) -> None:
    ensure_parent(path)
    path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def run_runner(
    spec: MilestoneSpec,
    hosts_env: Path,
    extra_args: List[str],
) -> subprocess.CompletedProcess[str]:
    runner_path = ROOT / spec.runner
    if not runner_path.exists():
        raise FileNotFoundError(str(runner_path))
    command = ["python3", str(runner_path), "--hosts-env", str(hosts_env)]
    command.extend(spec.default_args)
    command.extend(extra_args)
    return subprocess.run(
        command,
        cwd=str(ROOT),
        text=True,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )


def write_coordinator_report(
    artifact_root: Path,
    coordinator_run_id: str,
    status: str,
    summary: str,
    steps: List[Dict[str, object]],
) -> Path:
    run_dir = artifact_root / coordinator_run_id
    run_dir.mkdir(parents=True, exist_ok=True)
    report_path = run_dir / "report.json"
    write_json(
        report_path,
        {
            "run_id": coordinator_run_id,
            "kind": "coordinator",
            "status": status,
            "summary": summary,
            "finished_at": now_iso(),
            "steps": steps,
        },
    )
    return report_path


def write_sync_artifacts(
    artifact_root: Path,
    coordinator_run_id: str,
    status: str,
    summary: str,
    results: List[run_m0.CommandResult],
) -> List[str]:
    run_dir = artifact_root / coordinator_run_id
    sync_dir = run_dir / "sync"
    log_path = sync_dir / "remote-sync.log"
    state_path = sync_dir / "remote-sync.json"
    titles = ["rsync-dry-run", "rsync-apply"]
    for index, result in enumerate(results):
        title = titles[index] if index < len(titles) else f"rsync-step-{index + 1}"
        run_m0.append_section(log_path, title, result.stdout, result.stderr)
    write_json(
        state_path,
        {
            "status": status,
            "summary": summary,
            "commands": run_m0.summarize_commands_for_artifact(results),
            "updated_at": now_iso(),
        },
    )
    return [display_path(log_path), display_path(state_path)]


def main() -> int:
    parser = argparse.ArgumentParser(description="Run NetMic top-level harness coordinator")
    parser.add_argument("--hosts-env", default=str(DEFAULT_HOSTS_ENV))
    parser.add_argument("--until", choices=[spec.id for spec in WORKFLOW], default="M3")
    parser.add_argument("--run-m0-duration", type=int, default=300)
    args = parser.parse_args()

    hosts_env = Path(args.hosts_env).expanduser().resolve()
    if not hosts_env.exists():
        print(f"blocked: hosts env not found: {hosts_env}", file=sys.stderr)
        return 2

    env = parse_env_file(hosts_env)
    artifact_root = artifact_root_from_env(env)
    coordinator_run_id = datetime.now().astimezone().strftime("coordinator-%Y%m%dT%H%M%S")
    reports = load_run_reports(artifact_root)
    state = build_state(artifact_root, reports, args.until)
    write_json(STATE_PATH, state)

    steps: List[Dict[str, object]] = []
    final_status = "pass"
    final_summary = f"所有目标里程碑已完成（截至 {args.until}）"
    sync_keys = (
        "NETMIC_HARNESS_LINUX_HOST",
        "NETMIC_HARNESS_LINUX_USER",
        "NETMIC_HARNESS_LINUX_ROOT",
    )
    if all(env.get(key, "").strip() for key in sync_keys):
        prepare_status, prepare_summary = run_m0.require_local_tools(
            password_auth=bool(
                env.get("NETMIC_HARNESS_LINUX_PASSWORD", "")
                and not env.get("NETMIC_HARNESS_LINUX_SSH_KEY", "")
            ),
            needs_node=False,
            needs_rsync=True,
        )
        steps.append(
            {
                "id": "sync-remote-prepare",
                "status": prepare_status,
                "summary": prepare_summary,
            }
        )
        if prepare_status != "pass":
            final_status = prepare_status
            final_summary = prepare_summary
        else:
            sync_status, sync_summary, sync_logs = run_m0.sync_remote_workspace(env)
            artifacts = write_sync_artifacts(
                artifact_root,
                coordinator_run_id,
                sync_status,
                sync_summary,
                sync_logs,
            )
            steps.append(
                {
                    "id": "sync-remote",
                    "status": sync_status,
                    "summary": sync_summary,
                    "artifacts": artifacts,
                }
            )
            if sync_status != "pass":
                final_status = sync_status
                final_summary = sync_summary

    while final_status == "pass":
        reports = load_run_reports(artifact_root)
        unfinished = find_first_unfinished(reports, args.until)
        if unfinished is None:
            steps.append(
                {
                    "id": "scan",
                    "status": "pass",
                    "summary": final_summary,
                }
            )
            break

        steps.append(
            {
                "id": f"scan-{unfinished.id}",
                "status": "pass",
                "summary": f"下一个未完成里程碑：{unfinished.id} {unfinished.title}",
            }
        )

        runner_path = ROOT / unfinished.runner
        if not runner_path.exists():
            final_status = "fail"
            final_summary = (
                f"{unfinished.id} 缺少专门 runner：{display_path(runner_path)}。"
                " 这不是环境 blocked，而是 Harness 编排未闭环。"
            )
            steps.append(
                {
                    "id": f"dispatch-{unfinished.id}",
                    "status": "fail",
                    "summary": final_summary,
                    "runner": display_path(runner_path),
                }
            )
            break

        extra_args: List[str] = []
        if unfinished.id == "M0":
            extra_args.extend(["--duration", str(max(1, args.run_m0_duration))])
        result = run_runner(unfinished, hosts_env, extra_args)
        runner_status = "pass" if result.returncode == 0 else ("blocked" if result.returncode == 2 else "fail")
        summary = (
            result.stdout.strip()
            or result.stderr.strip()
            or f"{unfinished.id} runner exited with {result.returncode}"
        )
        steps.append(
            {
                "id": f"dispatch-{unfinished.id}",
                "status": runner_status,
                "summary": summary,
                "runner": display_path(runner_path),
            }
        )
        if runner_status != "pass":
            final_status = runner_status
            final_summary = summary
            break
        final_summary = f"{unfinished.id} 已通过，继续推进"

    report_path = write_coordinator_report(
        artifact_root,
        coordinator_run_id,
        final_status,
        final_summary,
        steps,
    )

    refreshed_reports = load_run_reports(artifact_root)
    state = build_state(artifact_root, refreshed_reports, args.until)
    state["coordinator"] = {
        "last_run": display_path(report_path),
        "status": final_status,
        "summary": final_summary,
    }
    write_json(STATE_PATH, state)
    print(f"{final_status}: {final_summary}")
    if final_status == "pass":
        return 0
    if final_status == "blocked":
        return 2
    return 1


if __name__ == "__main__":
    sys.exit(main())
