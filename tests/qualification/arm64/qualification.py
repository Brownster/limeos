"""Shared identity checks for disposable ARM64 qualification runs (stdlib only)."""

import hashlib
import json
import os
import platform
import re
import socket
import subprocess
from pathlib import Path

BINARIES = ("limeos-core", "limeos-password-worker", "limeos-executor", "limeosctl")
BASE_UNITS = ("limeos-core", "limeos-containerd", "limeos-storaged")
OPTIONAL_UNITS = ("limeos-storage-reader", "limeos-storage-targets")


def source_schema(source_root):
    """The schema a source tree declares: the last entry of its migration
    table (`MIGRATIONS` in crates/persistence/src/migrations.rs), or an older
    tree's `pub const SCHEMA_VERSION: u32 = N;` in lib.rs. Self-contained: this
    file runs in guests without the VM fixtures' reader
    (`tests/privileged_vm/current_authority_schema.py`), which a test holds
    to the same answer."""
    root = Path(source_root)
    table = root / "crates/persistence/src/migrations.rs"
    if table.exists():
        text = table.read_text()
        start = re.search(r"(?m)^[ \t]*pub\(crate\)[ \t]+const[ \t]+MIGRATIONS:[ \t]*&\[Migration\][ \t]*=[ \t]*&\[[ \t]*$", text)
        end = start and re.search(r"(?m)^\];[ \t]*$", text[start.end():])
        if not start or not end:
            raise ValueError(f"{table}: no `pub(crate) const MIGRATIONS: &[Migration] = &[` ... `];` table")
        body = text[start.end(): start.end() + end.start()]
        versions = [int(a or b) for a, b in re.findall(
            r"\b(?:plain|checked)\(\s*([0-9]+)\s*,|\bMigration\s*\{\s*version:\s*([0-9]+)\s*,", body)]
        if not versions or versions != list(range(1, len(versions) + 1)):
            raise ValueError(f"{table}: MIGRATIONS versions are not 1, 2, 3 ... in order: {versions}")
        return versions[-1]
    lib = root / "crates/persistence/src/lib.rs"
    found = re.search(r"(?m)^pub const SCHEMA_VERSION: u32 = ([1-9][0-9]*);$", lib.read_text())
    if found is None:
        raise ValueError(f"{lib}: no `pub const SCHEMA_VERSION: u32 = N;` line and no migration table")
    return int(found[1])


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load_manifest(path):
    manifest = json.loads(Path(path).read_text())
    identity = manifest["identity"]
    for field in ("source_commit", "fixtures_commit"):
        if not re.fullmatch(r"[0-9a-f]{40}", identity[field]):
            raise ValueError(f"invalid {field}")
    if not re.fullmatch(r"[0-9][A-Za-z0-9.+~-]*", identity["package_version"]):
        raise ValueError("invalid package_version")
    if identity["architecture"] != "arm64" or identity["authority_schema"] < 1:
        raise ValueError("positive schema and arm64 architecture required")
    return manifest


def guest_guard():
    if os.geteuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise RuntimeError("disposable limeos-p01-test guest required")
    if platform.machine() != "aarch64":
        raise RuntimeError("native aarch64 guest required")
    virtualization = subprocess.check_output(["systemd-detect-virt"], text=True).strip()
    if virtualization != "kvm":
        raise RuntimeError(f"native KVM guest required, found {virtualization}")


def verify_files(root, files):
    root = Path(root).resolve()
    if not files:
        raise ValueError("empty file manifest")
    for name, digest in files.items():
        path = root / name
        if not path.resolve().is_relative_to(root) or path.is_symlink():
            raise ValueError(f"unsafe manifest path: {name}")
        if sha(path) != digest:
            raise ValueError(f"SHA-256 mismatch: {name}")
    return len(files)


def verify_source(work, manifest):
    work = Path(work)
    count = verify_files(work / "source", manifest["runtime_source_sha256"])
    verify_files(work / "source", manifest["frontend_dist_sha256"])
    verify_files(work / "fixtures", manifest["fixtures_sha256"])
    schema = source_schema(work / "source")
    if schema != manifest["identity"]["authority_schema"]:
        raise ValueError(f"source schema {schema} differs from run identity")
    return count


def load_build(path, repo=None):
    build = load_manifest(path)
    kind = build.get("build_kind", "native-kvm")
    if kind == "native-ci-elfs-local-packaging":
        from ci_provenance import validate_build

        try:
            validate_build(Path(path), build, repo)
        except (KeyError, TypeError, IndexError, AttributeError) as error:
            raise ValueError("malformed CI build provenance") from error
    elif kind == "local-cross-build":
        from cross_build import validate_build

        try:
            validate_build(Path(path), build, repo)
        except (KeyError, TypeError, IndexError, AttributeError) as error:
            raise ValueError("malformed cross-build provenance") from error
    elif kind == "native-kvm":
        validate_native_gates(build)
    else:
        raise ValueError(f"unknown build provenance kind: {kind}")
    if set(build["binaries"]) != set(BINARIES):
        raise ValueError("all four runtime binaries required")
    if any(b["machine"] != "AArch64" for b in build["binaries"].values()):
        raise ValueError("AArch64 ELF binaries required")
    version = build["identity"]["package_version"]
    names = {f"{p}_{version}_arm64.deb" for p in ("limeos", "limeos-shadow")}
    if set(build["packages"]) != names:
        raise ValueError("exact standard and shadow package pair required")
    for package in ("limeos", "limeos-shadow"):
        name = f"{package}_{version}_arm64.deb"
        expected = f"Package: {package}\nVersion: {version}\nArchitecture: arm64\n"
        if build["package_control"][name] != expected:
            raise ValueError(f"package control identity differs: {name}")
    if repo is not None:
        packages = {p.name: sha(p) for p in Path(repo).rglob("*.deb")}
        if packages != build["packages"]:
            raise ValueError("repository package hashes differ from native build")
    return build


def validate_native_gates(build):
    if "ci" in build or "provenance_version" in build:
        raise ValueError("ambiguous native guest and CI provenance")
    required = {
        "apt",
        "rustup",
        "fetch",
        "fmt",
        "clippy",
        "test",
        "contracts",
        "dependency-direction",
        "release",
        "package-standard",
        "package-shadow",
        "signing-key",
        "repository",
    }
    steps = {step["step"]: step for step in build["steps"]}
    if not required.issubset(steps) or any(steps[s]["exit"] != 0 for s in required):
        raise ValueError("complete successful native build gates required")
    if build["tests"]["failed"] or not build["tests"]["passed"]:
        raise ValueError("successful workspace tests required")


def installed_binaries(build, prefix=Path("/usr/lib/limeos")):
    observed = {name: sha(prefix / name) for name in BINARIES}
    if observed != {n: b["sha256"] for n, b in build["binaries"].items()}:
        raise ValueError(f"installed binary hashes differ: {prefix}")
    return observed
