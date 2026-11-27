"""The household host with working container operations, logs and mounts.

Extends tests/frontend/household_fixture.py (left unchanged for the other
suites): eleven containers with varied images, a job queue that advances
one state per read and applies its effect to the overview, a log tail that
honours `tail` and the 24 KiB truncation, and the storage-dependency read.
No live host is contacted.
"""

import json
import time

from household_fixture import Household, _container, overview as household_overview

LOG_LINE = "2026-11-27T09:{m:02d}:{s:02d}Z INFO [{name}] request {i} served in {ms} ms"


def containers():
    rows = [
        _container(1, "jellyfin", "running", "media", "healthy", 12.4, 18.2),
        _container(2, "sonarr", "running", "media", "healthy", 1.1, 6.3),
        _container(3, "radarr", "running", "media", None, 0.9, 5.8),
        _container(4, "lidarr", "exited", "media"),
        _container(5, "transmission", "running", "downloads", "unhealthy", 4.2, 3.1),
        _container(6, "jackett", "running", "downloads", None, 0.3, 2.2),
        _container(9, "prowlarr", "running", "downloads", "starting", 91.0, 74.5),
        _container(7, "audiobookshelf", "running", None, None, 0.2, 2.9),
        _container(8, "navidrome", "running", None, "healthy", 0.1, 1.4),
        _container(10, "watchtower", "exited", None),
        _container(11, "photo-import", "created", None),
    ]
    rows[0]["image"] = "jellyfin/jellyfin:10.10.3"
    rows[6]["image"] = "ghcr.io/linuxserver/prowlarr:1.28.2@sha256:" + "4" * 64
    rows[9]["image"] = "containrrr/watchtower"
    rows[10]["image"] = "sha256:" + "9" * 64
    return rows


class ContainersHousehold(Household):
    def __init__(self, host, role="administrator", **kwargs):
        super().__init__(host, role=role, **kwargs)
        self.containers = containers()
        self.jobs = []
        self.queued = {}
        self.plans = 0
        self.log_calls = []
        self.storage_calls = 0
        self.overview_calls = 0
        self.hold_jobs = False

    def resource(self, identifier):
        return next(c for c in self.containers if c["id"] == identifier)

    def overview(self):
        value = household_overview(stale_docker=self.stale_docker)
        others = [r for r in value["resources"] if r["kind"] != "container"]
        # A stale source keeps its last readings; the UI must not show them as current.
        value["resources"] = json.loads(json.dumps(self.containers)) + others
        return value

    def advance(self):
        """Each jobs read moves every unfinished job one state on and applies its effect."""
        if self.hold_jobs:
            return
        for job in self.jobs:
            if job["state"] == "queued":
                job["state"] = "running"
            elif job["state"] == "running":
                job["state"] = "succeeded"
                target = self.resource(job["plan"]["expected"]["resource"])
                operation = job["plan"].get("operation", "restart")
                target["status"] = "exited" if operation == "stop" else "running"
                if operation == "stop":
                    target["cpu_percent"] = target["memory_percent"] = None
                    target["health"] = None

    def proposal(self, body):
        self.plans += 1
        target = self.resource(body["resource"])
        plan = {
            "id": f"{self.plans:064x}",
            "version": 1,
            "principal": "alex",
            "grant_revision": 1,
            "expected": {
                "resource": target["id"],
                "image": "sha256:" + "d" * 64,
                "running": target["status"] == "running",
                "started_at": "2026-11-24T07:00:00Z",
            },
            "created_at": int(time.time()),
            "expires_at": int(time.time()) + 300,
        }
        if body.get("operation", "restart") != "restart":
            plan["operation"] = body["operation"]
        return {"plan": plan, "digest": "e" * 64}

    def logs(self, identifier, tail):
        name = self.resource("container:" + identifier)["name"]
        available = 140 if name != "lidarr" else 3
        count = min(tail, available)
        lines = [
            LOG_LINE.format(m=i // 60 % 60, s=i % 60, name=name, i=i, ms=(i * 37) % 900)
            for i in range(available - count, available)
        ]
        if name == "jellyfin":
            lines.insert(len(lines) // 2, "[credential-bearing line redacted]")
        text = "\n".join(lines) + "\n"
        truncated = tail == 200 and name == "jellyfin"
        return {"resource": "container:" + identifier, "text": text, "truncated": truncated}

    def storage(self):
        now = int(time.time())
        consumers = []
        for c in self.containers:
            mounts = []
            if c["parent"] == "stack:media":
                mounts = [
                    {"source": {"kind": "bind", "path": f"/srv/config/{c['name']}"}, "destination": "/config", "writable": True},
                    {"source": {"kind": "bind", "path": "/mnt/media"}, "destination": "/data/media", "writable": c["name"] != "jellyfin"},
                ]
            elif c["name"] == "navidrome":
                mounts = [
                    {"source": {"kind": "volume", "path": "/var/lib/docker/volumes/navidrome_data/_data", "name": "navidrome_data", "driver": "local"}, "destination": "/data", "writable": True},
                ]
            consumers.append({
                "container": {"resource": c["id"], "image": "sha256:" + "d" * 64, "started_at": "2026-11-24T07:00:00.123456789Z", "running": c["status"] == "running"},
                "mounts": mounts,
            })
        return {"inventory": {"version": 1, "engine_id": "fixture-engine", "observed_at": now, "containers": consumers}, "digest": "f" * 64}

    def api(self, path, method, query, body):
        if path == "/api/v1/overview":
            self.overview_calls += 1
            if self.overview_down:
                return super().api(path, method, query, body)
            return 200, self.overview()
        if path == "/api/v1/container/plans" and method == "POST":
            return 201, self.proposal(body)
        if path.startswith("/api/v1/container/plans/") and path.endswith("/approval"):
            return 200, {"token": "f" * 64, "expires_at": int(time.time()) + 300}
        if path == "/api/v1/container/jobs" and method == "POST":
            key = body["idempotency_key"]
            if key not in self.queued:
                self.queued[key] = {"id": f"{len(self.jobs) + 1:064x}", "state": "queued", "plan": body["proposal"]["plan"]}
                self.jobs.append(self.queued[key])
            return 202, self.queued[key]
        if path == "/api/v1/container/jobs":
            self.advance()
            return 200, list(reversed(json.loads(json.dumps(self.jobs))))
        if path.startswith("/api/v1/container/jobs/") and path.endswith("/cancel"):
            job = next(j for j in self.jobs if path.split("/")[5] == j["id"])
            job["state"] = "canceled"
            return 204, None
        if path.startswith("/api/v1/container/jobs/"):
            job = next(j for j in self.jobs if path.split("/")[5] == j["id"])
            return 200, {"job": job, "events": [{"event": {"kind": "job_transition", "state": s}} for s in ("queued", "running", job["state"])]}
        if path.startswith("/api/v1/containers/") and path.endswith("/logs"):
            tail = int(query.get("tail", ["100"])[0])
            assert 1 <= tail <= 200, tail
            self.log_calls.append(tail)
            return 200, self.logs(path.split("/")[4], tail)
        if path == "/api/v1/storage/container-dependencies":
            assert method == "GET" and not query
            self.storage_calls += 1
            if self.session["principal"]["role"] != "administrator":
                return 403, {"code": "forbidden", "message": "Not permitted", "retry": False, "audit_id": "fixture-storage"}
            return 200, self.storage()
        return super().api(path, method, query, body)
