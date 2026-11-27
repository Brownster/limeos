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


def _storage(kind, identifier, name, parent=None, mountpoint=None, size=None, filesystem=None):
    if kind == "partition" and filesystem is None:
        filesystem = "ext4"
    if kind == "pool":
        filesystem = "fuse.mergerfs"
    return {
        "id": identifier,
        "kind": kind,
        "name": name,
        "status": "mounted" if mountpoint else "unmounted",
        "source": "pools" if kind == "pool" else "storage",
        "parent": parent,
        "identity": identifier,
        "filesystem": filesystem,
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
    ] + storage_resources()


# Filesystem UUIDs and disk serials shared by the overview and the storage inventory.
UUID = {
    "boot": "4A1B-2C3D",
    "root": "7d1f3a52-9c1e-4b8e-a0f4-2f6c1d9e8b10",
    "media": "a1b2c3d4-0e5f-4a6b-9c7d-8e9f0a1b2c3d",
    "backup": "5e6f7a8b-1c2d-4e3f-8a9b-0c1d2e3f4a5b",
    "spare": "c0ffee00-1234-4abc-9def-001122334455",
}
SERIAL = {"sd": "0x8d3e2f1a", "sda": "WD-WX12A3456789", "sdb": "ZA1B2C3D", "sdc": "S5H2NS0N123456"}


