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
import fnmatch
import gzip
import hashlib
import io
import json
import os
import subprocess
import sys
import tarfile
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
# Files that code reads, rewrites in place or cites as provenance. They stay
# tracked whatever their size or type; keep the reason next to each pattern.
PINNED = {
    "docs/rewrite-evidence/p01/gate-probes.json": "written by scripts/test_gates.py in CI",
    "docs/rewrite-evidence/p01/vm-result.json": "default output of tests/privileged_vm/run.py",
    "docs/rewrite-evidence/p03/lifecycle-browser-result.json": "written by tests/frontend/restart.py in CI",
    "docs/rewrite-evidence/p03/lifecycle-progress.png": "written by tests/frontend/restart.py in CI",
    "docs/rewrite-evidence/p03/compose-validation.json": "default output of tests/privileged_vm/record_compose_evidence.py",
    "docs/rewrite-evidence/p03/lifecycle-validation.json": "default output of tests/privileged_vm/record_p03_evidence.py",
    "docs/rewrite-evidence/p03/compose-source-sha256.json": "read by tests/privileged_vm/record_storage_evidence.py",
    "docs/rewrite-evidence/p04/storage-*-validation.json": "default output of tests/privileged_vm/record_storage_evidence.py",
    "docs/rewrite-evidence/p04/rw043/measurement-policy.json": "read by tests/qualification/arm64/inspector_guest.py",
    "docs/rewrite-evidence/p04/rw043-custody/measure_custody.py": "generator cited by tests/fixtures/backup-custody/README.md",
    "docs/rewrite-evidence/p04/rw043-staging/measure_staging.py": "generator cited by tests/fixtures/backup-staging/README.md",
    "docs/rewrite-evidence/p04/2026-10-08-library-handoff-integration/ci-passed-37746766303/ci-summary.json": "provenance cited by tests/fixtures/rw040-combined/packages-2fd7420.json",
    "docs/rewrite-evidence/p04/2026-10-06-handoff-integration/previous-schema7-artifact.json": "cited by tests/qualification/arm64/README.md",
    "docs/rewrite-evidence/arm64/2026-11-05-pi-night-prep/harness/*": "scripts cited by tests/qualification/arm64/README.md",
}


class EvidenceError(Exception):
    pass


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def is_pinned(rel):
    return any(fnmatch.fnmatchcase(str(rel), pattern) for pattern in PINNED)


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
        if not path.is_file():
            continue
        reason = problem(rel, path.stat().st_size)
        if reason:
            found.append(f"{rel}: {reason}")
    return found


def unit_of(rel):
    parts = PurePosixPath(rel).relative_to(EVIDENCE).parts
    if len(parts) < 2:
        return None
    return EVIDENCE.joinpath(*parts[:2]) if len(parts) > 2 else EVIDENCE / parts[0]


def bundle_name(unit):
    return (
        BUNDLE_PREFIX
        + "-".join(PurePosixPath(unit).relative_to(EVIDENCE).parts)
        + BUNDLE_SUFFIX
    )


def unit_files(root, unit):
    """Repository-relative files belonging to a unit, excluding its manifest."""
    base = Path(root) / unit
    loose = len(PurePosixPath(unit).relative_to(EVIDENCE).parts) == 1
    paths = base.iterdir() if loose else base.rglob("*")
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


def tar_bytes(root, files):
    """Deterministic tar of repository-relative files, plus a manifest entry for each."""
    buffer = io.BytesIO()
    entries = []
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for rel in sorted(files, key=str):
            path = Path(root) / rel
            data = path.read_bytes()
            info = tarfile.TarInfo(str(rel))
            info.size = len(data)
            info.mtime = MTIME
            info.mode = 0o755 if os.access(path, os.X_OK) else 0o644
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


def build_unit(root, unit, out, release, remove=True):
    """Bundle a unit's raw files; return the manifest, or None when nothing is raw."""
    root = Path(root)
    raw = [
        rel
        for rel in unit_files(root, unit)
        if problem(rel, (root / rel).stat().st_size)
    ]
    if not raw:
        return None
    manifest_path = root / unit / MANIFEST
    if manifest_path.exists():
        raise EvidenceError(
            f"{unit} already has {MANIFEST}; record new evidence in a new run directory"
        )
    archive, entries = tar_bytes(root, raw)
    compressed = gzip_bytes(archive)
    name = bundle_name(unit)
    Path(out).mkdir(parents=True, exist_ok=True)
    (Path(out) / name).write_bytes(compressed)
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
    manifest_path.write_text(render_manifest(manifest))
    if remove:
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
    return manifest


