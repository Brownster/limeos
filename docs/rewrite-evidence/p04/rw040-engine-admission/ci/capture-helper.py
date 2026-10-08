# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture exact-source CI proof under /tmp; never write the repository."""

import argparse
import gzip
import hashlib
import io
import json
import re
import subprocess
import tarfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path

RUN = 37827510060
SOURCE = "31fdb5f47fb680597f8806b41e6fb4bb7d504026"
OUT = Path(f"/home/marc/Documents/github/lime-os/target/engine-admission-ci-{RUN}")
CACHE = OUT / "downloads"
EVIDENCE = OUT / "evidence"
SELECTED = {
    "p01/ci-vm-result.json": "ci-foundation-vm-result.json",
    "p02/ci-vm-result.json": "ci-observations-vm-result.json",
    "p03/ci-reference-vm-result.json": "ci-reference-vm-result.json",
    "p04/ci-approved-vm-result.json": "ci-approved-vm-result.json",
    "p04/ci-dependencies-vm-result.json": "ci-dependencies-vm-result.json",
}
FAILURE_DIAGNOSTICS = {
    "p01/ci-vm-failure.txt": "ci-foundation-vm-failure.txt",
    "p02/ci-vm-failure.txt": "ci-observations-vm-failure.txt",
    "p03/ci-reference-vm-failure.txt": "ci-reference-vm-failure.txt",
    "p04/ci-approved-vm-failure.txt": "ci-approved-vm-failure.txt",
    "p04/ci-dependencies-vm-failure.txt": "ci-dependencies-vm-failure.txt",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write_json(name, data):
    (EVIDENCE / name).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def gh(endpoint):
    return subprocess.check_output(
        ["gh", "api", f"repos/Brownster/limeos/{endpoint}"], timeout=55
    )


def package_record(data, name, artifact):
    path = CACHE / name
    path.write_bytes(data)
    raw_control = subprocess.check_output(
        ["dpkg-deb", "--field", str(path)], text=True, timeout=30
    )
    control = dict(
        line.split(": ", 1) for line in raw_control.splitlines() if ": " in line
    )
    payload = subprocess.check_output(
        ["dpkg-deb", "--fsys-tarfile", str(path)], timeout=30
    )
    binaries = {}
    with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
        for entry in archive:
            binary = entry.name.rsplit("/", 1)[-1]
            if entry.isfile() and binary in {
                "limeos-core", "limeos-executor", "limeosctl", "limeos-password-worker"
            }:
                blob = archive.extractfile(entry).read()
                assert blob[:4] == b"\x7fELF" and blob[4] == 2
                machine = int.from_bytes(
                    blob[18:20], "little" if blob[5] == 1 else "big"
                )
                expected = 183 if control["Architecture"] == "arm64" else 62
                assert machine == expected, (name, machine, expected)
                binaries[binary] = {
                    "path": entry.name,
                    "bytes": len(blob),
                    "sha256": sha(blob),
                    "elf_class": 64,
                    "elf_machine": machine,
                    "elf_endianness": "little" if blob[5] == 1 else "big",
                }
    assert len(binaries) == 4, (name, binaries.keys())
    return {
        "artifact": artifact,
        "file": name,
        "bytes": len(data),
        "sha256": sha(data),
        "control": control,
        "raw_control": raw_control,
        "binaries": binaries,
    }


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--collect", action="store_true", help="Capture completed logs/artifacts")
args = parser.parse_args()
CACHE.mkdir(parents=True, exist_ok=True)
EVIDENCE.mkdir(exist_ok=True)
raw_run = gh(f"actions/runs/{RUN}")
raw_jobs = gh(f"actions/runs/{RUN}/jobs?per_page=100")
raw_artifacts = gh(f"actions/runs/{RUN}/artifacts?per_page=100")
run = json.loads(raw_run)
jobs = json.loads(raw_jobs)["jobs"]
artifacts = json.loads(raw_artifacts)["artifacts"]
assert run["head_sha"] == SOURCE
stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
status = {
    "observed_at": stamp,
    "tested_source": SOURCE,
    "run_id": RUN,
    "run": run,
    "jobs": jobs,
    "artifacts": artifacts,
}
write_json(f"status-{stamp}.json", status)
write_json("ci-complete.json", status)
summary = {
    "tested_source": SOURCE,
    "capture_kind": "rw040-engine-admission",
    "expected_native": {"rust_total": 394, "unit_integration": 392, "compile_fail": 2, "harness": 33},
    "expected_reference": 15,
    "run_id": RUN,
    "run_url": run["html_url"],
    "observed_at": stamp,
    "status": run["status"],
    "conclusion": run["conclusion"],
    "jobs": [],
    "packages": [],
    "installed": [],
    "current_failure_diagnostics": [],
    "excluded_artifact_paths": [],
    "selection": "Only five exact output paths from the tested workflow. A completed successful installed job establishes that all five were freshly overwritten; selected paths from a failed/incomplete job are not qualified as fresh proof.",
    "fresh_installed_results_qualified": False,
    "scope": "Exact Engine-only admission library source CI gates and existing installed suites. Native admission regression tests qualify this source; ordinary public-library probe tests select no host collection. Privileged named probe cases, installed combined resource capacity, Engine/kernel/physical composition, supervision and restore remain pending",
}
for job in jobs:
    active = [s["name"] for s in job["steps"] if s["status"] == "in_progress"]
    print(f"{job['name']}: {job['status']}/{job['conclusion']} {active}", flush=True)
    if not args.collect or job["status"] != "completed":
        continue
    arch = "arm64" if "-arm)" in job["name"] else "amd64" if job["name"].startswith("check (") else "vm"
    raw_path = CACHE / f"{job['id']}.log"
    if not raw_path.exists():
        raw_path.write_bytes(gh(f"actions/jobs/{job['id']}/logs"))
    data = raw_path.read_bytes()
    text = data.decode()
    assert SOURCE in text, (job["name"], "source missing")
    compressed = gzip.compress(data, mtime=0)
    (EVIDENCE / f"ci-{arch}.log.gz").write_bytes(compressed)
    counts = [tuple(map(int, row)) for row in re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", text)]
    summary["jobs"].append({
        "id": job["id"], "name": job["name"], "conclusion": job["conclusion"],
        "raw_log_sha256": sha(data), "raw_log_bytes": len(data),
        "log_gzip_sha256": sha(compressed), "log_file": f"ci-{arch}.log.gz",
        "rust_passed": sum(row[0] for row in counts),
        "rust_unit_integration_passed": sum(row[0] for row in counts) - len(re.findall(r"test[^\n]*compile fail[^\n]*\.\.\. ok", text)),
        "rust_failed": sum(row[1] for row in counts),
        "rust_ignored": sum(row[2] for row in counts),
        "compile_fail_doc_tests": len(re.findall(r"test[^\n]*compile fail[^\n]*\.\.\. ok", text)),
        "unittest_counts": list(map(int, re.findall(r"Ran (\d+) tests", text))),
        "failed_steps": [s["name"] for s in job["steps"] if s["conclusion"] == "failure"],
    })
    print(f"Captured {arch} log: {len(data)} bytes", flush=True)

if args.collect:
    vm_log_text = "\n".join(
        (CACHE / f"{job['id']}.log").read_text()
        for job in jobs
        if job["name"] == "debian-vm" and (CACHE / f"{job['id']}.log").exists()
    )
    for artifact in artifacts:
        if not (artifact["name"].startswith("debian-ubuntu-") or artifact["name"] == "debian-vm-evidence"):
            continue
        assert not artifact["expired"] and artifact["size_in_bytes"] < 64 * 1024 * 1024
        path = CACHE / f"{artifact['id']}.zip"
        if not path.exists():
            path.write_bytes(gh(f"actions/artifacts/{artifact['id']}/zip"))
        data = path.read_bytes()
        origin = artifact["workflow_run"]
        assert origin["id"] == RUN and origin["head_sha"] == SOURCE, origin
        identity = {"id": artifact["id"], "name": artifact["name"], "zip_bytes": len(data), "zip_sha256": sha(data), "api_digest": artifact.get("digest"), "workflow_run": origin}
        assert identity["api_digest"] == "sha256:" + identity["zip_sha256"], identity
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for member in archive.infolist():
                if member.is_dir():
                    continue
                assert member.file_size < 64 * 1024 * 1024
                if artifact["name"].startswith("debian-ubuntu-") and member.filename.endswith(".deb"):
                    summary["packages"].append(package_record(archive.read(member), Path(member.filename).name, identity))
                elif artifact["name"] == "debian-vm-evidence":
                    if member.filename in FAILURE_DIAGNOSTICS and f"Guest failure diagnostics: docs/rewrite-evidence/{member.filename}" in vm_log_text:
                        blob = archive.read(member)
                        filename = FAILURE_DIAGNOSTICS[member.filename]
                        (EVIDENCE / filename).write_bytes(blob)
                        summary["current_failure_diagnostics"].append({
                            "artifact": identity, "artifact_path": member.filename,
                            "file": filename, "bytes": len(blob), "sha256": sha(blob),
                            "freshness_basis": "Exact current job log names this generated diagnostic path",
                        })
                        continue
                    if member.filename not in SELECTED:
                        summary["excluded_artifact_paths"].append(member.filename)
                        continue
                    blob = archive.read(member)
                    parsed = json.loads(blob)
                    check_keys = [key for key in ("checks", "tests", "passed") if key in parsed]
                    assert len(check_keys) == 1, (member.filename, check_keys)
                    check_key = check_keys[0]
                    assert isinstance(parsed[check_key], list) and parsed[check_key]
                    if "result" in parsed:
                        assert parsed["result"] == "pass", member.filename
                    filename = SELECTED[member.filename]
                    (EVIDENCE / filename).write_bytes(blob)
                    summary["installed"].append({
                        "artifact": identity, "artifact_path": member.filename,
                        "file": filename, "bytes": len(blob), "sha256": sha(blob),
                        "check_field": check_key,
                        "checks": len(parsed[check_key]),
                        "package_sha256": parsed.get("package_sha256"),
                        "binary_sha256": parsed.get("binary_sha256"),
                        "image": parsed.get("image"),
                        "result_keys": sorted(parsed),
                    })
        print(f"Captured artifact {artifact['name']}", flush=True)

if run["status"] == "completed" and args.collect:
    assert len(summary["jobs"]) == len(jobs)
    if run["conclusion"] == "success":
        assert len(jobs) == 3 and all(j["conclusion"] == "success" for j in jobs)
        native = [j for j in summary["jobs"] if j["name"].startswith("check (")]
        assert len(native) == 2 and all(j["rust_passed"] == 394 and j["rust_unit_integration_passed"] == 392 and j["compile_fail_doc_tests"] == 2 and j["rust_failed"] == 0 and j["rust_ignored"] == 0 and 33 in j["unittest_counts"] for j in native), native
        assert len(summary["packages"]) == 6
        assert len(summary["installed"]) == 5
        vm_jobs = [j for j in summary["jobs"] if j["name"] == "debian-vm"]
        assert len(vm_jobs) == 1 and 15 in vm_jobs[0]["unittest_counts"]
        packages = {p["file"]: p for p in summary["packages"]}
        for result in summary["installed"]:
            for name, digest in (result["package_sha256"] or {}).items():
                assert packages[name]["sha256"] == digest, name
            for name, digest in (result["binary_sha256"] or {}).items():
                assert any(p["control"]["Architecture"] == "amd64" and p["binaries"][name]["sha256"] == digest for p in summary["packages"]), name
        summary["installed_total_checks"] = sum(r["checks"] for r in summary["installed"])
        images = {json.dumps(r["image"], sort_keys=True) for r in summary["installed"]}
        assert len(images) == 1 and None not in [r["image"] for r in summary["installed"]]
        summary["installed_image"] = summary["installed"][0]["image"]
        assert re.fullmatch(r"[a-f0-9]{128}", summary["installed_image"]["sha512"])
        summary["all_reported_package_binary_hashes_match_native_artifact"] = True
        summary["fresh_installed_results_qualified"] = True
if args.collect:
    write_json("ci-summary.json", summary)
sums = "".join(f"{sha(p.read_bytes())}  {p.name}\n" for p in sorted(EVIDENCE.iterdir()) if p.is_file() and p.name != "SHA256SUMS")
(EVIDENCE / "SHA256SUMS").write_text(sums)
print(json.dumps({"status": summary["status"], "conclusion": summary["conclusion"], "jobs": summary["jobs"], "packages": len(summary["packages"]), "installed": [(r["artifact_path"], r["checks"]) for r in summary["installed"]], "directory": str(EVIDENCE)}, sort_keys=True), flush=True)
