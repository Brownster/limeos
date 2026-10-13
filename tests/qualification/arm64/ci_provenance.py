"""Offline validation of native CI inputs. Never executes an ARM64 binary.

Version 1 supports the reviewed CI workflow digest below. GitHub API responses
are retained observations, not a cryptographic build attestation. The operator
must obtain them and the archive from the recorded repository/run.
"""

import gzip
import hashlib
import json
import re
import stat
import subprocess
import tarfile
import tempfile
import zipfile
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from pathlib import Path, PurePosixPath

from qualification import BINARIES, load_manifest, sha

BUILD_KIND = "native-ci-elfs-local-packaging"
REPOSITORY = "Brownster/limeos"
API = f"https://api.github.com/repos/{REPOSITORY}"
WEB = f"https://github.com/{REPOSITORY}"
WORKFLOW_SHA256 = "c771b4c9c5ed8ad533ebb5d015a20d06bbbc84cdb5e6f1776b377b51ba92ae73"
CONTAINERFILE_SHA256 = (
    "5ff45cc73ae22e80e58dce126c71fa71c0fcfd11ba73c0b706b36167f2a52f32"
)
ORIGINALS = {
    "limeos_0.4.4_arm64.deb": ("limeos", "0.4.4"),
    "limeos_0.4.4+ci.1_arm64.deb": ("limeos", "0.4.4+ci.1"),
    "limeos-shadow_0.4.4_arm64.deb": ("limeos-shadow", "0.4.4"),
}
REQUIRED_GATES = (
    "Run actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803",
    "Run actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38",
    "Toolchain and native validation tools",
    "Run cargo fmt --all -- --check",
    "Run cargo clippy --workspace --all-targets --locked -- -D warnings",
    "Run cargo test --workspace --locked",
    "Run cargo deny check",
    "Run cargo audit",
    "Run shellcheck packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm packaging/repository.sh tests/fixtures/backup-archives/generate.sh",
    "Unit validation",
    "Run python3 scripts/check_repository.py",
    "Run python3 scripts/check_contracts.py",
    "ARM64 qualification evidence and host guards",
    "Public-library probe artifact identity checks",
    "RW-040 console harness and probe delivery guards",
    "Run npm ci --ignore-scripts && npm test && npm run build && npm audit --audit-level=high",
    "Debian 12 release builds on native architecture",
    "Deliberate release-gate rejection probes",
    "Run actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def relative(name):
    path = PurePosixPath(name)
    require(
        isinstance(name, str)
        and name == path.as_posix()
        and not path.is_absolute()
        and ".." not in path.parts
        and name not in ("", "."),
        f"unsafe input path: {name}",
    )
    return path


def file_map(root):
    root = Path(root)
    require(
        root.is_dir() and not root.is_symlink(),
        f"regular input directory required: {root}",
    )
    result = {}
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), f"symlink input refused: {path}")
        if path.is_dir():
            continue
        require(path.is_file(), f"regular input file required: {path}")
        result[path.relative_to(root).as_posix()] = sha(path)
    require(bool(result), f"empty input directory: {root}")
    return result


