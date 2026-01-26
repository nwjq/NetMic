#!/usr/bin/env python3
import argparse
import json
import socket
import sys
import time
import urllib.request


def request_json(method: str, url: str, payload=None, timeout: int = 10):
    data = None
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=True).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
    except Exception as exc:
        return {"ok": False, "error": str(exc)}
    try:
        return json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError:
        return {"ok": False, "error": "bad_json", "raw": raw.decode("utf-8", "ignore")}


def register(hub: str, agent_id: str, role: str):
    payload = {
        "agent_id": agent_id,
        "role": role,
        "capabilities": {"host": socket.gethostname()},
        "host": socket.gethostname(),
        "tags": [],
        "status": "online",
    }
    return request_json("POST", f"{hub}/v1/register", payload)


def create_session(hub: str, agent_id: str, session_id: str):
    payload = {"agent_id": agent_id, "session_id": session_id, "params": {}}
    return request_json("POST", f"{hub}/v1/test_session_create", payload)


def join_session(hub: str, agent_id: str, session_id: str):
    payload = {"agent_id": agent_id, "session_id": session_id}
    return request_json("POST", f"{hub}/v1/test_session_join", payload)


def send_event(hub: str, agent_id: str, session_id: str, role: str):
    payload = {
        "agent_id": agent_id,
        "session_id": session_id,
        "kind": "node_ready",
        "role": role,
    }
    return request_json("POST", f"{hub}/v1/test_session_event", payload)


def wait_for_peer(hub: str, session_id: str, peer_role: str, timeout: int):
    start = time.time()
    since = 0
    while time.time() - start < timeout:
        resp = request_json("GET", f"{hub}/v1/events?since={since}&limit=200")
        if not resp.get("ok"):
            time.sleep(1)
            continue
        events = resp.get("events", [])
        for ev in events:
            since = max(since, int(ev.get("id", since)))
            if ev.get("kind") != "session.node_ready":
                continue
            payload = ev.get("payload", {})
            if payload.get("session_id") != session_id:
                continue
            if payload.get("role") == peer_role:
                return True
        time.sleep(1)
    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["health", "coord", "join"], required=True)
    parser.add_argument("--hub", required=True)
    parser.add_argument("--agent-id", default="agent")
    parser.add_argument("--role", default="node")
    parser.add_argument("--session-id", default="")
    parser.add_argument("--peer-role", default="")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()

    hub = args.hub.rstrip("/")

    if args.mode == "health":
        resp = request_json("GET", f"{hub}/v1/health")
        print(json.dumps(resp, ensure_ascii=True))
        return 0 if resp.get("ok") else 1

    reg = register(hub, args.agent_id, args.role)
    if not reg.get("ok"):
        print(json.dumps(reg, ensure_ascii=True))
        return 1

    if args.mode == "coord":
        if not args.session_id:
            print(json.dumps({"ok": False, "error": "session_id_required"}, ensure_ascii=True))
            return 1
        created = create_session(hub, args.agent_id, args.session_id)
        if not created.get("ok"):
            print(json.dumps(created, ensure_ascii=True))
            return 1
        ready = send_event(hub, args.agent_id, args.session_id, args.role)
        if not ready.get("ok"):
            print(json.dumps(ready, ensure_ascii=True))
            return 1
        print(json.dumps({"ok": True, "session_id": args.session_id}, ensure_ascii=True))
        return 0

    if args.mode == "join":
        if not args.session_id:
            print(json.dumps({"ok": False, "error": "session_id_required"}, ensure_ascii=True))
            return 1
        joined = join_session(hub, args.agent_id, args.session_id)
        if not joined.get("ok"):
            print(json.dumps(joined, ensure_ascii=True))
            return 1
        ready = send_event(hub, args.agent_id, args.session_id, args.role)
        if not ready.get("ok"):
            print(json.dumps(ready, ensure_ascii=True))
            return 1
        if args.peer_role:
            ok = wait_for_peer(hub, args.session_id, args.peer_role, args.timeout)
            if not ok:
                print(json.dumps({"ok": False, "error": "peer_timeout"}, ensure_ascii=True))
                return 1
        print(json.dumps({"ok": True}, ensure_ascii=True))
        return 0

    return 0


if __name__ == "__main__":
    sys.exit(main())
