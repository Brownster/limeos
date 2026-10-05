#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bundle the frozen runtime source plus built frontend assets for a native guest build.

Runtime source comes from `git archive` of the frozen commit, never the working
tree, so uncommitted edits cannot enter the build. The frontend is plain
JavaScript and is built on the workstation; its bytes are hashed separately.
"""

import argparse
import hashlib
import io
import json
import subprocess
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    commit = subprocess.check_output(
        ["git", "-C", str(ROOT), "rev-parse", args.commit + "^{commit}"], text=True
    ).strip()
    source = subprocess.check_output(
        ["git", "-C", str(ROOT), "archive", "--format=tar", "--prefix=source/", commit]
    )
    dist = ROOT / "frontend/dist"
    if not (dist / "index.html").is_file():
        parser.error("Build frontend/dist from the same commit first.")
    files = {}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(args.output, "w:gz", compresslevel=1) as out:
        with tarfile.open(fileobj=io.BytesIO(source)) as archive:
            for member in archive.getmembers():
                data = archive.extractfile(member) if member.isfile() else None
                payload = data.read() if data else None
                if payload is not None:
                    files[member.name.removeprefix("source/")] = hashlib.sha256(
                        payload
                    ).hexdigest()
                out.addfile(
                    member, io.BytesIO(payload) if payload is not None else None
                )
        frontend = {}
        for path in sorted(p for p in dist.rglob("*") if p.is_file()):
            name = "frontend/dist/" + path.relative_to(dist).as_posix()
            frontend[name] = hashlib.sha256(path.read_bytes()).hexdigest()
            out.add(path, arcname="source/" + name)
    node = subprocess.check_output(["node", "--version"], text=True).strip()
    with args.output.open("rb") as stream:
        bundle = hashlib.file_digest(stream, "sha256").hexdigest()
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(
        json.dumps(
            {
                "commit": commit,
                "bundle_sha256": bundle,
                "runtime_source_files": len(files),
                "runtime_source_sha256": files,
                "frontend_dist_sha256": frontend,
                "frontend_build": f"npm ci --ignore-scripts && npm test && npm run build (node {node}, workstation)",
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    print(json.dumps({"commit": commit, "bundle_sha256": bundle, "files": len(files)}))


if __name__ == "__main__":
    main()
