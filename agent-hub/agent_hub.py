#!/usr/bin/env python3
import json
import os
import sqlite3
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs


DB_PATH = os.environ.get("AGENT_HUB_DB", "./agent_hub.db")
HOST = os.environ.get("AGENT_HUB_HOST", "0.0.0.0")
PORT = int(os.environ.get("AGENT_HUB_PORT", "7788"))
EVENT_TTL_SECONDS = int(os.environ.get("AGENT_HUB_EVENT_TTL_SECONDS", "86400"))


def now_ts() -> int:
    return int(time.time())


def json_dumps(obj) -> str:
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=True)


def db_connect() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    return conn


def db_init(conn: sqlite3.Connection) -> None:
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            role TEXT,
            capabilities TEXT,
            host TEXT,
            tags TEXT,
            status TEXT,
            last_seen INTEGER
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT,
            claimed_by TEXT,
            meta TEXT,
            summary TEXT,
            updated_at INTEGER
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS task_updates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT,
            agent_id TEXT,
            status TEXT,
            summary TEXT,
            patch TEXT,
            links TEXT,
            metrics TEXT,
            created_at INTEGER
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            status TEXT,
            created_by TEXT,
            params TEXT,
            created_at INTEGER,
            updated_at INTEGER
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS session_participants (
            session_id TEXT,
            agent_id TEXT,
            joined_at INTEGER,
            PRIMARY KEY (session_id, agent_id)
        )
        """
    )
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT,
            ref_id TEXT,
            agent_id TEXT,
            payload TEXT,
            created_at INTEGER
        )
        """
    )
    conn.commit()


def db_cleanup(conn: sqlite3.Connection) -> None:
    cutoff = now_ts() - EVENT_TTL_SECONDS
    conn.execute("DELETE FROM events WHERE created_at < ?", (cutoff,))
    conn.execute("DELETE FROM task_updates WHERE created_at < ?", (cutoff,))
    conn.commit()


def log_event(conn: sqlite3.Connection, kind: str, ref_id: str, agent_id: str, payload: dict) -> int:
    cur = conn.execute(
        "INSERT INTO events (kind, ref_id, agent_id, payload, created_at) VALUES (?, ?, ?, ?, ?)",
        (kind, ref_id, agent_id, json_dumps(payload), now_ts()),
    )
    conn.commit()
    return int(cur.lastrowid)


