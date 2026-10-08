#!/usr/bin/env python3
"""Derive the evidence summary from one raw guest run, without interpretation.

python3 summarize.py guest-run > summary.json
"""

import json
import sys
from pathlib import Path


def brief(outcome: dict) -> str:
    return "ok" if outcome.get("ok") else outcome.get("name", outcome.get("error", "?"))


def main() -> None:
    run = Path(sys.argv[1])
    record = json.loads((run / "run.json").read_text())
    guest = run / "guest"
    load = lambda name: json.loads((guest / name).read_text())  # noqa: E731
    environment = load("environment/baseline.json")
    access = {}
    for mode, result in environment.items():
        if not isinstance(result, dict) or not result.get("report"):
            continue
        report = result["report"]
        access[mode] = {
            "removed_settings": result.get("removed", []),
            "uid_gid": [
                report["status"]["Uid"].split()[0],
                report["status"]["Gid"].split()[0],
            ],
            "cap_eff": report["status"]["CapEff"],
            "no_new_privs": report["status"]["NoNewPrivs"],
            "seccomp": report["status"]["Seccomp"],
            "rlimit_nofile": report["rlimit_nofile"],
            "cgroup": {
                k: (v["value"] if isinstance(v, dict) and v.get("ok") else v)
                for k, v in report["cgroup"].items()
                if k != "cgroup"
            },
            "openat2": brief(report["openat2_proc"]),
            "openat2_var_run_without_symlinks": brief(report["openat2_var_run_docker"]),
            "statx_mnt_id": brief(report["statx_root"]),
            "af_inet": brief(report["inet_socket"]),
            "docker_canonical": report["docker_canonical"].get("value")
            or brief(report["docker_canonical"]),
            "targets": {
                label: {
                    key: brief(value)
                    for key, value in facts.items()
                    if key not in ("pid", "status_owner_uid")
                }
                for label, facts in [
                    ("pid1", report["pid1"]),
                    *sorted(report["targets"].items()),
                ]
            },
        }
    selfcheck = {
        name: {
            "exit": v["exit"],
            "last": v["stdout_last"],
            "sampler_samples": v["sampler"]["samples"],
            "peak": v["sampler"]["peak"],
            "unit_after": v["unit_after"],
        }
        for name, v in load("selfcheck/ceilings.json").items()
        if name != "label"
    }
    transitions = {
        k: {"verified": v.get("verified"), "error": v.get("error")}
        for k, v in load("transitions/verification.json").items()
        if k != "label"
    }
    complete = load("ground-truth/complete.json")
    engine = load("ground-truth/engine.json")
    summary = {
        "run_id": record["run_id"],
        "source_commit": record["source_commit"],
        "source_dirty": record["source_dirty"],
        "outcome": record["outcome"],
        "guest_seconds": record["guest_seconds"],
        "deadline_reached": record["deadline_reached"],
        "teardown": record["teardown"],
        "teardown_actions": record["teardown_actions"],
        "qemu_exit": record["qemu_exit"],
        "image_sha512": record["image"]["sha512"],
        "inputs_tar_sha256": record["inputs_tar_sha256"],
        "stages": record["guest_stages"],
        "platform": {
            k: load("platform.json")[k]
            for k in (
                "page_size",
                "systemd",
                "ptrace_scope",
                "suid_dumpable",
                "virtualization",
            )
        }
        | {"kernel": load("platform.json")["uname"]},
        "confinement_equivalent": load("confinement-equivalence.json")["equivalent"],
        "compared_properties": len(
            load("confinement-equivalence.json")["compared_properties"]
        ),
        "engine": {
            "var_run": engine["var_run"],
            "socket": engine["socket"],
            "peer": engine["peer_canonical"],
            "activation": engine["activation"],
            "daemon_main_pid": engine["daemon"]["MainPID"],
            "server_version": engine["version"]["Version"],
        },
        "membership_complete": {
            "count": complete["membership"]["count"],
            "running": complete["membership"]["running"],
        },
        "nondumpable_checks": {
            n: p["nondumpable_indicator"]
            for n, p in {**complete["processes"], **complete["host_tasks"]}.items()
        },
        "access": access,
        "selfcheck": selfcheck,
        "transitions": transitions,
        "budget": {
            k: v
            for k, v in load("budget/feasibility.json").items()
            if k != "membership_sha256"
        },
        "cases": {c["id"]: c["status"] for c in load("cases.json")["cases"]},
    }
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