def validate_ci_records(run, job, artifact, source):
    """Bind actual API identities and every required gate of the pinned pipeline."""
    for label, record in (("run", run), ("job", job)):
        require(
            record["status"] == "completed" and record["conclusion"] == "success",
            f"successful CI {label} required",
        )
        require(record["head_sha"] == source, f"CI {label} source differs")
        require(
            type(record["id"]) is int and record["id"] > 0,
            f"positive CI {label} ID required",
        )
    run_id, job_id = run["id"], job["id"]
    require(
        run["repository"]["full_name"] == REPOSITORY
        and run["head_repository"]["full_name"] == REPOSITORY,
        "CI repository differs",
    )
    require(run["head_commit"]["id"] == source, "CI head commit differs")
    require(run["path"] == ".github/workflows/ci.yml", "CI workflow differs")
    require(
        run["url"] == f"{API}/actions/runs/{run_id}"
        and run["html_url"] == f"{WEB}/actions/runs/{run_id}",
        "CI run URL differs",
    )
    require(
        job["run_id"] == run_id and job["run_url"] == run["url"], "CI job run differs"
    )
    require(
        job["url"] == f"{API}/actions/jobs/{job_id}"
        and job["html_url"] == f"{WEB}/actions/runs/{run_id}/job/{job_id}",
        "CI job URL differs",
    )
    require(
        job["name"] == "check (ubuntu-24.04-arm)"
        and job["labels"] == ["ubuntu-24.04-arm"],
        "native ARM64 CI runner required",
    )
    steps = job["steps"]
    by_name = {s["name"]: s for s in steps}
    require(len(by_name) == len(steps), "duplicate CI gate name")
    numbers = [s["number"] for s in steps]
    require(
        all(type(n) is int and n > 0 for n in numbers)
        and len(set(numbers)) == len(numbers),
        "unique positive CI gate numbers required",
    )
    previous = 0
    for name in REQUIRED_GATES:
        gate = by_name.get(name)
        require(
            gate is not None
            and gate["status"] == "completed"
            and gate["conclusion"] == "success",
            f"successful CI gate required: {name}",
        )
        require(gate["number"] > previous, f"CI gate order differs: {name}")
        previous = gate["number"]
    artifact_id = artifact["id"]
    require(
        type(artifact_id) is int and artifact_id > 0, "positive artifact ID required"
    )
    require(
        artifact["name"] == "debian-ubuntu-24.04-arm", "ARM64 package artifact required"
    )
    require(
        artifact["url"] == f"{API}/actions/artifacts/{artifact_id}"
        and artifact["archive_download_url"]
        == f"{API}/actions/artifacts/{artifact_id}/zip",
        "artifact repository URL differs",
    )
    bound = artifact["workflow_run"]
    require(
        bound["id"] == run_id and bound["head_sha"] == source,
        "artifact run/source differs",
    )
    require(
        bound["repository_id"] == run["repository"]["id"]
        and bound["head_repository_id"] == run["head_repository"]["id"],
        "artifact repository ID differs",
    )
    require(
        re.fullmatch(r"sha256:[0-9a-f]{64}", artifact["digest"]) is not None,
        "artifact SHA-256 digest required",
    )
    # A locally retained archive may outlive GitHub retention. Its exact bytes,
    # rather than the current 'expired' flag, bind historical provenance.


def package_payload(package):
    """Inspect contained files, modes and owners without installing/executing."""
    control = subprocess.check_output(
        ["dpkg-deb", "-f", str(package), "Package", "Version", "Architecture"],
        text=True,
    )
    entries, runtime = {}, {}
    for option, label in (("--fsys-tarfile", "data"), ("--ctrl-tarfile", "control")):
        process = subprocess.Popen(
            ["dpkg-deb", option, str(package)], stdout=subprocess.PIPE
        )
        try:
            with tarfile.open(fileobj=process.stdout, mode="r|") as archive:
                for member in archive:
                    name = member.name.removeprefix("./").rstrip("/")
                    if name in ("", "."):
                        continue
                    relative(name)
                    require(
                        name not in entries.get(label, {}),
                        f"duplicate package member: {name}",
                    )
                    require(
                        member.isdir() or member.isfile() or member.issym(),
                        f"unsupported package member: {name}",
                    )
                    entry = {
                        "mode": member.mode,
                        "uid": member.uid,
                        "gid": member.gid,
                        "kind": "directory"
                        if member.isdir()
                        else "symlink"
                        if member.issym()
                        else "file",
                    }
                    if member.issym():
                        entry["target"] = member.linkname
                    if member.isfile():
                        require(
                            member.size <= 32 * 1024 * 1024,
                            f"oversize package member: {name}",
                        )
                        data = archive.extractfile(member).read()
                        entry.update(sha256=digest(data), bytes=len(data))
                        if label == "data":
                            for prefix in ("usr/lib/limeos/", "usr/lib/limeos-shadow/"):
                                if name.startswith(prefix):
                                    short = name.removeprefix(prefix)
                                    require(
                                        short not in runtime,
                                        f"ambiguous runtime package member: {short}",
                                    )
                                    runtime[short] = data
                    entries.setdefault(label, {})[name] = entry
            require(process.wait() == 0, f"package payload command failed: {package}")
        finally:
            process.stdout.close()
            if process.poll() is None:
                process.kill()
                process.wait()
    binaries = {}
    for name in BINARIES:
        data = runtime.get(name, b"")
        require(
            len(data) >= 20
            and data[:6] == b"\x7fELF\x02\x01"
            and int.from_bytes(data[18:20], "little") == 183,
            f"AArch64 ELF required: {name}",
        )
        binaries[name] = {
            "sha256": digest(data),
            "bytes": len(data),
            "machine": "AArch64",
        }
    return {
        "control": control,
        "payload": entries,
        "binaries": binaries,
        "frontend": {
            "frontend/dist/" + n.removeprefix("ui/"): digest(d)
            for n, d in runtime.items()
            if n.startswith("ui/")
        },
        "contracts": {
            "contracts/generated/" + n.removeprefix("contracts/"): digest(d)
            for n, d in runtime.items()
            if n.startswith("contracts/")
        },
    }, runtime


