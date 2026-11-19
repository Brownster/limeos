#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Move raw qualification evidence into reproducible bundles, verify and restore them.

The policy (what stays in git, what is raw, size limits, bundle names) is in
docs/rewrite-evidence/README.md. A unit is one run directory
docs/rewrite-evidence/<area>/<run>/ (recursive), or the loose files directly in
docs/rewrite-evidence/<area>/. Each bundled unit keeps a tracked
EVIDENCE-BUNDLE.json listing every moved file with its size and SHA-256.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
import re
import secrets
import stat
import subprocess
import sys
import tarfile
import zlib
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = PurePosixPath("docs/rewrite-evidence")
MANIFEST = "EVIDENCE-BUNDLE.json"
FORMAT = "limeos-evidence-bundle/1"
BUNDLE_PREFIX = "limeos-evidence-"
BUNDLE_SUFFIX = ".tar.gz"
# Fixed archive metadata: the same files always give the same tar bytes.
MTIME = 0

SMALL_LIMIT = 8 * 1024
INDEX_LIMIT = 128 * 1024
RAW_SUFFIXES = frozenset(
    {
        ".7z",
        ".bin",
        ".bmp",
        ".bz2",
        ".core",
        ".deb",
        ".gif",
        ".gz",
        ".img",
        ".iso",
        ".jpeg",
        ".jpg",
        ".jsonl",
        ".log",
        ".out",
        ".pcap",
        ".png",
        ".qcow2",
        ".stderr",
        ".stdout",
        ".tar",
        ".tgz",
        ".webp",
        ".xz",
        ".zip",
        ".zst",
    }
)
_STORAGE = "tests/privileged_vm/record_storage_evidence.py"
_COMPOSE = "tests/privileged_vm/record_compose_evidence.py"
_P03 = "tests/privileged_vm/record_p03_evidence.py"
_DEPS = "tests/privileged_vm/record_dependencies_evidence.py"
_ARM64 = "tests/qualification/arm64"
# Files that code reads, rewrites in place or cites. They stay tracked whatever
# their size or type. Exact paths under docs/rewrite-evidence/, never patterns.
_PINS = [
    ("p01/gate-probes.json", "written by scripts/test_gates.py in CI"),
    ("p01/vm-result.json", "default output of tests/privileged_vm/run.py"),
    ("p03/lifecycle-browser-result.json", "written by tests/frontend/restart.py"),
    ("p03/lifecycle-progress.png", "written by tests/frontend/restart.py in CI"),
    ("p03/compose-validation.json", f"default output of {_COMPOSE}"),
    ("p03/lifecycle-validation.json", f"output of {_P03}; read by {_COMPOSE}"),
    ("p03/compose-source-sha256.json", f"output of {_COMPOSE}; read by {_STORAGE}"),
    *[
        (f"p04/storage-{name}-validation.json", f"output of {_STORAGE}")
        for name in [
            "readiness",
            "planning",
            "targets",
            "targets-initial",
            "locks",
            "approved",
            "approved-initial",
            "contract",
        ]
    ],
    # Inputs the VM evidence recorders read (their evidence_inputs functions).
    ("p03/lifecycle-vm-result.json", f"read by {_P03}"),
    ("p03/rust-lifecycle-tests.txt", f"read by {_P03}"),
    ("p03/lifecycle-source-sha256.json", f"written by {_P03}; read by {_COMPOSE}"),
    ("p03/compose-vm-result.json", f"read by {_COMPOSE}"),
    ("p03/compose-arm64-result.json", f"read by {_COMPOSE}"),
    ("p03/compose-gate-probes.json", f"read by {_COMPOSE}"),
    ("p04/storage-vm-result.json", f"read by {_STORAGE} --slice readiness"),
    *[
        (f"p04/{name}-vm-result.json", f"read by {_STORAGE} --slice {name}")
        for name in ["planning", "targets", "locks", "approved"]
    ],
    *[
        (f"p04/{name}-container-vm-result.json", f"read by {_STORAGE} --slice {name}")
        for name in ["locks", "approved"]
    ],
    *[
        (f"p04/rust-{name}-tests.txt", f"read by {_STORAGE} --slice {name}")
        for name in ["readiness", "planning", "targets", "locks", "approved"]
    ],
    ("p04/rw040-dependencies/dependencies-vm-result.json", f"read by {_DEPS}"),
    ("p04/rw040-dependencies/rust-tests.txt", f"read by {_DEPS}"),
    ("p04/rw043/measurement-policy.json", f"read by {_ARM64}/inspector_guest.py"),
    ("p04/rw043-custody/measure_custody.py", "cited by tests/fixtures/backup-custody"),
    ("p04/rw043-staging/measure_staging.py", "cited by tests/fixtures/backup-staging"),
    (
        "p04/2026-10-08-library-handoff-integration/ci-passed-37746766303/ci-summary.json",
        "cited by tests/fixtures/rw040-combined/packages-2fd7420.json",
    ),
    (
        "p04/2026-10-06-handoff-integration/previous-schema7-artifact.json",
        f"cited by {_ARM64}/README.md",
    ),
    (
        "arm64/2026-11-05-pi-night-prep/harness/build_arm64_binaries.py",
        f"cited by {_ARM64}/README.md",
    ),
    (
        "arm64/2026-11-05-pi-night-prep/harness/package_arm64_candidate.py",
        f"cited by {_ARM64}/README.md",
    ),
]
PINNED = {f"{EVIDENCE}/{path}": reason for path, reason in _PINS}