def load_manifest(path):
    manifest = json.loads(Path(path).read_text())
    if manifest.get("format") != FORMAT:
        raise EvidenceError(
            f"{path}: unknown manifest format {manifest.get('format')!r}"
        )
    return manifest


def read_bundle(manifest, bundle_path):
    """Check a bundle against its manifest and return {repo path: (bytes, mode)}."""
    compressed = Path(bundle_path).read_bytes()
    record = manifest["bundle"]
    archive = gzip.decompress(compressed)
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
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
        for member in tar:
            name = PurePosixPath(member.name)
            if not member.isreg() or name.is_absolute() or ".." in name.parts:
                raise EvidenceError(f"{bundle_path}: unsafe member {member.name!r}")
            entry = expected.get(str(name))
            if entry is None or str(name) in contents:
                raise EvidenceError(f"{bundle_path}: unexpected member {member.name}")
            data = tar.extractfile(member).read()
            if len(data) != entry["size"] or sha256(data) != entry["sha256"]:
                raise EvidenceError(
                    f"{bundle_path}: {member.name} does not match the manifest"
                )
            contents[str(name)] = (data, 0o755 if member.mode & 0o111 else 0o644)
    missing = sorted(expected.keys() - contents.keys())
    if missing:
        raise EvidenceError(
            f"{bundle_path}: missing {len(missing)} files, first {missing[0]}"
        )
    return contents


def restore(root, manifest_path, bundles, force=False):
    manifest = load_manifest(manifest_path)
    contents = read_bundle(manifest, Path(bundles) / manifest["bundle"]["name"])
    for name, (data, mode) in contents.items():
        path = Path(root) / name
        if path.exists() and not force and path.read_bytes() != data:
            raise EvidenceError(
                f"{name} exists with different content; use --force to replace it"
            )
    for name, (data, mode) in sorted(contents.items()):
        path = Path(root) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(mode)
    return len(contents)


def manifests(root, selected):
    if selected:
        paths = [Path(item) for item in selected]
        return [p / MANIFEST if p.is_dir() else p for p in paths]
    return sorted((Path(root) / EVIDENCE).rglob(MANIFEST))


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
    build = commands.add_parser(
        "build", help="bundle raw files, write manifests, remove raw files"
    )
    build.add_argument(
        "--out", type=Path, required=True, help="directory for the bundles"
    )
    build.add_argument(
        "--release",
        required=True,
        help="GitHub release tag the bundles will be uploaded to",
    )
    build.add_argument(
        "--keep-files", action="store_true", help="leave the raw files in the tree"
    )
    build.add_argument(
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
            units = [
                PurePosixPath(Path(u).resolve().relative_to(root).as_posix())
                for u in args.units
            ]
            names = {}
            for unit in units or all_units(root):
                manifest = build_unit(
                    root, unit, args.out, args.release, remove=not args.keep_files
                )
                if manifest:
                    name = manifest["bundle"]["name"]
                    if name in names:
                        raise EvidenceError(
                            f"bundle name {name} used by {names[name]} and {unit}"
                        )
                    names[name] = unit
                    print(
                        f"{name}  {len(manifest['files'])} files  {manifest['bundle']['size']} bytes"
                    )
            print(f"Built {len(names)} bundles in {args.out}")
        elif args.command in ("verify", "restore"):
            found = manifests(root, args.manifests)
            count = 0
            for path in found:
                if args.command == "verify":
                    manifest = load_manifest(path)
                    count += len(
                        read_bundle(manifest, args.bundles / manifest["bundle"]["name"])
                    )
                else:
                    count += restore(root, path, args.bundles, force=args.force)
            verb = "Verified" if args.command == "verify" else "Restored"
            print(f"{verb} {count} files from {len(found)} bundles")
        else:
            tracked = (
                subprocess.run(
                    ["git", "ls-files", "-z", "--", str(EVIDENCE)],
                    cwd=root,
                    capture_output=True,
                    check=True,
                )
                .stdout.decode()
                .split("\0")
            )
            found = violations(root, [name for name in tracked if name])
            print("\n".join(found) or "Evidence policy check passed.")
            return 1 if found else 0
    except (EvidenceError, OSError, tarfile.TarError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
