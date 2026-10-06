#!/usr/bin/env python3
"""Repeat P03 against isolated surrogates of the redacted reference mount layout."""

import hashlib
import json
import os
import socket
import sys
from pathlib import Path

import p03_compose_guest as compose
import p03_guest as fixture

PROFILE = Path("/root/wybie-layout.json")
created = []
sentinels = {}
original_docker = fixture.docker
profiles = []


def prepare_sources():
    for service in profiles:
        assert service["application_owner"] == {"puid": 1000, "pgid": 1000}
        for mount in service["mounts"]:
            if mount["kind"] == "volume":
                continue
            assert mount["kind"] == "bind"
            source = Path(mount["source"])
            assert str(source).startswith(("/mnt/", "/home/test-operator/"))
            assert ".." not in source.parts
            source.mkdir(parents=True, exist_ok=True)
            os.chown(source, 1000, 1000)
            source.chmod(0o755)
            sentinel = source / ".limeos-reference-sentinel"
            sentinel.write_text("synthetic reference data; never copied from wybie\n")
            os.chown(sentinel, 1000, 1000)
            sentinel.chmod(0o644)
            sentinels[str(sentinel)] = (
                hashlib.sha256(sentinel.read_bytes()).hexdigest(),
                sentinel.stat().st_uid,
                sentinel.stat().st_gid,
            )


def reference_docker(*args):
    if not args or args[0] != "run":
        return original_docker(*args)
    # Only fixtures using the private static BusyBox image get a profile.
    assert "limeos-p03-fixture:local" in args
    if not created:
        original_docker("network", "create", "limeos-reference-bridge")
        original_docker(
            "run",
            "-d",
            "--name",
            "limeos-reference-vpn-anchor",
            "--network",
            "limeos-reference-bridge",
            "limeos-p03-fixture:local",
            "/bin/busybox",
            "sleep",
            "3600",
        )
    service = profiles[len(created) % len(profiles)]
    options = [
        "--user",
        "1000:1000",
        "--restart",
        service["restart"],
        "--label",
        "com.docker.compose.project=limeos-reference",
        "--label",
        "com.docker.compose.service=" + service["service"],
    ]
    if service["network"].startswith("container:"):
        options.extend(["--network", "container:limeos-reference-vpn-anchor"])
    elif service["network"] == "reference-bridge":
        options.extend(["--network", "limeos-reference-bridge"])
    else:
        assert service["network"] == "bridge"
        options.extend(["--network", "bridge"])
    for index, mount in enumerate(service["mounts"]):
        assert ":" not in mount["source"] and ":" not in mount["target"]
        assert mount["target"] not in ["/", "/bin", "/dev", "/proc", "/sys"]
        source = mount["source"]
        if mount["kind"] == "volume":
            source = f"limeos-reference-{service['service']}-{index}"
            original_docker("volume", "create", source)
            location = Path(
                original_docker(
                    "volume", "inspect", "--format", "{{.Mountpoint}}", source
                )
            )
            assert str(location).startswith("/var/lib/docker/volumes/")
            os.chown(location, 1000, 1000)
        options.extend(
            [
                "--volume",
                source
                + ":"
                + mount["target"]
                + (":ro" if mount["read_only"] else ":rw"),
            ]
        )
    identifier = original_docker("run", *options, *args[1:])
    created.append((identifier, service))
    return identifier


def media_preview():
    # Explicit partial planning snapshot, never an importer or executable stack.
    services = []
    for source in profiles[:2]:
        services.append(
            {
                "name": source["service"],
                "image": "reference.invalid/"
                + source["service"]
                + "@"
                + source["image_id"],
                "user": None,
                "restart": source["restart"],
                "ports": sorted(
                    [
                        {
                            "host_ip": "127.0.0.1",
                            "published": port["published"],
                            "target": port["target"],
                            "protocol": port["protocol"],
                        }
                        for port in source["ports"]
                    ],
                    key=lambda port: (
                        port["published"],
                        port["target"],
                        port["protocol"],
                    ),
                ),
                "mounts": sorted(source["mounts"], key=lambda mount: mount["target"]),
                "devices": [],
                "privileged": False,
                "host_network": False,
            }
        )
    services.sort(key=lambda service: service["name"])
    before = {"services": services}
    desired = json.loads(json.dumps(before))
    jellyfin = next(s for s in desired["services"] if s["name"] == "jellyfin")
    next(m for m in jellyfin["mounts"] if m["target"] == "/media")["read_only"] = True
    catalog = {
        "version": 1,
        "stacks": [
            {
                "id": "reference-media",
                "operator_files": profiles[0]["operator_files"],
                "current": before,
                "templates": [{"id": "read-only-media", "project": desired}],
            }
        ],
    }
    compose.CATALOG.write_text(json.dumps(catalog))
    compose.restart_core()
    _, _, principal = fixture.enroll()
    base = {"operation": "deployment_manage", "resource": "stack:reference-media"}
    cookie, csrf = compose.authority(principal["id"], "administrator", [base])
    selection = {"stack": "reference-media", "template": "read-only-media"}
    assert fixture.http(compose.BASE, "POST", selection, cookie, csrf)[0] == 403
    fingerprint = compose.digest(json.dumps(desired, separators=(",", ":")))
    elevated = {
        "operation": "deployment_elevated",
        "resource": f"stack:reference-media:template:read-only-media:{fingerprint}",
    }
    cookie, csrf = compose.authority(principal["id"], "administrator", [base, elevated])
    status, _, preview = fixture.http(compose.BASE, "POST", selection, cookie, csrf)
    assert status == 201, (status, preview)
    assert preview["plan"]["before"] == before
    assert preview["plan"]["desired"] == desired
    assert preview["plan"]["impact"]["elevated"] == ["host_mount"]
    assert len(preview["plan"]["impact"]["services"]) == 1
    assert preview["plan"]["impact"]["services"][0]["name"] == "jellyfin"
    assert preview["plan"]["operator_files"] == profiles[0]["operator_files"]
    token = compose.task(principal["id"], [base, elevated])
    task = fixture.rpc_as(
        "limeos-assistant",
        {
            "request": "propose_compose",
            "version": 1,
            "token": token,
            "task": "compose-preview",
            "selection": selection,
        },
    )
    assert task["response"] == "compose_plan"
    assert (
        task["plan"]["desired"] == desired
        and task["plan"]["impact"] == preview["plan"]["impact"]
    )
    fixture.passed(
        "reference media preview retains resolved mount paths, source fingerprints and image IDs; exact host-mount grant and task policy agree"
    )
    # The unsupported VPN contract must never silently turn into a normal template.
    unsupported = json.loads(json.dumps(catalog))
    unsupported["stacks"][0]["templates"][0]["project"]["services"][0]["cap_add"] = (
        profiles[-1]["cap_add"]
    )
    compose.invalid_catalog(
        json.dumps(unsupported).encode(),
        "reference VPN capabilities cannot silently enter the limited P03 template schema",
    )
    malformed = json.loads(json.dumps(catalog))
    navidrome = next(s for s in profiles if s["service"] == "navidrome")
    malformed["stacks"][0]["templates"][0]["project"]["services"][0]["mounts"] = (
        navidrome["mounts"]
    )
    compose.invalid_catalog(
        json.dumps(malformed).encode(),
        "reference quoted mount target is reported by strict planning validation, not normalized away",
    )
    compose.CATALOG.write_text(json.dumps(catalog))
    compose.restart_core()