class EvidenceError(Exception):
    pass


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def is_pinned(rel):
    return str(PurePosixPath(rel)) in PINNED


def is_index(name):
    """Human-written summaries and hash lists, allowed up to INDEX_LIMIT."""
    return (
        name == MANIFEST
        or name.lower().endswith(".md")
        or "SHA256SUMS" in name
        or "SHA512SUMS" in name
        or name.lower().endswith((".sha256", ".sha512"))
    )


def problem(rel, size):
    """Why this evidence file may not be tracked, or None when it may."""
    rel = PurePosixPath(rel)
    if is_pinned(rel):
        return None
    suffix = rel.suffix.lower()
    if suffix in RAW_SUFFIXES:
        return f"raw artifact type {suffix}"
    limit = INDEX_LIMIT if is_index(rel.name) else SMALL_LIMIT
    if size > limit:
        return f"{size} bytes exceeds the {limit}-byte limit"
    return None


def violations(root, tracked):
    """Policy violations among tracked repository-relative paths."""
    found = []
    for name in tracked:
        rel = PurePosixPath(name)
        if EVIDENCE not in rel.parents:
            continue
        path = Path(root) / rel
        if path.is_symlink() or not path.is_file():
            continue
        reason = problem(rel, path.stat().st_size)
        if reason:
            found.append(f"{rel}: {reason}")
    return found


def git_lines(root, *argv):
    """Git output split on NUL (pass -z) or newline, or None outside a repository."""
    try:
        result = subprocess.run(
            ["git", *argv], cwd=root, capture_output=True, check=False
        )
    except OSError:
        return None
    if result.returncode:
        return None
    separator = b"\0" if "-z" in argv else b"\n"
    return [os.fsdecode(item) for item in result.stdout.split(separator) if item]


def tracked_violations(root):
    """Policy violations among tracked evidence, or None when root is not a repository.

    This is the repository guard: scripts/check_repository.py calls it.
    """
    tracked = git_lines(root, "ls-files", "-z", "--", str(EVIDENCE))
    return None if tracked is None else violations(root, tracked)


def bad_component(part):
    """Empty, dot, or holding a control character (NUL included)."""
    return part in ("", ".", "..") or any(ord(c) < 32 or ord(c) == 127 for c in part)


def check_unit(unit):
    """A unit as docs/rewrite-evidence/<area>[/<run>], refusing anything else."""
    if any(ord(c) < 32 or ord(c) == 127 for c in str(unit)):
        raise EvidenceError(f"{unit!r}: control character in unit")
    unit = PurePosixPath(unit)
    if unit.is_absolute() or EVIDENCE not in unit.parents:
        raise EvidenceError(f"{unit}: not under {EVIDENCE}")
    parts = unit.relative_to(EVIDENCE).parts
    if len(parts) not in (1, 2) or any(bad_component(part) for part in parts):
        raise EvidenceError(f"{unit}: a unit is {EVIDENCE}/<area> or <area>/<run>")
    if len(parts) == 2 and parts[1].startswith("-"):
        raise EvidenceError(f"{unit}: run names may not start with '-'")
    return unit


