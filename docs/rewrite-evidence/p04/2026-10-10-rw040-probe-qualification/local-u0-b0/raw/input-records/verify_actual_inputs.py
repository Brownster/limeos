"""Read-only verification of the one actual, source-bound CI supply; no launch."""
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import stat
import subprocess
import sys
import zipfile

SOURCE = "46f7daadcc5d930ca97925d96f05baddb20e1b2f"
RUN = 38076433426
ROOT = Path("/tmp/limeos-rw040-u0-b0-46f7")
COLLECTOR = Path("/home/marc/Documents/github/lime-os/target/rw040-ci-38076433426")
OUTPUT = Path(__file__).resolve().parent
EXPECTED = {
    "debian-ubuntu-24.04": {"limeos_0.4.4_amd64.deb", "limeos_0.4.4+ci.1_amd64.deb", "limeos-shadow_0.4.4_amd64.deb"},
    "rw040-public-library-probe": {"manifest.json", "combined_read_probe.rs", "combined_read_probe"},
    "rw040-probe-build-evidence": {"compiler-messages.jsonl", "compiler-stderr.txt", "image-inspect.json", "build-provenance.json", "readelf-dynamic.txt", "readelf-version-info.txt", "readelf-program-headers.txt"},
}


def sha(blob):
    return hashlib.sha256(blob).hexdigest()


def unique(pairs):
    value = {}
    for key, item in pairs:
        assert key not in value, ("duplicate JSON field", key)
        value[key] = item
    return value


def load(path):
    return json.loads(path.read_text(), object_pairs_hook=unique)


def command(*argv):
    return subprocess.check_output(list(argv), cwd=ROOT, text=True, timeout=30)


assert command("git", "rev-parse", "HEAD").strip() == SOURCE
assert not command("git", "status", "--porcelain", "--untracked-files=all")
acquisition_path = COLLECTOR / "input-acquisition.json"
acquisition = load(acquisition_path)
assert acquisition["run_id"] == RUN and acquisition["tested_source"] == SOURCE
assert acquisition["guest_executed"] is False and acquisition["qualification_claimed"] is False
job = acquisition["amd64_job"]
assert job["name"] == "check (ubuntu-24.04)" and job["status"] == "completed" and job["conclusion"] == "success"
assert next(step for step in job["steps"] if step["name"] == "Debian 12 release builds on native architecture")["conclusion"] == "success"
artifacts = acquisition["artifacts"]
assert len(artifacts) == len(EXPECTED) and {item["name"] for item in artifacts} == set(EXPECTED)
verified_archives = []
directories = {}
for item in artifacts:
    origin = item["workflow_run"]
    assert origin["id"] == RUN and origin["head_sha"] == SOURCE
    zipped = Path(item["zip_path"])
    assert zipped.is_file() and not zipped.is_symlink()
    blob = zipped.read_bytes()
    digest = sha(blob)
    assert digest == item["zip_sha256"] and item["api_digest"] == "sha256:" + digest
    assert len(blob) == item["zip_bytes"] and len(blob) < 64 * 1024 * 1024
    directory = Path(item["extracted_path"])
    directories[item["name"]] = directory
    with zipfile.ZipFile(zipped) as archive:
        members = archive.infolist()
        assert len(members) == len(EXPECTED[item["name"]])
        assert {member.filename for member in members} == EXPECTED[item["name"]]
        assert sum(member.file_size for member in members) < 128 * 1024 * 1024
        for member in members:
            assert "/" not in member.filename and "\\" not in member.filename
            assert not member.is_dir() and 0 <= member.file_size < 64 * 1024 * 1024
            assert stat.S_IFMT(member.external_attr >> 16) in (0, stat.S_IFREG)
            extracted = directory / member.filename
            assert stat.S_ISREG(extracted.lstat().st_mode) and not extracted.is_symlink()
            assert extracted.read_bytes() == archive.read(member)
    verified_archives.append({"name": item["name"], "id": item["id"], "api_digest": item["api_digest"], "zip_path": str(zipped), "verified_sha256": digest})
