#!/usr/bin/env bash
# run_m1.py 远端服务生命周期脚本最小自测：启动前要清理旧 pid，停止时要等待并必要时强杀。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

PYTHONPATH="$ROOT/scripts/harness" python3 - <<'PY'
import run_m0
import run_m1


captured = []
original = run_m0.run_remote


def fake_run_remote(env, script):
    captured.append(script)
    return run_m0.CommandResult(command=["ssh"], returncode=0, stdout="", stderr="")


run_m0.run_remote = fake_run_remote
try:
    env = {
        "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
        "NETMIC_HARNESS_LINUX_USER": "arc",
        "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
    }
    remote_dir = "/tmp/netmic/server"
    run_m1.remote_start_server(env, remote_dir, 43000)
    run_m1.remote_stop_server(env, remote_dir)
finally:
    run_m0.run_remote = original

assert len(captured) == 2, captured
start_script, stop_script = captured

assert f"if [ -f {remote_dir}/server.pid ]; then" in start_script
assert 'kill "$pid" >/dev/null 2>&1 || true' in start_script
assert 'kill -0 "$pid" >/dev/null 2>&1' in start_script
assert 'kill -9 "$pid" >/dev/null 2>&1 || true' in start_script
assert f"rm -f {remote_dir}/server.pid {remote_dir}/runtime.log {remote_dir}/audio_dump.pcm" in start_script
assert f"NETMIC_SERVER_AUDIO_DUMP={remote_dir}/audio_dump.pcm" in start_script
assert "echo $! >" in start_script
assert "sleep 1" in start_script

assert f'pid="$(cat {remote_dir}/server.pid)"' in stop_script
assert 'kill "$pid" >/dev/null 2>&1 || true' in stop_script
assert 'kill -0 "$pid" >/dev/null 2>&1' in stop_script
assert 'kill -9 "$pid" >/dev/null 2>&1 || true' in stop_script
assert f"rm -f {remote_dir}/server.pid" in stop_script
PY

echo "[ok] run_m1 remote server lifecycle script test passed"
