# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Derive isolated fixture packages from CI payloads and the committed executor."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[2]
state = Path(__file__).resolve().parent
build = json.loads((state / "debian-build-diagnostics/record.json").read_text())
assert build["exit_code"] == 0 and build["stop_reason"] is None
assert build["inputs_unchanged"] and build["container_removed"]
binary = state / "debian-target/release/limeos-executor"
assert hashlib.sha256(binary.read_bytes()).hexdigest() == build["binary_sha256"]
version = "0.4.4+namespace.2"
out = state / "diagnostic-candidate"
out.mkdir()
packages = out / "packages"
packages.mkdir()
image = "sha256:e0df67a00f0f3080a0d01274c4c492c1173c6e39853913864d2cf596ad726fab"
container = ["docker", "run", "--rm", "--network=none", "--memory=128m", "--pids-limit=32", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--security-opt=label=disable", "--read-only", "--user", "1000:1000", "--env", "GNUPGHOME=/build/.cache/a1-namespace-review/gnupg", "--env", "TMPDIR=/build/.cache/a1-namespace-review/tmp", "--volume", str(root) + ":/build", "--workdir", "/build", image]

def inside(path):
    return "/build/" + str(path.relative_to(root))

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def replace(path, data, mode):
    # Publish a new complete inode across the Docker Desktop shared mount.
    temporary = path.with_name(path.name + ".a1-new")
    with temporary.open("xb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.chmod(mode)
    temporary.replace(path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)

# The reused binaries' complete workspace dependency closures are unchanged.
unchanged = ["bins/core", "bins/ctl", "crates/domain", "crates/api", "crates/contracts", "crates/executor-protocol", "crates/observations", "crates/persistence", "crates/policy", "crates/identity", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo"]
subprocess.run(["git", "diff", "--exit-code", "477b81f", build["source"], "--", *unchanged], cwd=root, check=True)
spec = importlib.util.spec_from_file_location("fixture_package_validation", root / "packaging/build.py")
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)
records = []
for name in ["limeos", "limeos-shadow"]:
    original = state / "packages" / f"{name}_0.4.4_amd64.deb"
    stage = out / (name + "-stage")
    subprocess.run(["dpkg-deb", "--raw-extract", str(original), str(stage)], check=True)
    lib = stage / "usr/lib" / name
    before = {path.name: digest(path) for path in lib.iterdir() if path.is_file()}
    replace(lib / "limeos-executor", binary.read_bytes(), 0o755)
    document = stage / "usr/share/doc" / name / "p04-storage-mounts-profile.md"
    replace(document, (root / "docs/p04-storage-mounts-profile.md").read_bytes(), 0o644)
    control = stage / "DEBIAN/control"
    text = control.read_text()
    assert text.count("Version: 0.4.4\n") == 1
    replace(control, text.replace("Version: 0.4.4\n", f"Version: {version}\n").encode(), 0o644)
    validator.check_root_execution_payload(stage, name)
    checksums = []
    for file in sorted(stage.rglob("*")):
        relative = file.relative_to(stage)
        assert not file.is_symlink()
        if file.is_file() and relative.parts[0] != "DEBIAN":
            with file.open("rb") as stream:
                value = hashlib.file_digest(stream, "md5").hexdigest()
            checksums.append(f"{value}  {relative}")
    checksum_file = stage / "DEBIAN/md5sums"
    replace(checksum_file, ("\n".join(checksums) + "\n").encode(), 0o644)
    package = packages / f"{name}_{version}_amd64.deb"
    # Assemble entirely on the host. Docker Desktop returned different bytes
    # immediately after host updates to its shared staging tree; the failed
    # checksum handoff is preserved rather than retried into a false success.
    subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build", str(stage), str(package)], check=True)
    after = {path.name: digest(path) for path in lib.iterdir() if path.is_file()}
    for executable in ["limeos-core", "limeosctl", "limeos-password-worker"]:
        assert before[executable] == after[executable]
    records.append({"package": package.name, "sha256": digest(package), "baseline_package": original.name, "baseline_sha256": digest(original), "before_binaries": before, "after_binaries": after})
fingerprint = json.loads((state / "inputs.json").read_text())["signing_fingerprint"]
immutable_inputs = sorted(packages.glob("*.deb"))
observed = subprocess.check_output([*container, "sha256sum", *map(inside, immutable_inputs)], text=True).splitlines()
expected = [digest(path) for path in immutable_inputs]
(out / "package-handoff.json").write_text(json.dumps({"container": observed, "host": expected}, indent=2) + "\n")
assert [line.split()[0] for line in observed] == expected, "Immutable package handoff mismatch"
subprocess.run([*container, "packaging/repository.sh", inside(out / "repository"), fingerprint, *[inside(file) for file in sorted(packages.glob("*.deb"))]], check=True)
guest = out / "guest"
shutil.copytree(state / "guest", guest)
wrapper = guest / "namespace_review_guest.py"
text = wrapper.read_text()
assert text.count("approved.main()") == 1
wrapper.write_text(text.replace("approved.main()", f"approved.main(package_version={version!r})"))
manifest = {"executor_source": build["source"], "baseline_source": "477b81f2ba78c9d100a6857950f50467079b406e", "baseline_run": 39429582703, "scope": "derived disposable fixture packages; unchanged Core/ctl/password-worker/UI/contracts/units from CI, newly built executor plus updated profile documentation", "package_version": version, "packaging_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), "packages": records, "guest": {p.name: digest(p) for p in sorted(guest.glob("*.py"))}}
(out / "inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"repository": str(out / "repository"), "version": version, "executor_source": build["source"], "scope": manifest["scope"]}))