def unit_of(rel):
    parts = PurePosixPath(rel).relative_to(EVIDENCE).parts
    if len(parts) < 2:
        return None
    return EVIDENCE.joinpath(*parts[:2]) if len(parts) > 2 else EVIDENCE / parts[0]


def bundle_name(unit):
    """limeos-evidence-<area>[-<run>].tar.gz, with each '-' in the area doubled.

    Areas without '-' keep the plain form. The area ends at the first single
    '-', and runs never start with '-', so different units get different names.
    """
    area, *run = check_unit(unit).relative_to(EVIDENCE).parts
    encoded = area.replace("-", "--") + "".join("-" + part for part in run)
    return BUNDLE_PREFIX + encoded + BUNDLE_SUFFIX


def is_loose(unit):
    return len(PurePosixPath(unit).relative_to(EVIDENCE).parts) == 1


def unit_files(root, unit):
    """Repository-relative files belonging to a unit, excluding its manifest."""
    base = Path(root) / unit
    paths = base.iterdir() if is_loose(unit) else base.rglob("*")
    files = []
    for path in paths:
        if path.is_symlink():
            raise EvidenceError(f"symbolic link in evidence: {path}")
        if path.is_file() and not (path.parent == base and path.name == MANIFEST):
            files.append(PurePosixPath(path.relative_to(root).as_posix()))
    return sorted(files, key=str)


def all_units(root):
    base = Path(root) / EVIDENCE
    units = []
    for area in sorted(p for p in base.iterdir() if p.is_dir()):
        units.append(EVIDENCE / area.name)
        units.extend(
            EVIDENCE / area.name / run.name
            for run in sorted(area.iterdir())
            if run.is_dir()
        )
    return units


def file_modes(root):
    """Git index modes when root is a repository's top level, else an empty map."""
    top = git_lines(root, "rev-parse", "--show-toplevel")
    if not top or Path(top[0].strip()).resolve() != Path(root).resolve():
        return {}
    modes = {}
    for record in git_lines(root, "ls-files", "-s", "-z", "--", str(EVIDENCE)) or []:
        meta, name = record.split("\t", 1)
        modes[name] = 0o755 if meta.split()[0] == "100755" else 0o644
    return modes


def tar_bytes(root, files, modes=None):
    """Deterministic tar of repository-relative files, plus a manifest entry for each.

    A file's mode comes from the git index when it is tracked, else from its
    owner-execute bit, so a noexec mount does not change the bytes.
    """
    modes = modes or {}
    buffer = io.BytesIO()
    entries = []
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for rel in sorted(files, key=str):
            path = Path(root) / rel
            data = path.read_bytes()
            info = tarfile.TarInfo(str(rel))
            info.size = len(data)
            info.mtime = MTIME
            mode = modes.get(str(rel))
            if mode is None:
                mode = 0o755 if path.stat().st_mode & 0o100 else 0o644
            info.mode = mode
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            archive.addfile(info, io.BytesIO(data))
            entries.append((rel, len(data), sha256(data)))
    return buffer.getvalue(), entries


def gzip_bytes(data):
    buffer = io.BytesIO()
    with gzip.GzipFile(
        filename="", mode="wb", fileobj=buffer, compresslevel=9, mtime=MTIME
    ) as stream:
        stream.write(data)
    return buffer.getvalue()


def render_manifest(manifest):
    """JSON with one line per file, so diffs and reviews stay readable."""
    head = json.dumps({k: v for k, v in manifest.items() if k != "files"}, indent=2)
    entries = ",\n".join("    " + json.dumps(entry) for entry in manifest["files"])
    return head[: -len("\n}")] + ',\n  "files": [\n' + entries + "\n  ]\n}\n"


def existing_manifests(root):
    return sorted((Path(root) / EVIDENCE).rglob(MANIFEST))