raw_log = Path(acquisition["amd64_log"]["path"]).read_bytes()
assert sha(raw_log) == acquisition["amd64_log"]["sha256"] and SOURCE.encode() in raw_log

runner_path = ROOT / "tests/qualification/rw040-combined/run_guest.py"
spec = importlib.util.spec_from_file_location("verified_owned_runner", runner_path)
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)
probe_dir = directories["rw040-public-library-probe"]
probe = runner.verify_probe(probe_dir, SOURCE)
assert probe["source_commit"] == SOURCE
evidence_dir = directories["rw040-probe-build-evidence"]
provenance = load(evidence_dir / "build-provenance.json")
assert provenance["tested_source"] == SOURCE and provenance["manifest"] == probe
assert provenance["build_glibc"] == "glibc 2.36"
assert re.search(r"^release: 1\.88\.0$", provenance["rustc_vv"], re.MULTILINE)
assert re.search(r"^host: x86_64-unknown-linux-gnu$", provenance["rustc_vv"], re.MULTILINE)
images = load(evidence_dir / "image-inspect.json")
assert len(images) == 1 and re.fullmatch(r"sha256:[0-9a-f]{64}", images[0]["Id"])
assert provenance["builder_image_id"] == images[0]["Id"]
assert provenance["builder_repo_digests"] == images[0].get("RepoDigests", [])
messages_blob = (evidence_dir / "compiler-messages.jsonl").read_bytes()
assert sha(messages_blob) == provenance["compiler_messages_sha256"]
messages = [json.loads(line, object_pairs_hook=unique) for line in messages_blob.splitlines() if line.strip()]
finished = [message for message in messages if message.get("reason") == "build-finished"]
assert len(finished) == 1 and finished[0].get("success") is True
selected = [message for message in messages if message.get("reason") == "compiler-artifact" and message.get("target", {}).get("name") == "combined_read_probe"]
assert len(selected) == 1
artifact = selected[0]
assert artifact["target"]["kind"] == ["test"]
assert artifact["target"]["src_path"] == "/build/bins/executor/tests/combined_read_probe.rs"
assert artifact["manifest_path"] == "/build/bins/executor/Cargo.toml"
assert artifact["profile"]["opt_level"] == "3" and artifact["profile"]["test"] is True and artifact["profile"]["debug_assertions"] is False
assert re.fullmatch(r"/build/target/release/deps/combined_read_probe-[0-9a-f]+", artifact["executable"])

executable = probe_dir / "combined_read_probe"
binary = executable.read_bytes()
assert binary[:7] == b"\x7fELF\x02\x01\x01" and int.from_bytes(binary[18:20], "little") == 62
assert provenance["elf"] == {"class": 64, "endianness": "little", "machine": 62}
headers = command("readelf", "--program-headers", str(executable))
dynamic = command("readelf", "--dynamic", str(executable))
versions = command("readelf", "--version-info", str(executable))
assert "[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]" in headers
needed = re.findall(r"\(NEEDED\).*Shared library: \[([^\]]+)\]", dynamic)
assert len(needed) == len(set(needed)) and {"libc.so.6", "libgcc_s.so.1"} <= set(needed) <= {"libc.so.6", "libgcc_s.so.1", "libm.so.6", "ld-linux-x86-64.so.2"}
assert sorted(needed) == provenance["needed_libraries"]
required = set(re.findall(r"\bGLIBC_([0-9]+(?:\.[0-9]+)+)\b", versions))
assert required and all(tuple(map(int, value.split("."))) <= (2, 36) for value in required)
assert not set(re.findall(r"\bGLIBC_[A-Za-z0-9_.]+\b", versions)) - {"GLIBC_" + value for value in required} - {"GLIBC_ABI_DT_RELR"}
assert sorted(required, key=lambda value: tuple(map(int, value.split(".")))) == provenance["required_glibc_versions"]
for name, output in (("local-readelf-program-headers.txt", headers), ("local-readelf-dynamic.txt", dynamic), ("local-readelf-version-info.txt", versions)):
    (OUTPUT / name).write_text(output)

