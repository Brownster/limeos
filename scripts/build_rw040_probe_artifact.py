#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Publish a hash-bound Debian 12 AMD64 public-library probe supply from CI."""

import argparse
import hashlib
import json
import re
import shutil
import stat
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROBE = Path("bins/executor/tests/combined_read_probe.rs")
SOURCE_SHA256 = "3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c"
PACKAGE_NAMES = (
    "limeos_0.4.4_amd64.deb",
    "limeos_0.4.4+ci.1_amd64.deb",
    "limeos-shadow_0.4.4_amd64.deb",
)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def command(*args: str) -> str:
    invocation = list(args)
    if invocation[0] == "git":
        invocation[1:1] = ["-c", f"safe.directory={ROOT}"]
    return subprocess.check_output(invocation, cwd=ROOT, text=True, timeout=30)


def regular_bytes(path: Path) -> bytes:
    if not stat.S_ISREG(path.lstat().st_mode) or path.resolve() != path.absolute():
        raise ValueError(f"expected a regular non-symlink file: {path}")
    return path.read_bytes()


def select_executable(messages: str, root: Path) -> Path:
    selected = []
    finished = []
    for line in messages.splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        if not isinstance(record, dict):
            raise TypeError("compiler message must be an object")
        if record.get("reason") == "build-finished":
            finished.append(record.get("success"))
        if record.get("reason") != "compiler-artifact":
            continue
        target = record.get("target", {})
        if not isinstance(target, dict) or target.get("name") != "combined_read_probe":
            continue
        if target.get("kind") != ["test"] or target.get("src_path") != str(
            root / PROBE
        ):
            raise ValueError("probe target kind or source path does not match")
        if record.get("manifest_path") != str(root / "bins/executor/Cargo.toml"):
            raise ValueError("probe package manifest does not match")
        profile = record.get("profile", {})
        if (
            not isinstance(profile, dict)
            or profile.get("opt_level") != "3"
            or profile.get("test") is not True
            or profile.get("debug_assertions") is not False
        ):
            raise ValueError("probe must be the release integration-test artifact")
        executable = record.get("executable")
        if not isinstance(executable, str):
            raise TypeError("probe compiler artifact has no executable")
        path = Path(executable)
        if path.parent != root / "target/release/deps" or not re.fullmatch(
            r"combined_read_probe-[0-9a-f]+", path.name
        ):
            raise ValueError(
                "probe executable is outside its exact release artifact path"
            )
        selected.append(path)
    if len(finished) != 1 or finished[0] is not True or len(selected) != 1:
        raise ValueError(
            "require one successful build and exactly one probe executable"
        )
    return selected[0]


def elf_identity(data: bytes) -> dict:
    if len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01":
        raise ValueError("probe is not a 64-bit little-endian ELF")
    machine = int.from_bytes(data[18:20], "little")
    if machine != 62:
        raise ValueError("probe must be AMD64")
    return {"class": 64, "endianness": "little", "machine": machine}


def glibc_requirements(version_info: str) -> list[str]:
    values = set(re.findall(r"\bGLIBC_([0-9]+(?:\.[0-9]+)+)\b", version_info))
    names = set(re.findall(r"\bGLIBC_[A-Za-z0-9_.]+\b", version_info))
    unknown = names - {"GLIBC_" + value for value in values} - {"GLIBC_ABI_DT_RELR"}
    if not values or unknown:
        raise ValueError("probe GLIBC requirements are absent or private")
    versions = sorted(values, key=lambda value: tuple(map(int, value.split("."))))
    if any(tuple(map(int, value.split("."))) > (2, 36) for value in versions):
        raise ValueError("probe requires GLIBC newer than Debian 12")
    return versions


def needed_libraries(dynamic_info: str) -> list[str]:
    names = re.findall(r"\(NEEDED\).*Shared library: \[([^\]]+)\]", dynamic_info)
    allowed = {"libgcc_s.so.1", "libm.so.6", "libc.so.6", "ld-linux-x86-64.so.2"}
    if not {"libgcc_s.so.1", "libc.so.6"} <= set(names) <= allowed or len(names) != len(
        set(names)
    ):
        raise ValueError("probe has unexpected dynamic library dependencies")
    return sorted(names)


def supply_manifest(source: str, source_bytes: bytes, binary: bytes) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", source) or sha(source_bytes) != SOURCE_SHA256:
        raise ValueError(
            "probe source identity does not match the supplied public probe"
        )
    elf_identity(binary)
    return {
        "contract": 1,
        "source_commit": source,
        "source_sha256": SOURCE_SHA256,
        "binary": "combined_read_probe",
        "binary_sha256": sha(binary),
        "toolchain": "1.88.0",
        "profile": "release",
        "library_source": source,
    }