def plan_build(root, units):
    """Raw files and bundle name per unit, checked before anything is written."""
    names = {}
    for path in existing_manifests(root):
        manifest = load_manifest(path, root)
        names[manifest["bundle"]["name"]] = manifest["unit"]
    plans = []
    for unit in units:
        unit = PurePosixPath(unit)
        raw = [
            rel
            for rel in unit_files(root, unit)
            if problem(rel, (Path(root) / rel).stat().st_size)
        ]
        if not raw:
            continue
        for rel in raw:
            if any(bad_component(part) for part in rel.parts):
                raise EvidenceError(f"{rel!r}: control character in a file name")
        if (Path(root) / unit / MANIFEST).exists():
            if is_loose(unit):
                raise EvidenceError(
                    f"{raw[0]} is a loose raw file in {unit}, whose loose files are "
                    f"already bundled; move it into a run directory such as "
                    f"{unit}/YYYY-MM-DD-<name>/ and bundle that"
                )
            raise EvidenceError(
                f"{unit} already has {MANIFEST} but {raw[0]} is raw; record new "
                "evidence in a new run directory"
            )
        name = bundle_name(unit)
        if name in names:
            raise EvidenceError(
                f"bundle name {name} is used by {names[name]} and {unit}"
            )
        names[name] = str(unit)
        plans.append((unit, raw, name))
    return plans


def build(root, units, out, release, remove=True):
    """Bundle every unit's raw files; delete inputs only after all bundles verify."""
    root = Path(root)
    out = Path(out)
    plans = plan_build(root, units)
    modes = file_modes(root)
    out.mkdir(parents=True, exist_ok=True)
    built = []
    for unit, raw, name in plans:
        archive, entries = tar_bytes(root, raw, modes)
        compressed = gzip_bytes(archive)
        manifest = {
            "format": FORMAT,
            "policy": "docs/rewrite-evidence/README.md",
            "unit": str(unit),
            "bundle": {
                "release": release,
                "name": name,
                "size": len(compressed),
                "sha256": sha256(compressed),
                "tar_sha256": sha256(archive),
            },
            "files": [
                {"path": str(rel.relative_to(unit)), "size": size, "sha256": digest}
                for rel, size, digest in entries
            ],
        }
        target = out / name
        if target.exists() and target.read_bytes() != compressed:
            raise EvidenceError(f"{target} exists with different content")
        partial = out / (name + ".partial")
        partial.write_bytes(compressed)
        try:
            read_bundle(manifest, partial)
            os.replace(partial, target)
        finally:
            partial.unlink(missing_ok=True)
        read_bundle(manifest, target)
        built.append((unit, raw, manifest))
    for unit, raw, manifest in built:
        with open(root / unit / MANIFEST, "x") as stream:
            stream.write(render_manifest(manifest))
    if remove:
        for unit, raw, manifest in built:
            for rel, entry in zip(raw, manifest["files"]):
                if sha256((root / rel).read_bytes()) != entry["sha256"]:
                    raise EvidenceError(f"{rel} changed while it was bundled")
        for unit, raw, manifest in built:
            for rel in raw:
                (root / rel).unlink()
            for rel in sorted(
                {rel.parent for rel in raw}, key=lambda p: len(p.parts), reverse=True
            ):
                directory = root / rel
                while (
                    directory != root / unit
                    and directory.is_dir()
                    and not any(directory.iterdir())
                ):
                    directory.rmdir()
                    directory = directory.parent
    return [manifest for _, _, manifest in built]


def build_unit(root, unit, out, release, remove=True):
    """Bundle one unit; return its manifest, or None when nothing is raw."""
    manifests = build(root, [unit], out, release, remove=remove)
    return manifests[0] if manifests else None


def checked_path(text, loose):
    if not isinstance(text, str) or any(bad_component(p) for p in text.split("/")):
        raise EvidenceError(f"bad file path {text!r}")
    path = PurePosixPath(text)
    if (
        not text
        or path.is_absolute()
        or path.as_posix() != text
        or (loose and len(path.parts) != 1)
        or path == PurePosixPath(MANIFEST)
    ):
        raise EvidenceError(f"bad file path {text!r}")
    return path