def inspect_ci(root, source):
    observations = {
        name: json.loads((root / f"{name}.json").read_text())
        for name in ("run", "job", "artifact")
    }
    validate_ci_records(**observations, source=source)
    archive = root / "artifact.zip"
    artifact = observations["artifact"]
    require(
        sha(archive) == artifact["digest"].removeprefix("sha256:")
        and archive.stat().st_size == artifact["size_in_bytes"],
        "CI archive digest/size differs",
    )
    packages = {}
    native = None
    ui = None
    with (
        zipfile.ZipFile(archive) as zipped,
        tempfile.TemporaryDirectory(prefix="limeos-ci-package-") as directory,
    ):
        members = zipped.infolist()
        require(
            len(members) == len(ORIGINALS)
            and {m.filename for m in members} == set(ORIGINALS),
            "exact CI package archive required",
        )
        for member in members:
            mode = member.external_attr >> 16
            require(
                member.file_size <= 32 * 1024 * 1024
                and not member.is_dir()
                and not stat.S_ISLNK(mode)
                and not member.flag_bits & 1,
                "regular unencrypted CI package required",
            )
            data = zipped.read(member)
            path = Path(directory) / member.filename
            path.write_bytes(data)
            observed, runtime = package_payload(path)
            package, version = ORIGINALS[member.filename]
            require(
                observed["control"]
                == f"Package: {package}\nVersion: {version}\nArchitecture: arm64\n",
                "original CI control differs",
            )
            if native is None:
                native, ui = observed["binaries"], observed["frontend"]
            require(
                observed["binaries"] == native and observed["frontend"] == ui,
                "mixed native CI binary/frontend payloads",
            )
            packages[member.filename] = {
                "sha256": digest(data),
                "bytes": len(data),
                **observed,
            }
            if member.filename == "limeos_0.4.4_arm64.deb":
                standard_runtime = runtime
    return {"observations": observations, "packages": packages}, standard_runtime


def verify_bundle(bundle, manifest):
    require(sha(bundle) == manifest["bundle_sha256"], "source bundle SHA-256 differs")
    expected = {
        "source/" + n: d
        for n, d in (
            manifest["runtime_source_sha256"] | manifest["frontend_dist_sha256"]
        ).items()
    }
    expected.update(
        {"fixtures/" + n: d for n, d in manifest["fixtures_sha256"].items()}
    )
    require(
        all(
            manifest[k]
            for k in (
                "runtime_source_sha256",
                "frontend_dist_sha256",
                "fixtures_sha256",
            )
        ),
        "complete source/frontend/fixture manifests required",
    )
    seen = set()
    schema = None
    with tarfile.open(bundle) as archive:
        for member in archive:
            relative(member.name)
            require(
                member.isfile()
                and member.name not in seen
                and member.size <= 32 * 1024 * 1024,
                "regular unique source bundle files required",
            )
            seen.add(member.name)
            data = archive.extractfile(member).read()
            if member.name == "source-manifest.json":
                require(
                    json.loads(data)
                    == {k: v for k, v in manifest.items() if k != "bundle_sha256"},
                    "internal source manifest differs",
                )
            else:
                require(
                    expected.get(member.name) == digest(data),
                    f"bundle member SHA-256 differs: {member.name}",
                )
            if member.name == "source/crates/persistence/src/lib.rs":
                schema = int(
                    re.search(rb"pub const SCHEMA_VERSION: u32 = (\d+);", data)[1]
                )
    require(
        seen == set(expected) | {"source-manifest.json"},
        "source bundle file set differs",
    )
    require(
        schema == manifest["identity"]["authority_schema"],
        "bundle authority schema differs",
    )
    source = manifest["runtime_source_sha256"]
    require(
        source.get(".github/workflows/ci.yml") == WORKFLOW_SHA256
        and source.get("packaging/Containerfile") == CONTAINERFILE_SHA256,
        "unreviewed native CI pipeline; update provenance version before use",
    )


