#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture selected read semantics from frozen source with synthetic adapters.

Never starts the reference application, reads its state, or opens Docker.
"""

import argparse
import ast
import hashlib
import json
import tempfile
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]


def load(path: Path, namespace: dict, excluded: set[str] = frozenset()) -> dict:
    tree = ast.parse(path.read_text())
    tree.body = [
        node
        for node in tree.body
        if not (isinstance(node, ast.ImportFrom) and node.module in excluded)
    ]
    # Execute only the operator's frozen local source, with synthetic adapters.
    exec(compile(tree, str(path), "exec"), namespace)  # noqa: S102
    return namespace


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--reference", type=Path, default=Path("/home/marc/Documents/github/pi-health")
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "tests/fixtures/read-semantics.json"
    )
    args = parser.parse_args()
    block = {
        "blockdevices": [
            {
                "name": "sda",
                "path": "/dev/sda",
                "type": "disk",
                "size": 100000,
                "serial": "drive-1",
                "children": [
                    {
                        "name": "sda1",
                        "path": "/dev/sda1",
                        "type": "part",
                        "size": 99000,
                        "uuid": "data-1",
                        "fstype": "ext4",
                        "mountpoints": ["/mnt/data"],
                    }
                ],
            },
            {"name": "loop0", "type": "loop"},
        ]
    }
    ns = load(
        args.reference / "disk_inventory_service.py", {"HelperPort": object}, {"ports"}
    )
    disk = ns["process_device"](
        block["blockdevices"][0],
        {"/dev/sda1": {"UUID": "data-1", "TYPE": "ext4"}},
        {"/dev/sda1": {"mountpoint": "/mnt/data", "options": "rw"}},
        {},
        {},
        {},
    )
    containers = [
        {
            "Id": "a" * 64,
            "Names": ["/jellyfin"],
            "Image": "jellyfin:stable",
            "State": "running",
            "Status": "Up (healthy)",
            "Labels": {"com.docker.compose.project": "media"},
        },
        {
            "Id": "b" * 64,
            "Names": ["/sonarr"],
            "Image": "sonarr:stable",
            "State": "exited",
            "Status": "Exited (0)",
            "Labels": {"com.docker.compose.project": "media"},
        },
    ]
    helpers = {
        "DockerPort": object,
        "analyze_network_topology": lambda _: ({}, []),
        "get_container_ports_cached": lambda *_: [],
        "get_container_web_metadata": lambda _: {},
        "inherit_ports_from_network_service": lambda *_args, **_kwargs: [],
    }
    ns = load(
        args.reference / "container_inventory_service.py",
        helpers,
        {"container_helpers", "ports"},
    )
    synthetic = [
        SimpleNamespace(
            id=c["Id"],
            name=c["Names"][0][1:],
            status=c["State"],
            image=SimpleNamespace(tags=[]),
            attrs={
                "Config": {"Image": c["Image"], "Labels": c["Labels"]},
                "State": {
                    "Health": {"Status": "healthy"} if c["State"] == "running" else {}
                },
            },
        )
        for c in containers
    ]
    adapter = SimpleNamespace(available=True, list_containers=lambda **_: synthetic)
    old = ns["ContainerInventoryService"](
        docker=adapter, stats_reader=lambda _: None, update_reader=lambda _: False
    ).list_containers(include_stats=False)
    expected_containers = [
        {
            "id_prefix": c["id"],
            "name": c["name"],
            "status": c["status"],
            "image": c["image"],
            "health": c["health"],
            "stack": c["stack"],
        }
        for c in old
    ]
    at = 2_000_000_000
    ns = load(args.reference / "metric_history.py", {})
    with tempfile.TemporaryDirectory(prefix="limeos-reference-fixture-") as tmp:
        store = ns["MetricHistoryStore"](Path(tmp) / "history.sqlite", clock=lambda: at)
        for time, cpu, mem in [
            (at - 600, 20, None),
            (at - 540, 40, 80),
            (at, None, None),
        ]:
            store.record(
                {"cpu_usage_percent": cpu, "memory_usage": {"percent": mem}},
                sampled_at=time,
            )
        history = store.query("24h")
    sources = [
        "disk_inventory_service.py",
        "container_inventory_service.py",
        "metric_history.py",
    ]
    evidence = {
        "reference_sources": {
            name: hashlib.sha256((args.reference / name).read_bytes()).hexdigest()
            for name in sources
        },
        "lsblk": block,
        "docker": containers,
        "expected": {
            "disk": {
                "serial": disk["serial"],
                "partition_uuid": disk["partitions"][0]["uuid"],
                "partition_mounted": disk["partitions"][0]["mounted"],
                "partition_mountpoint": disk["partitions"][0]["mountpoint"],
            },
            "containers": expected_containers,
            "history": history,
        },
        "history_at": at,
    }
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