def load_manifest(path, root=None):
    """Read and check a manifest; with root, it must sit in its own unit directory."""
    try:
        manifest = json.loads(Path(path).read_text())
        if manifest.get("format") != FORMAT:
            raise EvidenceError(f"unknown format {manifest.get('format')!r}")
        unit = check_unit(manifest["unit"])
        record = manifest["bundle"]
        if record["name"] != bundle_name(unit):
            raise EvidenceError(f"bundle name {record['name']!r} does not match {unit}")
        for key in ("sha256", "tar_sha256"):
            if not isinstance(record[key], str) or len(record[key]) != 64:
                raise EvidenceError(f"bad bundle {key}")
        files = manifest["files"]
        if not isinstance(files, list) or not files:
            raise EvidenceError("no files listed")
        seen = set()
        for entry in files:
            name = checked_path(entry["path"], is_loose(unit))
            if not isinstance(entry["size"], int) or entry["size"] < 0:
                raise EvidenceError(f"bad size for {name}")
            if not isinstance(entry["sha256"], str) or len(entry["sha256"]) != 64:
                raise EvidenceError(f"bad sha256 for {name}")
            if name in seen:
                raise EvidenceError(f"{name} listed twice")
            seen.add(name)
    except EvidenceError as error:
        raise EvidenceError(f"{path}: {error}") from None
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        raise EvidenceError(f"{path}: unreadable manifest ({error!r})") from None
    if root is not None:
        location = Path(path).resolve().parent
        try:
            actual = PurePosixPath(
                location.relative_to(Path(root).resolve()).as_posix()
            )
        except ValueError:
            actual = None
        if actual != unit:
            raise EvidenceError(f"{path}: manifest for {unit} is not in that directory")
    return manifest


def read_bundle(manifest, bundle_path):
    """Check a bundle against its manifest and return {repo path: (bytes, mode)}."""
    try:
        compressed = Path(bundle_path).read_bytes()
        archive = gzip.decompress(compressed)
    except (OSError, EOFError, zlib.error) as error:
        raise EvidenceError(f"{bundle_path}: unreadable bundle ({error})") from None
    record = manifest["bundle"]
    if (
        sha256(compressed) != record["sha256"]
        and sha256(archive) != record["tar_sha256"]
    ):
        raise EvidenceError(
            f"{bundle_path}: SHA-256 matches neither the bundle nor its tar stream"
        )
    unit = PurePosixPath(manifest["unit"])
    expected = {str(unit / entry["path"]): entry for entry in manifest["files"]}
    contents = {}
    try:
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
            for member in tar:
                name = PurePosixPath(member.name)
                if not member.isreg() or name.is_absolute() or ".." in name.parts:
                    raise EvidenceError(f"{bundle_path}: unsafe member {member.name!r}")
                entry = expected.get(str(name))
                if entry is None or str(name) in contents:
                    raise EvidenceError(
                        f"{bundle_path}: unexpected member {member.name}"
                    )
                data = tar.extractfile(member).read()
                if len(data) != entry["size"] or sha256(data) != entry["sha256"]:
                    raise EvidenceError(
                        f"{bundle_path}: {member.name} does not match the manifest"
                    )
                contents[str(name)] = (data, 0o755 if member.mode & 0o111 else 0o644)
    except (tarfile.TarError, EOFError) as error:
        raise EvidenceError(f"{bundle_path}: unreadable tar ({error})") from None
    missing = sorted(expected.keys() - contents.keys())
    if missing:
        raise EvidenceError(
            f"{bundle_path}: missing {len(missing)} files, first {missing[0]}"
        )
    return contents


def destination_state(root, name, data, mode):
    """'create', 'same' or 'differs'; refuses symbolic links and non-files on the path."""
    current = Path(root)
    parts = PurePosixPath(name).parts
    for index, part in enumerate(parts):
        current = current / part
        try:
            status = os.lstat(current)
        except FileNotFoundError:
            return "create"
        if stat.S_ISLNK(status.st_mode):
            raise EvidenceError(f"{name}: {current} is a symbolic link")
        last = index == len(parts) - 1
        if not last and not stat.S_ISDIR(status.st_mode):
            raise EvidenceError(f"{name}: {current} is not a directory")
        if last and not stat.S_ISREG(status.st_mode):
            raise EvidenceError(f"{name}: {current} is not a regular file")
    with open(current, "rb") as stream:
        same = stream.read() == data
    executable = bool(status.st_mode & 0o100)
    return "same" if same and executable == bool(mode & 0o100) else "differs"


TEMPORARY = re.compile(r"^\..+\.evidence-restore-[0-9a-f]{16}$")


def open_directory(root, parts, expected=None, create=False, created=None):
    """Open root/parts one component at a time, never following a link.

    Returns (fd, ids) with the (device, inode) of each directory walked. With
    expected, every directory must still be the one recorded; with create,
    missing directories are made and appended to created as (parts, ids).
    """
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    ids = []
    try:
        for index, part in enumerate(parts):
            made = False
            if create:
                try:
                    os.mkdir(part, 0o755, dir_fd=directory)
                    made = True
                except FileExistsError:
                    pass
            child = os.open(
                part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory
            )
            os.close(directory)
            directory = child
            status = os.fstat(directory)
            ids.append((status.st_dev, status.st_ino))
            if expected is not None and ids[-1] != expected[index]:
                raise EvidenceError(f"{'/'.join(parts[: index + 1])} was replaced")
            if made:
                created.append((parts[: index + 1], list(ids)))
        return directory, ids
    except BaseException:
        os.close(directory)
        raise