def storage_resources():
    """A Pi 5 with its SD card, two data disks, a new unmounted disk and a pool.
    Disks are identified by serial and filesystems by UUID, as the observer
    does; pools carry no size, as on a real host."""
    tb, gb = 1000**4, 1000**3

    def disk(key, name, size):
        return _storage("disk", f"serial:{SERIAL[key]}", name, size=size)

    def part(key, name, parent, mount, size, filesystem=None):
        return _storage(
            "partition", f"uuid:{UUID[key]}", name, f"serial:{SERIAL[parent]}", mount, size, filesystem
        )

    return [
        disk("sd", "mmcblk0", 64 * gb),
        part("boot", "mmcblk0p1", "sd", "/boot/firmware", 536_870_912, "vfat"),
        part("root", "mmcblk0p2", "sd", "/", 63 * gb),
        disk("sda", "sda", 4 * tb),
        part("media", "sda1", "sda", "/mnt/storage", 4 * tb),
        disk("sdb", "sdb", 2 * tb),
        part("backup", "sdb1", "sdb", "/mnt/backup", 2 * tb),
        disk("sdc", "sdc", 1 * tb),
        part("spare", "sdc1", "sdc", None, 1 * tb),
        _storage("pool", "pool:/mnt/media", "/mnt/media", None, "/mnt/media", None),
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


def _member(index, running, image="a"):
    return {
        "container": {
            "resource": f"container:{index:064x}",
            "image": "sha256:" + image * 64,
            "started_at": "2026-11-24T06:12:00Z",
            "running": running,
        },
        "one_off": False,
    }


def _service(name, members):
    running = sum(1 for m in members if m["container"]["running"])
    counts = {"ordinary": len(members), "running": running, "one_offs": 0, "one_off_running": 0}
    runtime = "running" if running == len(members) else "stopped" if running == 0 else "partial"
    return {"name": name, "declared": True, "runtime": runtime, "counts": counts, "members": members}


def _registered(name, services):
    ordinary = sum(s["counts"]["ordinary"] for s in services)
    running = sum(s["counts"]["running"] for s in services)
    runtime = "running" if running == ordinary else "stopped" if running == 0 else "partial"
    return {
        "stack": name,
        "project": name,
        "working_directory": f"/opt/stacks/{name}",
        "config_files": [f"/opt/stacks/{name}/compose.yaml"],
        "files": {"state": "not_inspected"},
        "runtime": runtime,
        "counts": {"ordinary": ordinary, "running": running, "one_offs": 0, "one_off_running": 0},
        "missing_services": 0,
        "services": services,
        "issues": [],
    }


def household_stacks(now=None, reader_down=False):
    """The stack inventory for the household's containers above (same IDs),
    or what core answers while the stack reader is not running."""
    now = int(now or time.time())
    if reader_down:
        source = {"state": "unavailable", "sampled_at": None, "max_age_seconds": 90, "valid_for_millis": 0}
        return {
            "version": 1, "generated_at": now, "catalog": dict(source), "engine": dict(source),
            "engine_matches_catalog": None, "stacks": [], "omitted_containers": [], "other_projects": [],
            "standalone_containers": None, "truncated_containers": None, "unattributed_containers": None,
        }
    source = {"state": "fresh", "sampled_at": now, "max_age_seconds": 90, "valid_for_millis": 90_000}
    media = _registered("media", [
        _service("jellyfin", [_member(1, True, "1")]),
        _service("sonarr", [_member(2, True, "2")]),
        _service("radarr", [_member(3, True, "3")]),
        _service("lidarr", [_member(4, False, "4")]),
    ])
    downloads = _registered("downloads", [
        _service("transmission", [_member(5, True, "5")]),
        _service("jackett", [_member(6, True, "6")]),
    ])
    return {
        "version": 1, "generated_at": now, "catalog": dict(source), "engine": dict(source),
        "engine_matches_catalog": True, "stacks": [downloads, media], "omitted_containers": [],
        "other_projects": [], "standalone_containers": 2, "truncated_containers": 0,
        "unattributed_containers": 0,
    }


def storage_inventory():
    """GET /storage/inventory for an administrator: the same filesystems with
    serials, every mount and the /etc/fstab entries (backup is LimeOS-managed)."""

    def device(major, minor, key, fs, serial, mounts, boot=False):
        return {
            "device": {"major": major, "minor": minor},
            "filesystem_uuid": UUID[key],
            "filesystem": fs,
            "serial": SERIAL[serial],
            "boot_backing": boot,
            "in_use_as_swap": False,
            "mounts": [
                {"mountpoint": m, "mount_id": 20 + i + minor, "filesystem_root": "/", "filesystem": fs, "writable": True}
                for i, m in enumerate(mounts)
            ],
        }

    def fstab(key, mount, managed=False):
        return {"filesystem_uuid": UUID[key], "device": None, "mountpoint": mount, "managed": managed}

    return {
        "inventory": {
            "host_root_mount_id": 1,
            "topology_digest": "1" * 64,
            "mounts_digest": "2" * 64,
            "fstab_digest": "3" * 64,
            "fstab_entries": [
                fstab("boot", "/boot/firmware"),
                fstab("root", "/"),
                fstab("media", "/mnt/storage"),
                fstab("backup", "/mnt/backup", managed=True),
            ],
            "devices": [
                device(179, 1, "boot", "vfat", "sd", ["/boot/firmware"], boot=True),
                device(179, 2, "root", "ext4", "sd", ["/"], boot=True),
                device(8, 1, "media", "ext4", "sda", ["/mnt/storage", "/srv/media"]),
                device(8, 17, "backup", "ext4", "sdb", ["/mnt/backup"]),
                device(8, 33, "spare", "ext4", "sdc", []),
            ],
        },
        "digest": "4" * 64,
    }


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
        # Stack inventory: "household" (default), "wire" (the Rust-bound
        # fixture), "reader_down" or "error".
        self.stacks_mode = "household"
        # Storage inventory: "ok", "denied" (403) or "down" (503).
        self.storage_inventory = "ok"
        # Whole-stack jobs queued through the fixture, newest first.
        # `job_state` is what the newest reports until it succeeds or fails;
        # `job_receipt` gives a running job a partial receipt.
        self.stack_jobs = []
        self.job_state = "running"
        self.job_receipt = False
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
            if self.stacks_mode == "error":
                return 503, {
                    "code": "unavailable",
                    "message": "Stack inventory is unavailable.",
                    "retry": True,
                    "audit_id": "fixture-stacks",
                }
            if self.stacks_mode == "wire":
                return 200, stacks()
            return 200, household_stacks(reader_down=self.stacks_mode == "reader_down")
        if path == "/api/v1/container/jobs":
            return 200, []
        if path.startswith("/api/v1/stack/"):
            return self.stack_api(path, method, body)
        if path == "/api/v1/jobs/needs-attention":
            return 200, []
        if path == "/api/v1/storage/inventory":
            assert method == "GET"
            if self.storage_inventory == "denied":
                return 403, {"code": "forbidden", "message": "Forbidden", "retry": False, "audit_id": "fixture-storage"}
            if self.storage_inventory == "down":
                return 503, {
                    "code": "unavailable",
                    "message": "The storage reader is not running.",
                    "retry": True,
                    "audit_id": "fixture-storage",
                }
            return 200, storage_inventory()
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

    def stack_api(self, path, method, body):
        """Whole-stack plan, approval, queue, job list and progress."""
        now = int(time.time())
        if path == "/api/v1/stack/plans" and method == "POST":
            assert body["operation"] in ("start", "stop", "restart")
            inventory = household_stacks(now)
            stack = next(s for s in inventory["stacks"] if s["project"] == body["project"])
            members = [m["container"] for svc in stack["services"] for m in svc["members"]]
            plan = {
                "id": f"{len(self.stack_jobs) + 1:064x}",
                "version": 1,
                "operation": body["operation"],
                "principal": self.session["principal"]["id"],
                "grant_revision": 1,
                "catalog_stack": stack["stack"],
                "expected": {"resource": f"stack:{stack['project']}", "members": members},
                "created_at": now,
                "expires_at": now + 300,
            }
            return 201, {"plan": plan, "digest": "d" * 64}
        if path.endswith("/approval") and method == "POST":
            return 200, {"token": "f" * 64, "expires_at": now + 300}
        if path == "/api/v1/stack/jobs" and method == "POST":
            plan = body["proposal"]["plan"]
            job = {"id": "9" * 56 + f"{len(self.stack_jobs) + 1:08x}", "state": "queued", "plan": plan}
            self.stack_jobs.insert(0, job)
            return 201, job
        if path == "/api/v1/stack/jobs" and method == "GET":
            for job in self.stack_jobs[:1]:
                if job["state"] not in ("succeeded", "failed"):
                    job["state"] = self.job_state
            return 200, self.stack_jobs
        if path.startswith("/api/v1/stack/jobs/") and method == "GET":
            job_id = path.rsplit("/", 1)[1]
            job = next(j for j in self.stack_jobs if j["id"] == job_id)
            receipt = None
            if self.job_receipt or job["state"] in ("succeeded", "failed"):
                operation = job["plan"]["operation"]
                targets = [m for m in job["plan"]["expected"]["members"] if m["running"] != (operation == "start")]
                done = targets if job["state"] == "succeeded" else targets[:1]
                receipt = {
                    "action": job["id"],
                    "plan_digest": "d" * 64,
                    "operation": operation,
                    "before": job["plan"]["expected"],
                    "state": "verified" if job["state"] == "succeeded" else "effect_accepted",
                    "members": [{"resource": m["resource"], "state": "verified", "error": None} for m in done],
                    "error": None,
                }
            return 200, {"job": job, "events": [], "receipt": receipt}
        raise AssertionError(f"Unexpected stack request: {method} {path}")

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