def verify_supply(directory: Path, manifest: dict) -> None:
    if (
        regular_bytes(directory / "manifest.json")
        != (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    ):
        raise ValueError("published manifest identity changed")
    if (
        sha(regular_bytes(directory / "combined_read_probe"))
        != manifest["binary_sha256"]
        or sha(regular_bytes(directory / "combined_read_probe.rs"))
        != manifest["source_sha256"]
    ):
        raise ValueError("published binary or source identity changed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler-messages", type=Path, required=True)
    parser.add_argument("--image-info", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--packages-dir", type=Path, required=True)
    args = parser.parse_args()
    if command("git", "rev-parse", "HEAD").strip() != args.source or command(
        "git", "status", "--porcelain", "--untracked-files=no"
    ):
        raise ValueError("require the clean exact workflow source")
    source_bytes = regular_bytes(ROOT / PROBE)
    if command("git", "show", f"{args.source}:{PROBE}").encode() != source_bytes:
        raise ValueError("probe source differs from qualified Git bytes")
    executable = select_executable(regular_bytes(args.compiler_messages).decode(), ROOT)
    binary = regular_bytes(executable)
    if not executable.stat().st_mode & 0o111:
        raise ValueError("probe artifact has no executable permission")
    manifest = supply_manifest(args.source, source_bytes, binary)
    rustc = command("rustc", "-Vv")
    if not re.search(r"^release: 1\.88\.0$", rustc, re.MULTILINE) or not re.search(
        r"^host: x86_64-unknown-linux-gnu$", rustc, re.MULTILINE
    ):
        raise ValueError("require Rust 1.88.0 native AMD64")
    libc = command("getconf", "GNU_LIBC_VERSION").strip()
    if libc != "glibc 2.36":
        raise ValueError("probe must be built in the Debian 12 libc environment")
    dynamic = command("readelf", "--dynamic", str(executable))
    versions = command("readelf", "--version-info", str(executable))
    headers = command("readelf", "--program-headers", str(executable))
    if "[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]" not in headers:
        raise ValueError("probe has an unexpected ELF interpreter")
    libraries = needed_libraries(dynamic)
    required_glibc = glibc_requirements(versions)
    images = json.loads(regular_bytes(args.image_info))
    if (
        not isinstance(images, list)
        or len(images) != 1
        or not re.fullmatch(r"sha256:[0-9a-f]{64}", images[0].get("Id", ""))
    ):
        raise ValueError("require one resolved builder image identity")
    packages = []
    for name in PACKAGE_NAMES:
        path = args.packages_dir / name
        blob = regular_bytes(path)
        control = command("dpkg-deb", "--field", str(path))
        fields = dict(
            line.split(": ", 1) for line in control.splitlines() if ": " in line
        )
        package, version, architecture = name.removesuffix(".deb").split("_")
        if any(
            fields.get(key) != value
            for key, value in {
                "Package": package,
                "Version": version,
                "Architecture": architecture,
            }.items()
        ):
            raise ValueError("package identity does not match its selected filename")
        packages.append(
            {
                "file": name,
                "bytes": len(blob),
                "sha256": sha(blob),
                "tested_source": args.source,
                "control": control,
            }
        )
    inputs = []
    tracked = command("git", "ls-files").splitlines()
    for path in tracked:
        if path.startswith(("crates/", "bins/", ".cargo/", "scripts/")) or path in {
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain.toml",
            "packaging/Containerfile",
            ".github/workflows/ci.yml",
        }:
            blob = regular_bytes(ROOT / path)
            if (
                subprocess.check_output(
                    [
                        "git",
                        "-c",
                        f"safe.directory={ROOT}",
                        "show",
                        f"{args.source}:{path}",
                    ],
                    cwd=ROOT,
                )
                != blob
            ):
                raise ValueError(f"source changed: {path}")
            inputs.append({"path": path, "bytes": len(blob), "sha256": sha(blob)})
    if args.output.exists():
        raise ValueError("probe output directory already exists")
    args.evidence.mkdir(parents=True, exist_ok=True)
    provenance = {
        "tested_source": args.source,
        "manifest": manifest,
        "builder_image_id": images[0]["Id"],
        "builder_repo_digests": images[0].get("RepoDigests", []),
        "rustc_vv": rustc,
        "build_glibc": libc,
        "elf": elf_identity(binary),
        "needed_libraries": libraries,
        "required_glibc_versions": required_glibc,
        "cargo_lock_sha256": sha(regular_bytes(ROOT / "Cargo.lock")),
        "compiler_messages_sha256": sha(regular_bytes(args.compiler_messages)),
        "packages": packages,
        "source_inputs": inputs,
        "scope": "Supplied test executable and build compatibility only; no privileged probe execution or gate closure",
    }
    (args.evidence / "build-provenance.json").write_text(
        json.dumps(provenance, indent=2, sort_keys=True) + "\n"
    )
    (args.evidence / "readelf-dynamic.txt").write_text(dynamic)
    (args.evidence / "readelf-version-info.txt").write_text(versions)
    (args.evidence / "readelf-program-headers.txt").write_text(headers)
    args.output.mkdir(parents=True)
    (args.output / "combined_read_probe").write_bytes(binary)
    (args.output / "combined_read_probe").chmod(0o755)
    (args.output / "combined_read_probe.rs").write_bytes(source_bytes)
    (args.output / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    try:
        verify_supply(args.output, manifest)
    except ValueError:
        shutil.rmtree(args.output)
        raise
    print(
        json.dumps(
            {
                "tested_source": args.source,
                "binary_sha256": manifest["binary_sha256"],
                "output": str(args.output),
            }
        )
    )


if __name__ == "__main__":
    main()