tracked = command("git", "ls-files").splitlines()
expected_inputs = {name for name in tracked if name.startswith(("crates/", "bins/", ".cargo/", "scripts/")) or name in {"Cargo.lock", "Cargo.toml", "rust-toolchain.toml", "packaging/Containerfile", ".github/workflows/ci.yml"}}
inputs = provenance["source_inputs"]
assert len(inputs) == len(expected_inputs) and {item["path"] for item in inputs} == expected_inputs
for item in inputs:
    blob = (ROOT / item["path"]).read_bytes()
    assert len(blob) == item["bytes"] and sha(blob) == item["sha256"]
assert sha((ROOT / "Cargo.lock").read_bytes()) == provenance["cargo_lock_sha256"]
package_dir = directories["debian-ubuntu-24.04"]
assert len(provenance["packages"]) == 3 and {item["file"] for item in provenance["packages"]} == EXPECTED["debian-ubuntu-24.04"]
for item in provenance["packages"]:
    package = package_dir / item["file"]
    blob = package.read_bytes()
    assert len(blob) == item["bytes"] and sha(blob) == item["sha256"] and item["tested_source"] == SOURCE
    control = command("dpkg-deb", "--field", str(package))
    assert control == item["control"]
    fields = dict(line.split(": ", 1) for line in control.splitlines() if ": " in line)
    name, version, architecture = item["file"].removesuffix(".deb").split("_")
    assert fields["Package"] == name and fields["Version"] == version and fields["Architecture"] == architecture == "amd64"
packages = runner.derive_package_manifest(package_dir, SOURCE)
assert len(packages["packages"]) == 3
for item in packages["packages"]:
    assert {"limeos-core", "limeos-executor", "limeosctl"} <= set(item["binaries"])
binary_maps = [item["binaries"] for item in packages["packages"]]
assert all(item == binary_maps[0] for item in binary_maps)
(OUTPUT / "derived-package-manifest.json").write_text(json.dumps(packages, indent=2, sort_keys=True) + "\n")
image = Path("/tmp/limeos-rw040-inputs/image/debian-12-genericcloud-amd64.qcow2")
checksums = image.parent / "SHA512SUMS"
image_sha512 = runner.verify_image(image, checksums)
record = {"schema": 1, "verified_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "source_commit": SOURCE, "source_clean": True, "run_id": RUN, "amd64_job_id": job["id"], "amd64_job_conclusion": job["conclusion"], "acquisition_record": str(acquisition_path), "acquisition_record_sha256": sha(acquisition_path.read_bytes()), "verified_archives": verified_archives, "probe": probe, "selected_cargo_executable": artifact["executable"], "builder_image_id": provenance["builder_image_id"], "build_glibc": provenance["build_glibc"], "required_glibc_versions": provenance["required_glibc_versions"], "needed_libraries": sorted(needed), "source_inputs_verified": len(inputs), "derived_package_manifest": str(OUTPUT / "derived-package-manifest.json"), "package_binary_identity_matches_across_all_three_packages": True, "image_sha512": image_sha512, "image_checksum_file_sha256": sha(checksums.read_bytes()), "worktree": str(ROOT), "work_root": "/home/marc/Documents/github/lime-os/target/rw040-guests", "deadline_seconds": 1800, "guest_budget_seconds": 1680, "cpus": 2, "memory_mib": 1536, "budget_total": 65, "scope": "Only U0/B0 standalone library/tool baselines; no policy variants or combined-worker claims", "guest_launched": False}
(OUTPUT / "verified-inputs.json").write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
print(json.dumps(record, indent=2, sort_keys=True))
