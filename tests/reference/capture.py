#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Print a whitelist-only reference layout; run over SSH stdin without installing it.

Reads seven representative containers once. Never emits environment values,
commands, raw labels, container IDs or operator file contents. Numeric PUID/PGID
are the only environment-derived values. Does not query stats or write the host.
"""

import hashlib
import json
import re
import socket
import subprocess
from datetime import datetime, timezone
from pathlib import Path

SERVICES = [
    "jellyfin",
    "audiobookshelf",
    "navidrome",
    "sonarr",
    "radarr",
    "transmission",
    "vpn",
]


def literal_path(value):
    if (
        not value.startswith("/")
        or len(value) > 256
        or any(ord(char) < 32 or ord(char) == 127 for char in value)
    ):
        raise ValueError("Reference path needs explicit redaction")
    if any(part in [".", ".."] for part in value.split("/")):
        raise ValueError("Noncanonical reference path")
    return value


def redact_source(value, index):
    value = literal_path(value)
    if value in ["/var/run/docker.sock", "/run/docker.sock"]:
        return value
    if value.startswith("/home/"):
        return "/home/test-operator/" + value.split("/", 3)[3]
    if value.startswith(("/mnt/", "/media/")):
        return value
    return f"/srv/reference/bind-{index:02d}"


def main():
    raw = subprocess.check_output(
        ["docker", "inspect", *SERVICES], timeout=10, stderr=subprocess.DEVNULL
    )
    records = json.loads(raw)
    result = []
    for index, record in enumerate(records):
        config, host = record["Config"], record["HostConfig"]
        env = dict(item.split("=", 1) for item in config.get("Env", []) if "=" in item)
        app_user = {
            key.lower(): int(env[key])
            for key in ["PUID", "PGID"]
            if key in env and re.fullmatch(r"[0-9]{1,9}", env[key])
        }
        user = config.get("User", "")
        if not re.fullmatch(r"[0-9]*(:[0-9]+)?", user):
            user = "named-user-omitted"
        network = host.get("NetworkMode", "default")
        if network.startswith("container:"):
            network = "container:<reference-vpn>"
        elif network not in ["default", "bridge", "host", "none"]:
            network = "reference-bridge"
        files = []
        labels = config.get("Labels") or {}
        for source in labels.get("com.docker.compose.project.config_files", "").split(
            ","
        ):
            if not source:
                continue
            path = Path(source)
            if path.name not in [
                "compose.yaml",
                "compose.yml",
                "docker-compose.yaml",
                "docker-compose.yml",
            ]:
                raise ValueError("Unexpected operator Compose filename")
            with path.open("rb") as stream:
                content = stream.read(1024 * 1024 + 1)
            if len(content) > 1024 * 1024:
                raise ValueError("Operator Compose file too large")
            files.append(
                {"name": path.name, "sha256": hashlib.sha256(content).hexdigest()}
            )
        mounts = []
        for number, mount in enumerate(record.get("Mounts", [])):
            mounts.append(
                {
                    "kind": mount["Type"],
                    "source": redact_source(mount["Source"], index * 16 + number),
                    "target": literal_path(mount["Destination"]),
                    "read_only": not mount["RW"],
                }
            )
        ports = []
        for internal, bindings in (host.get("PortBindings") or {}).items():
            target, protocol = internal.split("/")
            for binding in bindings or []:
                ports.append(
                    {
                        "target": int(target),
                        "published": int(binding["HostPort"]),
                        "protocol": protocol,
                    }
                )
        result.append(
            {
                "service": SERVICES[index],
                "running": record["State"]["Running"],
                "image_id": record["Image"],
                "runtime_user": user,
                "application_owner": app_user,
                "restart": host["RestartPolicy"]["Name"],
                "network": network,
                "privileged": host["Privileged"],
                "cap_add": host.get("CapAdd") or [],
                "devices": [
                    {
                        "source": literal_path(d["PathOnHost"]),
                        "target": literal_path(d["PathInContainer"]),
                    }
                    for d in host.get("Devices") or []
                ],
                "mounts": mounts,
                "ports": ports,
                "operator_files": files,
                "environment_values_omitted": len(env) - len(app_user),
            }
        )
    print(
        json.dumps(
            {
                "version": 1,
                "captured_at_utc": datetime.now(timezone.utc).isoformat(),
                "host": socket.gethostname(),
                "projection": "resolved Docker layout; secrets, commands, IDs and raw Compose omitted",
                "services": result,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
