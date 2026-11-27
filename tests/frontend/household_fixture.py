"""A closed, synthetic household host for browser tests and screenshots.

No live host is contacted: every `/api/` request is answered here and every
other path is served from the built UI the way Caddy does it
(`try_files {path} /index.html`), so history-API routes and reloads work.
"""

import json
import math
import mimetypes
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[2]
DIST = ROOT / "frontend/dist"


def _container(index, name, status, stack, health=None, cpu=None, memory=None):
    identifier = f"{index:064x}"
    return {
        "id": "container:" + identifier,
        "kind": "container",
        "name": name,
        "status": status,
        "source": "docker",
        "parent": f"stack:{stack}" if stack else None,
        "identity": identifier,
        "filesystem": None,
        "mountpoint": None,
        "size_bytes": None,
        "image": f"lscr.io/linuxserver/{name}:latest",
        "health": health,
        "cpu_percent": cpu,
        "memory_percent": memory,
    }


def _storage(kind, identifier, name, parent=None, mountpoint=None, size=None):
    return {
        "id": identifier,
        "kind": kind,
        "name": name,
        "status": "unmounted" if kind == "disk" else "mounted",
        "source": "pools" if kind == "pool" else "storage",
        "parent": parent,
        "identity": identifier,
        "filesystem": "ext4" if kind == "partition" else None,
        "mountpoint": mountpoint,
        "size_bytes": size,
        "image": None,
        "health": None,
        "cpu_percent": None,
        "memory_percent": None,
    }


def _stack(name, status):
    return {
        "id": f"stack:{name}",
        "kind": "stack",
        "name": name,
        "status": status,
        "source": "docker",
        "parent": None,
        "identity": None,
        "filesystem": None,
        "mountpoint": None,
        "size_bytes": None,
        "image": None,
        "health": None,
        "cpu_percent": None,
        "memory_percent": None,
    }


def resources():
    tb = 1000**4
    return [
        _container(1, "jellyfin", "running", "media", "healthy", 12.4, 18.2),
        _container(2, "sonarr", "running", "media", "healthy", 1.1, 6.3),
        _container(3, "radarr", "running", "media", None, 0.9, 5.8),
        _container(4, "lidarr", "exited", "media"),
        _container(5, "transmission", "running", "downloads", "unhealthy", 4.2, 3.1),
        _container(6, "jackett", "running", "downloads", None, 0.3, 2.2),
        _container(7, "audiobookshelf", "running", None, None, 0.2, 2.9),
        _container(8, "navidrome", "running", None, "healthy", 0.1, 1.4),
        _stack("media", "degraded"),
        _stack("downloads", "running"),
        _storage("disk", "disk:sda", "sda", size=4 * tb),
        _storage("disk", "disk:sdb", "sdb", size=2 * tb),
        _storage("partition", "uuid:media", "sda1", "disk:sda", "/mnt/storage", 4 * tb),
        _storage("partition", "uuid:backup", "sdb1", "disk:sdb", "/mnt/backup", 2 * tb),
        _storage("pool", "pool:/mnt/media", "/mnt/media", None, "/mnt/media", 6 * tb),
    ]


def overview(now=None, stale_docker=False):
    now = int(now or time.time())
    sources = [
        {
            "source": source,
            "state": "fresh",
            "sampled_at": now - 3,
            "max_age_seconds": 90,
            "warnings": [],
        }
        for source in ("host", "storage", "pools", "docker", "history")
    ]
    if stale_docker:
        sources[3].update(state="stale", sampled_at=now - 600)
    return {
        "revision": 7,
        "generated_at": now,
        "host": {
            "cpu_percent": 23.4,
            "memory_percent": 61.2,
            "temperature_celsius": 58.3,
            "disk_percent": 78.9,
            "load_one": 1.42,
            "uptime_seconds": 3 * 86400 + 4 * 3600 + 17 * 60,
        },
        "sources": sources,
        "resources": resources(),
    }


