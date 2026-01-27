#!/usr/bin/env python3
"""NetMic MVP Gate 验证入口（单命令真相）。

设计目标：
- 以 docs/MVP_GATES.yaml 作为机器可读 Gate 定义。
- 输出统一 JSON 结果，供监督器/Agent 低上下文读取。
- 在环境阻塞（依赖/权限/端口）时明确标记 BLOCKED，避免盲目自改。
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import shlex
import subprocess
import sys
import textwrap
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_GATES_FILE = ROOT / "docs" / "MVP_GATES.yaml"
DEFAULT_STATE_DIR = ROOT / ".autopilot"

PASS = "pass"
FAIL = "fail"
BLOCKED = "blocked"
PENDING = "pending"


@dataclasses.dataclass
class CheckResult:
    check_id: str
    name: str
    run: str
    status: str
    exit_code: Optional[int]
    duration_sec: float
    hint: str
    output_tail: List[str]
    blocked_reason: Optional[str] = None


@dataclasses.dataclass
class GateResult:
    gate_id: str
    title: str
    description: str
    doc_refs: List[str]
    status: str
    checks: List[CheckResult]


def ts_now() -> str:
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")


def load_gates(path: Path) -> Dict[str, Any]:
    # 使用 JSON 子集格式，避免引入 PyYAML 依赖。
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise SystemExit(f"gates file not found: {path}")
    except json.JSONDecodeError as exc:
        raise SystemExit(f"gates file is not valid JSON/YAML subset: {path} ({exc})")


def merge_blocked_patterns(defaults: Sequence[str], check: Dict[str, Any]) -> List[str]:
    patterns = list(defaults)
    patterns.extend(str(p) for p in check.get("blocked_patterns", []) if p)
    # 统一 lower-case 比较。
    return [p.lower() for p in patterns]


def extract_first_token(cmd: str) -> str:
    try:
        parts = shlex.split(cmd, posix=True)
    except ValueError:
        parts = cmd.strip().split()
    return parts[0] if parts else ""


def is_repo_script_missing(cmd: str) -> Tuple[bool, str]:
    token = extract_first_token(cmd)
    if not token:
        return False, ""
    if token.startswith("scripts/"):
        script_path = ROOT / token
        if not script_path.exists():
            return True, token
    return False, token


def shorten_lines(text: str, max_lines: int) -> List[str]:
    lines = [ln.rstrip() for ln in text.splitlines()]
    if len(lines) <= max_lines:
        return lines
    head = lines[: max_lines // 2]
    tail = lines[-(max_lines - len(head)) :]
    return head + ["…(truncated)…"] + tail


def pick_hint(output_tail: Sequence[str]) -> str:
    if not output_tail:
        return "(no output)"
    keywords = ("error", "fail", "denied", "not found", "missing", "blocked")
    for line in output_tail:
        lower = line.lower()
        if any(k in lower for k in keywords) and line.strip():
            return line.strip()[:200]
    for line in reversed(output_tail):
        if line.strip():
            return line.strip()[:200]
    return "(no non-empty output)"


def output_matches_blocked(output_tail: Sequence[str], patterns: Sequence[str]) -> Optional[str]:
    if not output_tail:
        return None
    joined = "\n".join(output_tail).lower()
    for pat in patterns:
        if pat and pat in joined:
            return pat
    return None


def compute_failure_signature(
    gate_id: str,
    check_id: str,
    status: str,
    exit_code: Optional[int],
    hint: str,
) -> str:
    base = f"{gate_id}|{check_id}|{status}|{exit_code}|{hint.strip().lower()}"
    digest = hashlib.sha1(base.encode("utf-8")).hexdigest()[:10]
    exit_part = f"exit={exit_code}" if exit_code is not None else "exit=na"
    return f"{gate_id}/{check_id}/{status}/{exit_part}/sig={digest}"


def run_check(
    gate_id: str,
    check: Dict[str, Any],
    defaults: Dict[str, Any],
    blocked_patterns_default: Sequence[str],
    max_output_lines: int,
) -> CheckResult:
    check_id = str(check["id"])
    name = str(check.get("name") or check_id)
    cmd = str(check["run"])

    missing, token = is_repo_script_missing(cmd)
    if missing:
        hint = f"missing repo script: {token}"
        return CheckResult(
            check_id=check_id,
            name=name,
            run=cmd,
            status=FAIL,
            exit_code=127,
            duration_sec=0.0,
            hint=hint,
            output_tail=[hint],
        )

    success_exit_codes = [int(x) for x in check.get("success_exit_codes", defaults.get("success_exit_codes", [0]))]
    blocked_exit_codes = [int(x) for x in check.get("blocked_exit_codes", defaults.get("blocked_exit_codes", [126, 127]))]
    timeout_sec = int(check.get("timeout_sec", defaults.get("timeout_sec", 300)))

    blocked_patterns = merge_blocked_patterns(blocked_patterns_default, check)

    start = datetime.now(timezone.utc)
    try:
        proc = subprocess.run(
            ["bash", "-lc", cmd],
            cwd=str(ROOT),
            text=True,
            capture_output=True,
            timeout=timeout_sec,
            env=os.environ.copy(),
            encoding="utf-8",
            errors="replace",
        )
        exit_code = proc.returncode
        output = (proc.stdout or "") + (proc.stderr or "")
    except subprocess.TimeoutExpired as exc:
        duration = (datetime.now(timezone.utc) - start).total_seconds()
        output = (exc.stdout or "") + "\n" + (exc.stderr or "")
        output_tail = shorten_lines(output, max_output_lines)
        hint = f"timeout after {timeout_sec}s"
        return CheckResult(
            check_id=check_id,
            name=name,
            run=cmd,
            status=FAIL,
            exit_code=None,
            duration_sec=duration,
            hint=hint,
            output_tail=output_tail or [hint],
        )

    duration = (datetime.now(timezone.utc) - start).total_seconds()
    output_tail = shorten_lines(output, max_output_lines)
    hint = pick_hint(output_tail)

    blocked_match = output_matches_blocked(output_tail, blocked_patterns)
    if blocked_match:
        return CheckResult(
            check_id=check_id,
            name=name,
            run=cmd,
            status=BLOCKED,
            exit_code=exit_code,
            duration_sec=duration,
            hint=hint,
            output_tail=output_tail,
            blocked_reason=f"pattern:{blocked_match}",
        )

    # 对 repo 内脚本，避免将 “脚本缺失/未实现” 误判为 blocked。
    repo_script = token.startswith("scripts/") if token else False
    if exit_code in blocked_exit_codes and not repo_script:
        return CheckResult(
            check_id=check_id,
            name=name,
            run=cmd,
            status=BLOCKED,
            exit_code=exit_code,
            duration_sec=duration,
            hint=hint,
            output_tail=output_tail,
            blocked_reason=f"exit_code:{exit_code}",
        )

    if exit_code in success_exit_codes:
        return CheckResult(
            check_id=check_id,
            name=name,
            run=cmd,
            status=PASS,
            exit_code=exit_code,
            duration_sec=duration,
            hint="pass",
            output_tail=output_tail,
        )

    return CheckResult(
        check_id=check_id,
        name=name,
        run=cmd,
        status=FAIL,
        exit_code=exit_code,
        duration_sec=duration,
        hint=hint,
        output_tail=output_tail,
    )


def evaluate_gates(
    gates_doc: Dict[str, Any],
    max_output_lines: int,
) -> Tuple[List[GateResult], Dict[str, Any]]:
    defaults = dict(gates_doc.get("verify_defaults", {}))
    blocked_patterns_default = [str(p).lower() for p in gates_doc.get("blocked_patterns_default", [])]

    gate_results: List[GateResult] = []
    first_non_pass: Optional[GateResult] = None
    first_failure: Optional[Tuple[GateResult, CheckResult]] = None

    for gate in gates_doc.get("gates", []):
        gate_id = str(gate["id"])
        title = str(gate.get("title") or gate_id)
        description = str(gate.get("description") or "")
        doc_refs = [str(x) for x in gate.get("doc_refs", [])]

        if first_non_pass is not None:
            gate_results.append(
                GateResult(
                    gate_id=gate_id,
                    title=title,
                    description=description,
                    doc_refs=doc_refs,
                    status=PENDING,
                    checks=[],
                )
            )
            continue

        checks: List[CheckResult] = []
        gate_status = PASS
        for check in gate.get("checks", []):
            check_result = run_check(
                gate_id=gate_id,
                check=check,
                defaults=defaults,
                blocked_patterns_default=blocked_patterns_default,
                max_output_lines=max_output_lines,
            )
            checks.append(check_result)
            if check_result.status != PASS:
                gate_status = check_result.status
                if first_failure is None:
                    first_failure = (
                        GateResult(
                            gate_id=gate_id,
                            title=title,
                            description=description,
                            doc_refs=doc_refs,
                            status=gate_status,
                            checks=list(checks),
                        ),
                        check_result,
                    )
                break

        gate_result = GateResult(
            gate_id=gate_id,
            title=title,
            description=description,
            doc_refs=doc_refs,
            status=gate_status,
            checks=checks,
        )
        gate_results.append(gate_result)
        if gate_status != PASS:
            first_non_pass = gate_result

    passed_gates = [g.gate_id for g in gate_results if g.status == PASS]
    active_gate_id = first_non_pass.gate_id if first_non_pass else (gate_results[-1].gate_id if gate_results else "")

    overall_status = PASS
    if first_non_pass:
        overall_status = first_non_pass.status

    failure_payload: Dict[str, Any] = {}
    failure_signature = ""
    if first_failure:
        gate_result, check_result = first_failure
        failure_signature = compute_failure_signature(
            gate_result.gate_id,
            check_result.check_id,
            check_result.status,
            check_result.exit_code,
            check_result.hint,
        )
        failure_payload = {
            "gate_id": gate_result.gate_id,
            "gate_title": gate_result.title,
            "check_id": check_result.check_id,
            "check_name": check_result.name,
            "status": check_result.status,
            "exit_code": check_result.exit_code,
            "hint": check_result.hint,
            "blocked_reason": check_result.blocked_reason,
            "run": check_result.run,
            "output_tail": check_result.output_tail,
            "failure_signature": failure_signature,
        }

    progress_signature = hashlib.sha1("|".join(passed_gates).encode("utf-8")).hexdigest()[:10] if passed_gates else "none"

    summary = {
        "overall_status": overall_status,
        "active_gate_id": active_gate_id,
        "passed_gates": passed_gates,
        "progress_signature": progress_signature,
        "failure": failure_payload,
        "failure_signature": failure_signature,
    }
    return gate_results, summary


def gate_to_dict(gate: GateResult) -> Dict[str, Any]:
    return {
        "gate_id": gate.gate_id,
        "title": gate.title,
        "description": gate.description,
        "doc_refs": gate.doc_refs,
        "status": gate.status,
        "checks": [
            {
                "check_id": c.check_id,
                "name": c.name,
                "run": c.run,
                "status": c.status,
                "exit_code": c.exit_code,
                "duration_sec": round(c.duration_sec, 3),
                "hint": c.hint,
                "blocked_reason": c.blocked_reason,
                "output_tail": c.output_tail,
            }
            for c in gate.checks
        ],
    }


def render_text(summary: Dict[str, Any], gates: Sequence[GateResult]) -> str:
    lines: List[str] = []
    lines.append("== NetMic MVP Gate 验证 ==")
    lines.append(f"time:   {ts_now()}")
    lines.append(f"result: {summary['overall_status']}")
    lines.append(f"active: {summary['active_gate_id']}")
    lines.append("")

    for gate in gates:
        lines.append(f"[{gate.status}] {gate.gate_id} - {gate.title}")
        if gate.status in {FAIL, BLOCKED} and gate.checks:
            last = gate.checks[-1]
            lines.append(f"  check: {last.check_id} ({last.status}) exit={last.exit_code}")
            lines.append(f"  hint:  {last.hint}")
        lines.append("")

    failure = summary.get("failure") or {}
    if failure:
        lines.append("-- failure detail --")
        lines.append(f"gate/check: {failure.get('gate_id')}/{failure.get('check_id')}")
        lines.append(f"signature:  {failure.get('failure_signature')}")
        lines.append(f"run:        {failure.get('run')}")
        hint = failure.get("hint")
        if hint:
            lines.append(f"hint:       {hint}")
        output_tail = failure.get("output_tail") or []
        if output_tail:
            lines.append("output tail:")
            for ln in output_tail[-20:]:
                lines.append(f"  {ln}")
    return "\n".join(lines).rstrip() + "\n"


def ensure_state_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Verify NetMic MVP gates")
    parser.add_argument("--gates", default=str(DEFAULT_GATES_FILE), help="Path to docs/MVP_GATES.yaml")
    parser.add_argument("--state-dir", default=str(DEFAULT_STATE_DIR), help="State directory (default: .autopilot)")
    parser.add_argument("--json-out", default="", help="Write JSON result to this file (default: <state-dir>/verify_status.json)")
    parser.add_argument("--text-out", default="", help="Write text summary to this file (default: <state-dir>/verify_status.txt)")
    parser.add_argument("--hub-url", default=os.environ.get("HUB_URL", ""), help="Hub URL for checks that need it")
    parser.add_argument("--max-output-lines", type=int, default=120, help="Max lines to keep per check output tail")
    parser.add_argument("--print-json", action="store_true", help="Also print JSON result to stdout")
    return parser.parse_args(list(argv))


def main(argv: Sequence[str]) -> int:
    args = parse_args(argv)

    gates_path = Path(args.gates)
    state_dir = Path(args.state_dir)
    ensure_state_dir(state_dir)

    json_out = Path(args.json_out) if args.json_out else state_dir / "verify_status.json"
    text_out = Path(args.text_out) if args.text_out else state_dir / "verify_status.txt"

    gates_doc = load_gates(gates_path)

    hub_url = args.hub_url.strip() or os.environ.get("HUB_URL", "").strip()
    if not hub_url:
        hub_url = "http://127.0.0.1:7788"
    os.environ["HUB_URL"] = hub_url

    gates, summary = evaluate_gates(gates_doc, max_output_lines=max(40, args.max_output_lines))

    result = {
        "ok": True,
        "generated_at": ts_now(),
        "root": str(ROOT),
        "hub_url": hub_url,
        "gates_file": str(gates_path),
        "overall": summary,
        "gates": [gate_to_dict(g) for g in gates],
    }

    json_out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    text_out.write_text(render_text(summary, gates), encoding="utf-8")

    if args.print_json:
        print(json.dumps(result, ensure_ascii=False))
    else:
        # 默认输出简短文本，便于命令行直接观察。
        sys.stdout.write(render_text(summary, gates))

    status = summary.get("overall_status")
    if status == PASS:
        return 0
    if status == BLOCKED:
        return 2
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