def entry_id(directory, name):
    """(device, inode) of a regular file in an open directory, without following."""
    try:
        status = os.stat(name, dir_fd=directory, follow_symlinks=False)
    except FileNotFoundError:
        return None
    return (status.st_dev, status.st_ino) if stat.S_ISREG(status.st_mode) else None


def remove_stale_temporaries(directory):
    """Delete regular files left by an interrupted forced restore in this directory."""
    for name in os.listdir(directory):
        if TEMPORARY.match(name) and entry_id(directory, name):
            os.unlink(name, dir_fd=directory)


def write_file(root, name, data, mode, replace, created_dirs):
    """Write one file below root without following links; return an undo record.

    A new file is created exclusively. With replace, the data goes to a unique
    temporary file, the old file is kept as a hard-linked backup, and the
    temporary file is renamed over it.
    """
    parts = PurePosixPath(name).parts
    directory, ids = open_directory(root, parts[:-1], create=True, created=created_dirs)
    leaf = parts[-1]
    token = secrets.token_hex(8)
    target = f".{leaf}.evidence-restore-{token}" if replace else leaf
    backup = None
    created = False
    try:
        if replace:
            remove_stale_temporaries(directory)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        handle = os.open(target, flags, 0o600, dir_fd=directory)
        created = True
        with os.fdopen(handle, "wb") as stream:
            stream.write(data)
            os.fchmod(stream.fileno(), mode)
        if replace:
            backup = f".{leaf}.evidence-backup-{token}"
            os.link(
                leaf,
                backup,
                src_dir_fd=directory,
                dst_dir_fd=directory,
                follow_symlinks=False,
            )
            os.replace(target, leaf, src_dir_fd=directory, dst_dir_fd=directory)
        return {
            "parts": parts,
            "dirs": ids,
            "file": entry_id(directory, leaf),
            "backup": backup,
        }
    except BaseException:
        if created and entry_id(directory, target):
            os.unlink(target, dir_fd=directory)
        if backup and entry_id(directory, backup):
            os.unlink(backup, dir_fd=directory)
        raise
    finally:
        os.close(directory)


def undo(root, records, created_dirs):
    """Reverse what restore wrote, through directory handles; return what was skipped."""
    skipped = []
    for record in reversed(records):
        parts = record["parts"]
        try:
            directory, _ = open_directory(root, parts[:-1], expected=record["dirs"])
        except (OSError, EvidenceError):
            skipped.append("/".join(parts))
            continue
        try:
            if entry_id(directory, parts[-1]) != record["file"]:
                skipped.append("/".join(parts))
            elif record["backup"]:
                os.replace(
                    record["backup"],
                    parts[-1],
                    src_dir_fd=directory,
                    dst_dir_fd=directory,
                )
            else:
                os.unlink(parts[-1], dir_fd=directory)
        finally:
            os.close(directory)
    for parts, ids in reversed(created_dirs):
        try:
            directory, _ = open_directory(root, parts[:-1], expected=ids[:-1])
        except (OSError, EvidenceError):
            continue
        try:
            status = os.stat(parts[-1], dir_fd=directory, follow_symlinks=False)
            if (status.st_dev, status.st_ino) == ids[-1]:
                os.rmdir(parts[-1], dir_fd=directory)
        except OSError:
            pass
        finally:
            os.close(directory)
    return skipped


def drop_backups(root, records):
    for record in records:
        if not record["backup"]:
            continue
        parts = record["parts"]
        try:
            directory, _ = open_directory(root, parts[:-1], expected=record["dirs"])
        except (OSError, EvidenceError):
            continue
        try:
            if entry_id(directory, record["backup"]):
                os.unlink(record["backup"], dir_fd=directory)
        finally:
            os.close(directory)