class AgentHubHandler(BaseHTTPRequestHandler):
    server_version = "AgentHub/0.1"

    def _send_json(self, status: int, body: dict) -> None:
        payload = json_dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _read_json(self) -> dict:
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0:
            return {}
        raw = self.rfile.read(length)
        try:
            return json.loads(raw.decode("utf-8"))
        except json.JSONDecodeError:
            return {}

    def _route(self, method: str, path: str):
        if method == "GET" and path == "/v1/health":
            return self.handle_health
        if method == "GET" and path == "/v1/events":
            return self.handle_events
        if method == "POST" and path == "/v1/register":
            return self.handle_register
        if method == "POST" and path == "/v1/heartbeat":
            return self.handle_heartbeat
        if method == "POST" and path == "/v1/claim_task":
            return self.handle_claim_task
        if method == "POST" and path == "/v1/release_task":
            return self.handle_release_task
        if method == "POST" and path == "/v1/task_update":
            return self.handle_task_update
        if method == "POST" and path == "/v1/test_session_create":
            return self.handle_test_session_create
        if method == "POST" and path == "/v1/test_session_join":
            return self.handle_test_session_join
        if method == "POST" and path == "/v1/test_session_event":
            return self.handle_test_session_event
        return None

    def do_GET(self):
        parsed = urlparse(self.path)
        handler = self._route("GET", parsed.path)
        if not handler:
            return self._send_json(404, {"ok": False, "error": "not_found"})
        return handler(parsed)

    def do_POST(self):
        parsed = urlparse(self.path)
        handler = self._route("POST", parsed.path)
        if not handler:
            return self._send_json(404, {"ok": False, "error": "not_found"})
        return handler(parsed)

    def handle_health(self, parsed):
        with db_connect() as conn:
            db_cleanup(conn)
            agents = conn.execute("SELECT COUNT(*) as c FROM agents").fetchone()["c"]
            tasks = conn.execute("SELECT COUNT(*) as c FROM tasks").fetchone()["c"]
            sessions = conn.execute("SELECT COUNT(*) as c FROM sessions").fetchone()["c"]
        return self._send_json(
            200,
            {
                "ok": True,
                "time": now_ts(),
                "agents": agents,
                "tasks": tasks,
                "sessions": sessions,
            },
        )

    def handle_events(self, parsed):
        qs = parse_qs(parsed.query or "")
        since = int(qs.get("since", ["0"])[0])
        limit = int(qs.get("limit", ["200"])[0])
        limit = max(1, min(limit, 1000))
        with db_connect() as conn:
            db_cleanup(conn)
            rows = conn.execute(
                "SELECT id, kind, ref_id, agent_id, payload, created_at FROM events WHERE id > ? ORDER BY id ASC LIMIT ?",
                (since, limit),
            ).fetchall()
        events = [
            {
                "id": row["id"],
                "kind": row["kind"],
                "ref_id": row["ref_id"],
                "agent_id": row["agent_id"],
                "payload": json.loads(row["payload"]),
                "created_at": row["created_at"],
            }
            for row in rows
        ]
        return self._send_json(200, {"ok": True, "events": events})

    def handle_register(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id") or str(uuid.uuid4())
        role = body.get("role", "")
        capabilities = body.get("capabilities", {})
        host = body.get("host", "")
        tags = body.get("tags", [])
        status = body.get("status", "online")
        with db_connect() as conn:
            db_cleanup(conn)
            conn.execute(
                """
                INSERT INTO agents (agent_id, role, capabilities, host, tags, status, last_seen)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(agent_id) DO UPDATE SET
                    role=excluded.role,
                    capabilities=excluded.capabilities,
                    host=excluded.host,
                    tags=excluded.tags,
                    status=excluded.status,
                    last_seen=excluded.last_seen
                """,
                (
                    agent_id,
                    role,
                    json_dumps(capabilities),
                    host,
                    json_dumps(tags),
                    status,
                    now_ts(),
                ),
            )
            conn.commit()
            event_id = log_event(conn, "agent.register", agent_id, agent_id, body)
        return self._send_json(200, {"ok": True, "agent_id": agent_id, "event_id": event_id})

    def handle_heartbeat(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        if not agent_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_required"})
        status = body.get("status", "online")
        with db_connect() as conn:
            db_cleanup(conn)
            conn.execute(
                "UPDATE agents SET status=?, last_seen=? WHERE agent_id=?",
                (status, now_ts(), agent_id),
            )
            conn.commit()
            event_id = log_event(conn, "agent.heartbeat", agent_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})

    def handle_claim_task(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        task_id = body.get("task_id")
        if not agent_id or not task_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_and_task_id_required"})
        meta = body.get("meta", {})
        with db_connect() as conn:
            db_cleanup(conn)
            row = conn.execute(
                "SELECT claimed_by, status FROM tasks WHERE task_id=?",
                (task_id,),
            ).fetchone()
            if row and row["claimed_by"] and row["claimed_by"] != agent_id:
                return self._send_json(409, {"ok": False, "error": "task_busy", "claimed_by": row["claimed_by"]})
            conn.execute(
                """
                INSERT INTO tasks (task_id, status, claimed_by, meta, summary, updated_at)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(task_id) DO UPDATE SET
                    status=excluded.status,
                    claimed_by=excluded.claimed_by,
                    meta=excluded.meta,
                    updated_at=excluded.updated_at
                """,
                (task_id, "claimed", agent_id, json_dumps(meta), "", now_ts()),
            )
            conn.commit()
            event_id = log_event(conn, "task.claim", task_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})

    def handle_release_task(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        task_id = body.get("task_id")
        status = body.get("status", "released")
        if not agent_id or not task_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_and_task_id_required"})
        with db_connect() as conn:
            db_cleanup(conn)
            conn.execute(
                "UPDATE tasks SET status=?, claimed_by=NULL, updated_at=? WHERE task_id=?",
                (status, now_ts(), task_id),
            )
            conn.commit()
            event_id = log_event(conn, "task.release", task_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})

    def handle_task_update(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        task_id = body.get("task_id")
        if not agent_id or not task_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_and_task_id_required"})
        status = body.get("status", "in_progress")
        summary = body.get("summary", "")
        patch = body.get("patch", "")
        links = body.get("links", [])
        metrics = body.get("metrics", {})
        with db_connect() as conn:
            db_cleanup(conn)
            conn.execute(
                """
                INSERT INTO task_updates (task_id, agent_id, status, summary, patch, links, metrics, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    task_id,
                    agent_id,
                    status,
                    summary,
                    patch,
                    json_dumps(links),
                    json_dumps(metrics),
                    now_ts(),
                ),
            )
            conn.execute(
                "UPDATE tasks SET status=?, summary=?, updated_at=? WHERE task_id=?",
                (status, summary, now_ts(), task_id),
            )
            conn.commit()
            event_id = log_event(conn, "task.update", task_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})

    def handle_test_session_create(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        session_id = body.get("session_id") or str(uuid.uuid4())
        params = body.get("params", {})
        if not agent_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_required"})
        with db_connect() as conn:
            db_cleanup(conn)
            row = conn.execute(
                "SELECT session_id FROM sessions WHERE session_id=?",
                (session_id,),
            ).fetchone()
            if row:
                return self._send_json(409, {"ok": False, "error": "session_exists"})
            conn.execute(
                """
                INSERT INTO sessions (session_id, status, created_by, params, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                (session_id, "created", agent_id, json_dumps(params), now_ts(), now_ts()),
            )
            conn.commit()
            event_id = log_event(conn, "session.create", session_id, agent_id, body)
        return self._send_json(200, {"ok": True, "session_id": session_id, "event_id": event_id})

    def handle_test_session_join(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        session_id = body.get("session_id")
        if not agent_id or not session_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_and_session_id_required"})
        with db_connect() as conn:
            db_cleanup(conn)
            conn.execute(
                """
                INSERT OR IGNORE INTO session_participants (session_id, agent_id, joined_at)
                VALUES (?, ?, ?)
                """,
                (session_id, agent_id, now_ts()),
            )
            conn.execute(
                "UPDATE sessions SET status=?, updated_at=? WHERE session_id=?",
                ("active", now_ts(), session_id),
            )
            conn.commit()
            event_id = log_event(conn, "session.join", session_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})

    def handle_test_session_event(self, parsed):
        body = self._read_json()
        agent_id = body.get("agent_id")
        session_id = body.get("session_id")
        kind = body.get("kind", "event")
        if not agent_id or not session_id:
            return self._send_json(400, {"ok": False, "error": "agent_id_and_session_id_required"})
        with db_connect() as conn:
            db_cleanup(conn)
            event_id = log_event(conn, f"session.{kind}", session_id, agent_id, body)
        return self._send_json(200, {"ok": True, "event_id": event_id})


def run() -> None:
    with db_connect() as conn:
        db_init(conn)
    server = ThreadingHTTPServer((HOST, PORT), AgentHubHandler)
    print(f"[agent-hub] listening on {HOST}:{PORT} db={DB_PATH}")
    server.serve_forever()


if __name__ == "__main__":
    run()