def history(range_="24h", now=None):
    now = int(now or time.time())
    bucket = {"24h": 300, "7d": 3600, "30d": 4 * 3600}[range_]
    span = {"24h": 86400, "7d": 7 * 86400, "30d": 30 * 86400}[range_]
    start = now - span
    points = []
    for index in range(span // bucket):
        at = start + index * bucket
        day = math.sin((at % 86400) / 86400 * 2 * math.pi)
        wobble = math.sin(index * 0.7) * 3
        gap = 0.42 < index / (span // bucket) < 0.45
        points.append(
            {
                "at": at,
                "cpu_percent": None if gap else round(max(1, 18 + 14 * day + wobble), 1),
                "memory_percent": None if gap else round(55 + 6 * day + wobble / 2, 1),
                "temperature_celsius": None if gap else round(52 + 7 * day + wobble / 3, 1),
                "disk_percent": None if gap else round(76 + index / (span // bucket) * 3, 1),
            }
        )
    return {
        "range": range_,
        "from": start,
        "to": now,
        "bucket_seconds": bucket,
        "points": points,
        "legacy_history": "Earlier history from the previous app is kept separately and is not shown here.",
    }


def stacks(now=None):
    now = int(now or time.time())
    value = json.loads(
        (ROOT / "tests/fixtures/deployment/discovery/wire.json").read_text()
    )
    value["generated_at"] = now
    for source in ("catalog", "engine"):
        value[source].update(sampled_at=now, valid_for_millis=90_000)
    return value


def alerts():
    return {
        "items": [
            {
                "key": "container:transmission",
                "kind": "container",
                "last_sequence": 4,
                "incident": {
                    "phase": "open",
                    "generation": 2,
                    "severity": "warning",
                    "summary": "transmission reports unhealthy",
                    "announced": True,
                },
            },
            {
                "key": "mount:/mnt/storage",
                "kind": "mount",
                "last_sequence": 2,
                "incident": {"phase": "clear", "failures": 0},
            },
        ],
        "next": None,
        "partial": False,
    }


def notifications():
    return {
        "items": [
            {
                "id": "a" * 64,
                "revision": 3,
                "event": {
                    "key": "container:transmission",
                    "generation": 2,
                    "event": "incident",
                    "kind": "container",
                    "severity": "warning",
                    "summary": "transmission reports unhealthy",
                    "at": int(time.time()) - 1800,
                },
                "channel": "in_app",
                "delivery": {"state": "delivered", "at": int(time.time()) - 1790},
                "attempts": 1,
                "acknowledged_at": None,
            }
        ],
        "next": None,
        "partial": False,
    }


def schedules():
    return {
        "items": [
            {
                "schedule": {
                    "id": "e" * 64,
                    "revision": 1,
                    "paused": False,
                    "settled_through": None,
                    "definition": {
                        "operation": "observe_health",
                        "anchor": int(time.time()) - 86400,
                        "interval_seconds": 300,
                        "grace_seconds": 60,
                        "missed": "skip",
                        "window": None,
                    },
                },
                "status": "awaiting_producer",
                "last_occurrence": None,
            }
        ],
        "next": None,
        "partial": False,
    }


def policy():
    return {
        "revision": 1,
        "updated": 200,
        "definition": {
            "fail_threshold": 2,
            "disabled": [],
            "required_mounts": ["/mnt/storage"],
            "silences": [],
        },
    }


CSP = (
    "default-src 'self'; script-src 'self'; style-src 'self'; "
    "connect-src 'self'; frame-ancestors 'none'"
)

UNAVAILABLE = {
    "code": "unavailable",
    "message": "The storage helper is not running.",
    "retry": True,
    "audit_id": "fixture-mounts",
}


class Household:
    """Mutable fixture state plus a Playwright route handler."""

    def __init__(self, host, role="administrator", dist=DIST, signed_in=True):
        self.host = host
        self.dist = Path(dist)
        self.calls = []
        self.signed_in = signed_in
        self.stale_docker = False
        self.overview_down = False
        self.session = {
            "principal": {"id": "alex", "role": role, "grant_revision": 1},
            "csrf_token": "a" * 64,
            "expires_at": int(time.time()) + 3600,
        }

    def api(self, path, method, query, body):
        if path == "/api/v1/auth/session":
            if not self.signed_in:
                return 401, {
                    "code": "unauthenticated",
                    "message": "Sign in to continue.",
                    "retry": False,
                    "audit_id": "fixture-session",
                }
            return 200, self.session
        if path == "/api/v1/auth/login" and method == "POST":
            self.signed_in = True
            return 200, self.session
        if path == "/api/v1/auth/logout" and method == "POST":
            self.signed_in = False
            return 204, None
        if path == "/api/v1/overview":
            if self.overview_down:
                return 503, {
                    "code": "unavailable",
                    "message": "Observations unavailable",
                    "retry": True,
                    "audit_id": "fixture-overview",
                }
            return 200, overview(stale_docker=self.stale_docker)
        if path == "/api/v1/observations/stream":
            return 503, {
                "code": "unavailable",
                "message": "No stream in this fixture",
                "retry": True,
                "audit_id": "fixture-stream",
            }
        if path == "/api/v1/system/history":
            return 200, history(query.get("range", ["24h"])[0])
        if path == "/api/v1/deployment/stacks":
            return 200, stacks()
        if path in ("/api/v1/container/jobs", "/api/v1/stack/jobs"):
            return 200, []
        if path == "/api/v1/jobs/needs-attention":
            return 200, []
        if path.startswith("/api/v1/storage/mounts/"):
            return 503, UNAVAILABLE
        if path == "/api/v1/automation/alerts":
            return 200, alerts()
        if path == "/api/v1/automation/notifications":
            return 200, notifications()
        if path == "/api/v1/automation/schedules":
            return 200, schedules()
        if path == "/api/v1/automation/health-policy":
            return 200, policy()
        raise AssertionError(f"Unexpected fixture request: {method} {path}")

    def respond(self, route):
        request = route.request
        url = urlsplit(request.url)
        assert url.netloc == self.host, request.url
        path = url.path
        if path.startswith("/api/"):
            body = request.post_data_json if request.post_data else None
            self.calls.append((path, request.method))
            status, data = self.api(path, request.method, parse_qs(url.query), body)
            route.fulfill(
                status=status,
                content_type="application/json",
                body="" if data is None else json.dumps(data),
            )
            return
        # Caddy: try_files {path} /index.html
        asset = (self.dist / path.lstrip("/")).resolve()
        assert asset.is_relative_to(self.dist.resolve())
        if not asset.is_file():
            asset = self.dist / "index.html"
        route.fulfill(
            status=200,
            content_type=mimetypes.guess_type(str(asset))[0]
            or "application/octet-stream",
            # The production policy from packaging/Caddyfile.cutover.example.
            headers={"Content-Security-Policy": CSP},
            body=asset.read_bytes(),
        )