def verify_repository(repo, fingerprint, now=None):
    require(
        re.fullmatch(r"[A-F0-9]{40,64}", fingerprint) is not None,
        "signing fingerprint required",
    )
    repo = Path(repo).resolve()
    files = file_map(repo)
    indexes = {
        f"dists/stable/main/binary-{a}/Packages{s}"
        for a in ("arm64", "amd64")
        for s in ("", ".gz")
    }
    package_files = {
        n
        for n in files
        if n.startswith("pool/main/")
        and n.endswith(".deb")
        and len(PurePosixPath(n).parts) == 3
    }
    require(
        bool(package_files)
        and set(files)
        == indexes
        | package_files
        | {
            "limeos-archive-keyring.gpg",
            "dists/stable/Release",
            "dists/stable/InRelease",
            "dists/stable/Release.gpg",
        },
        "exact public test repository file set required",
    )
    for architecture in ("arm64", "amd64"):
        index = repo / f"dists/stable/main/binary-{architecture}/Packages"
        require(
            gzip.decompress(index.with_suffix(".gz").read_bytes())
            == index.read_bytes(),
            "repository compressed index differs",
        )
    for signature, arguments in (
        ("InRelease", []),
        ("Release.gpg", [str(repo / "dists/stable/Release")]),
    ):
        result = subprocess.run(
            [
                "gpgv",
                "--homedir",
                str(repo),
                "--status-fd",
                "1",
                "--keyring",
                str(repo / "limeos-archive-keyring.gpg"),
                str(repo / "dists/stable" / signature),
                *arguments,
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        valid = [
            line.split()
            for line in result.stdout.splitlines()
            if line.startswith("[GNUPG:] VALIDSIG ")
        ]
        require(
            result.returncode == 0 and len(valid) == 1 and valid[0][2] == fingerprint,
            "repository signature/fingerprint differs",
        )
    release = (repo / "dists/stable/Release").read_text()
    inrelease = (repo / "dists/stable/InRelease").read_text()
    clear = inrelease.split("\n\n", 1)[1].split("-----BEGIN PGP SIGNATURE-----", 1)[0]
    require(clear == release, "InRelease and detached Release content differs")
    validity = re.findall(r"^Valid-Until: (.+)$", release, re.MULTILINE)
    require(
        len(validity) == 1
        and parsedate_to_datetime(validity[0]) > (now or datetime.now(UTC)),
        "test repository has expired",
    )
    hashes = release.split("SHA256:\n", 1)[1]
    covered = {}
    for line in hashes.splitlines():
        if not line.startswith(" "):
            break
        value, size, name = line.split()
        relative(name)
        path = repo / "dists/stable" / name
        require(
            files.get("dists/stable/" + name) == value
            and path.stat().st_size == int(size),
            "repository Release index binding differs",
        )
        covered[name] = value
    require(
        {
            f"main/binary-{a}/Packages{s}"
            for a in ("arm64", "amd64")
            for s in ("", ".gz")
        }.issubset(covered),
        "repository Release omits package indexes",
    )
    packages = {}
    for paragraph in (
        (repo / "dists/stable/main/binary-arm64/Packages")
        .read_text()
        .strip()
        .split("\n\n")
    ):
        fields = dict(
            line.split(": ", 1)
            for line in paragraph.splitlines()
            if ": " in line and not line.startswith(" ")
        )
        name = fields["Filename"]
        relative(name)
        require(
            name.startswith("pool/main/") and fields["Architecture"] == "arm64",
            "repository ARM64 index identity differs",
        )
        require(
            name not in packages
            and files.get(name) == fields["SHA256"]
            and (repo / name).stat().st_size == int(fields["Size"]),
            "repository package index binding differs",
        )
        packages[name] = fields["SHA256"]
    require(
        set(packages) == {n for n in files if n.endswith(".deb")},
        "repository has unindexed or duplicate packages",
    )
    return files


def validate_build(path, build, repo=None):
    """Require adjacent inputs/, including CI ZIP, source bundle and signed repo."""
    require(
        type(build.get("provenance_version")) is int
        and build["provenance_version"] == 1,
        "unknown CI provenance version",
    )
    require(
        "steps" not in build and "tests" not in build and "platform" not in build,
        "ambiguous CI and native guest build provenance",
    )
    root = path.parent / "inputs"
    require(file_map(root) == build["input_sha256"], "CI input file manifest differs")
    required_files = {
        "source-manifest.json",
        "source.tar.gz",
        "ci/run.json",
        "ci/job.json",
        "ci/artifact.json",
        "ci/artifact.zip",
        "packaging/standard.txt",
        "packaging/shadow.txt",
    } | {"repo/" + n for n in build["repository_sha256"]}
    require(
        set(build["input_sha256"]) == required_files,
        "exact public CI input file set required",
    )
    manifest_path = root / "source-manifest.json"
    manifest = load_manifest(manifest_path)
    require(
        manifest["identity"] == build["identity"]
        and sha(manifest_path) == build["source_manifest_sha256"],
        "CI source manifest identity/bytes differ",
    )
    require(
        manifest["identity"].get("qualification_kind") == "native-ci-build-kvm-install",
        "CI qualification kind differs",
    )
    for field in ("runtime_source_sha256", "frontend_dist_sha256", "fixtures_sha256"):
        require(build[field] == manifest[field], f"CI build {field} differs")
    verify_bundle(root / "source.tar.gz", manifest)
    ci, _ = inspect_ci(root / "ci", build["identity"]["source_commit"])
    require(ci == build["ci"], "CI observed provenance differs")
    native = ci["packages"]["limeos_0.4.4_arm64.deb"]
    frontend = manifest.get("frontend_build", {})
    require(
        frontend
        == {
            "kind": "native-ci",
            "run_id": ci["observations"]["run"]["id"],
            "job_id": ci["observations"]["job"]["id"],
            "artifact_zip_sha256": ci["observations"]["artifact"][
                "digest"
            ].removeprefix("sha256:"),
        },
        "CI frontend provenance differs",
    )
    require(
        native["binaries"] == build["binaries"]
        and native["frontend"] == build["frontend_dist_sha256"],
        "candidate binary/frontend differs from native CI",
    )
    contracts = {
        n: d
        for n, d in build["runtime_source_sha256"].items()
        if n.startswith("contracts/generated/")
    }
    require(
        bool(contracts)
        and all(p["contracts"] == contracts for p in ci["packages"].values()),
        "CI generated contracts differ from frozen source",
    )
    packaging = build["packaging"]
    require(
        packaging["method"] == "frozen-builder-payload-comparison"
        and packaging["builder_sha256"]
        == manifest["runtime_source_sha256"]["packaging/build.py"],
        "frozen package builder differs",
    )
    require(
        len(packaging["steps"]) == 2
        and {s["profile"] for s in packaging["steps"]} == {"standard", "shadow"}
        and all(
            s["exit"] == 0
            and s["log_sha256"]
            == build["input_sha256"][f"packaging/{s['profile']}.txt"]
            for s in packaging["steps"]
        ),
        "successful local package comparison gates required",
    )
    signed = root / "repo"
    require(
        verify_repository(signed, build["signing_fingerprint"])
        == build["repository_sha256"],
        "signed repository input differs",
    )
    if repo is not None:
        require(
            file_map(repo) == build["repository_sha256"],
            "installed repository differs from CI input",
        )
    observed = {}
    for package in signed.rglob("*.deb"):
        payload, _ = package_payload(package)
        require(
            payload["binaries"] == build["binaries"]
            and payload["frontend"] == build["frontend_dist_sha256"]
            and payload["contracts"] == contracts,
            "candidate package runtime inputs differ",
        )
        require(
            payload["control"] == build["package_control"][package.name]
            and sha(package) == build["packages"][package.name],
            "candidate package identity differs",
        )
        observed[package.name] = payload["payload"]
    require(
        observed == build["package_payloads"],
        "candidate package payload differs from frozen builder comparison",
    )