def restore_all(root, manifest_paths, bundles, force=False):
    """Restore bundles into root: check everything first, write, undo on failure.

    Returns (files, written, replaced names). A failure part-way removes the
    files this run created and puts replaced files back.
    """
    root = Path(root).resolve()
    actions = {}
    for path in manifest_paths:
        manifest = load_manifest(path, root)
        bundle = Path(bundles) / manifest["bundle"]["name"]
        for name, (data, mode) in read_bundle(manifest, bundle).items():
            if name in actions:
                raise EvidenceError(f"{name} is in more than one bundle")
            actions[name] = (data, mode)
    plan = []
    for name, (data, mode) in sorted(actions.items()):
        state = destination_state(root, name, data, mode)
        if state == "differs" and not force:
            raise EvidenceError(
                f"{name} exists with different content; use --force to replace it"
            )
        plan.append((name, data, mode, state))
    records, created_dirs = [], []
    try:
        for name, data, mode, state in plan:
            if state != "same":
                records.append(
                    write_file(root, name, data, mode, state == "differs", created_dirs)
                )
    except (OSError, EvidenceError) as error:
        skipped = undo(root, records, created_dirs)
        note = f"; left in place because they changed: {skipped}" if skipped else ""
        raise EvidenceError(f"{error}; restore undone{note}") from None
    except BaseException:
        undo(root, records, created_dirs)
        raise
    drop_backups(root, records)
    replaced = [name for name, _, _, state in plan if state == "differs"]
    return len(plan), len(records), replaced


def restore(root, manifest_path, bundles, force=False):
    return restore_all(root, [manifest_path], bundles, force)[0]


def manifests(root, selected):
    if selected:
        paths = [Path(item) for item in selected]
        return [p / MANIFEST if p.is_dir() else p for p in paths]
    return existing_manifests(root)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=ROOT,
        help="repository tree (default: this checkout)",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    build_cmd = commands.add_parser(
        "build", help="bundle raw files, write manifests, remove raw files"
    )
    build_cmd.add_argument(
        "--out", type=Path, required=True, help="directory for the bundles"
    )
    build_cmd.add_argument(
        "--release",
        required=True,
        help="GitHub release tag the bundles will be uploaded to",
    )
    build_cmd.add_argument(
        "--keep-files", action="store_true", help="leave the raw files in the tree"
    )
    build_cmd.add_argument(
        "units", nargs="*", help="unit directories (default: every unit)"
    )
    verify = commands.add_parser("verify", help="check bundles against their manifests")
    restore_cmd = commands.add_parser(
        "restore", help="extract bundles back into the tree"
    )
    for sub in (verify, restore_cmd):
        sub.add_argument(
            "--bundles", type=Path, required=True, help="directory holding the bundles"
        )
        sub.add_argument(
            "manifests", nargs="*", help="manifests or unit directories (default: all)"
        )
    restore_cmd.add_argument(
        "--force", action="store_true", help="replace files that differ"
    )
    commands.add_parser(
        "check", help="list tracked evidence files that break the size/type policy"
    )
    args = parser.parse_args(argv)
    root = args.root.resolve()
    try:
        if args.command == "build":
            units = []
            for unit in args.units:
                try:
                    relative = Path(unit).resolve().relative_to(root)
                except ValueError:
                    raise EvidenceError(f"{unit}: not under {root}") from None
                units.append(check_unit(relative.as_posix()))
            built = build(
                root,
                units or all_units(root),
                args.out,
                args.release,
                remove=not args.keep_files,
            )
            for manifest in built:
                record = manifest["bundle"]
                print(
                    f"{record['name']}  {len(manifest['files'])} files  {record['size']} bytes"
                )
            print(f"Built {len(built)} bundles in {args.out}")
        elif args.command == "verify":
            found = manifests(root, args.manifests)
            count = 0
            for path in found:
                manifest = load_manifest(path, root)
                bundle = args.bundles / manifest["bundle"]["name"]
                count += len(read_bundle(manifest, bundle))
            print(f"Verified {count} files from {len(found)} bundles")
        elif args.command == "restore":
            found = manifests(root, args.manifests)
            count, written, replaced = restore_all(
                root, found, args.bundles, force=args.force
            )
            for name in replaced:
                print(f"replaced {name}")
            print(
                f"Restored {count} files from {len(found)} bundles "
                f"({count - written} already in place, {len(replaced)} replaced)"
            )
        else:
            found = tracked_violations(root)
            if found is None:
                raise EvidenceError(f"{root} is not a git repository")
            print("\n".join(found) or "Evidence policy check passed.")
            return 1 if found else 0
    except (EvidenceError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