def main(package_version="0.3.2", authority_schema=5, qualify_upgrade=True):
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    profiles.extend(json.loads(PROFILE.read_text())["services"])
    assert [s["service"] for s in profiles] == [
        "jellyfin",
        "audiobookshelf",
        "navidrome",
        "sonarr",
        "radarr",
        "transmission",
        "vpn",
    ]
    prepare_sources()
    fixture.docker = reference_docker
    compose.main(package_version, authority_schema, qualify_upgrade)
    representative = {}
    for identifier, service in created:
        inspected = fixture.inspect(identifier)
        assert inspected["Config"]["User"] == "1000:1000"
        mounts = inspected["Mounts"]
        assert sorted((m["Destination"], not m["RW"]) for m in mounts) == sorted(
            (m["target"], m["read_only"]) for m in service["mounts"]
        )
        assert not inspected["HostConfig"]["Privileged"]
        representative.setdefault(service["service"], (identifier, service))
    assert len(representative) == 7
    fixture.passed(
        "all seven reference layouts retain numeric application ownership and exact mount targets/read modes throughout lifecycle interruption tests"
    )
    for identifier, service in representative.values():
        if not fixture.inspect(identifier)["State"]["Running"]:
            original_docker("start", identifier)
        for mount in service["mounts"]:
            target = mount["target"] + "/.limeos-reference-write-check"
            probe = fixture.run(
                "docker",
                "--host",
                "unix://" + fixture.REAL_SOCKET,
                "exec",
                identifier,
                "/bin/busybox",
                "touch",
                target,
                check=False,
            )
            assert (probe.returncode != 0) == mount["read_only"], (
                service["service"],
                mount,
            )
            if not mount["read_only"]:
                original_docker("exec", identifier, "/bin/busybox", "rm", target)
    fixture.passed(
        "UID/GID 1000 can write each intended bind or volume; the reference music mount remains read-only"
    )
    for name, expected in sentinels.items():
        path = Path(name)
        assert (
            hashlib.sha256(path.read_bytes()).hexdigest(),
            path.stat().st_uid,
            path.stat().st_gid,
        ) == expected
    fixture.passed(
        "all synthetic reference data sentinels and numeric ownership survive the complete mutation matrix"
    )
    media_preview()
    output = Path(sys.argv[2])
    result = json.loads(output.read_text())
    result["checks"] = fixture.checks
    result["container_fixture"] = (
        "static BusyBox surrogates with redacted reference mounts and private bridge/shared namespace; no production images/data"
    )
    result["reference"] = {
        "profile_sha256": hashlib.sha256(PROFILE.read_bytes()).hexdigest(),
        "services": list(representative),
        "profiled_containers": len(created),
        "sentinels_preserved": len(sentinels),
        "application_uid_gid": [1000, 1000],
        "production_environment_values_copied": 0,
        "host": "wybie (read-only capture)",
        "limitations": [
            "Resolved configuration projection, not raw Compose/.env import or application qualification",
            "BusyBox runs as the application UID; production images may initialize as root then drop privileges",
            "Privileged flags, VPN capabilities/devices and published listeners are not enabled in these surrogates",
            "VPN clients share a stable surrogate namespace; dependent-stack mount/network supervision belongs to P04",
            "Anonymous volume is recreated with a test name; no production data or image is fetched",
            "Native ARM64 and reference-hardware measurements remain pending",
        ],
    }
    result["compose"]["additional_reference_preview"] = (
        "partial non-executable media snapshot and explicit rejection of unsupported VPN/quoted-target contracts"
    )
    output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
