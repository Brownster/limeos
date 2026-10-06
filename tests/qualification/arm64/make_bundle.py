#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Archive frozen runtime and separately committed fixtures; build UI from that archive.

The bundle contains source/, fixtures/ and source-manifest.json. Frontend
commands run against archived source; logs remain next to the output bundle.
"""

import argparse
import io
import json
import subprocess
import tarfile
import tempfile
from pathlib import Path

from qualification import load_manifest, sha, verify_source

ROOT = Path(__file__).resolve().parents[3]


def archive(commit, prefix, paths=()):
    return subprocess.check_output(
        [
            "git",
            "-C",
            str(ROOT),
            "archive",
            "--format=tar",
            f"--prefix={prefix}/",
            commit,
            *paths,
        ]
    )


def extract(payload, root):
    with tarfile.open(fileobj=io.BytesIO(payload)) as source:
        source.extractall(root, filter="data")


def files(root, excluded=()):
    return {
        p.relative_to(root).as_posix(): sha(p)
        for p in sorted(root.rglob("*"))
        if p.is_file()
        and not p.is_symlink()
        and not any(part in excluded for part in p.relative_to(root).parts)
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commit", required=True, help="Frozen runtime commit")
    parser.add_argument(
        "--fixtures-commit", required=True, help="Committed harness and guest fixtures"
    )
    parser.add_argument(
        "--package-version", required=True, help="Distinct qualification Debian version"
    )
    parser.add_argument("--authority-schema", required=True, type=int)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    logs = args.output.with_suffix(".frontend-logs")
    if args.output.exists() or args.manifest.exists() or logs.exists():
        parser.error(
            "use new output, manifest and log paths; historical artifacts are immutable"
        )
    commits = {
        name: subprocess.check_output(
            ["git", "-C", str(ROOT), "rev-parse", ref + "^{commit}"], text=True
        ).strip()
        for name, ref in (
            ("source_commit", args.commit),
            ("fixtures_commit", args.fixtures_commit),
        )
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    logs.mkdir()
    with tempfile.TemporaryDirectory(prefix="limeos-arm64-bundle-") as temporary:
        work = Path(temporary)
        extract(archive(commits["source_commit"], "source"), work)
        extract(
            archive(
                commits["fixtures_commit"],
                "fixtures",
                ("tests/qualification/arm64", "tests/privileged_vm"),
            ),
            work,
        )
        source_files = files(work / "source")
        fixtures_files = files(work / "fixtures")
        frontend = work / "source/frontend"
        commands = [
            ["npm", "ci", "--ignore-scripts"],
            ["npm", "test"],
            ["npm", "run", "build"],
        ]
        steps = []
        for i, command in enumerate(commands):
            with (logs / f"{i}.txt").open("w") as log:
                result = subprocess.run(
                    command,
                    cwd=frontend,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    check=False,
                )
            steps.append(
                {"command": command, "exit": result.returncode, "log": f"{i}.txt"}
            )
            (logs / "steps.json").write_text(json.dumps(steps, indent=2) + "\n")
            if result.returncode:
                raise SystemExit(f"frontend gate failed: {command}; see {logs}")
        manifest = {
            "identity": {
                **commits,
                "package_version": args.package_version,
                "authority_schema": args.authority_schema,
                "architecture": "arm64",
                "qualification_kind": "native-kvm",
            },
            "runtime_source_sha256": source_files,
            "fixtures_sha256": fixtures_files,
            "frontend_dist_sha256": {
                "frontend/dist/" + n: s for n, s in files(frontend / "dist").items()
            },
            "frontend_build": {
                "steps": steps,
                "node": subprocess.check_output(
                    ["node", "--version"], text=True
                ).strip(),
            },
        }
        internal = work / "source-manifest.json"
        internal.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        load_manifest(internal)
        verify_source(work, manifest)
        with tarfile.open(args.output, "w:gz", compresslevel=1) as out:
            for prefix, paths in (
                ("source", {**source_files, **manifest["frontend_dist_sha256"]}),
                ("fixtures", fixtures_files),
            ):
                for name in sorted(paths):
                    out.add(work / prefix / name, arcname=f"{prefix}/{name}")
            out.add(internal, arcname="source-manifest.json")
        manifest["bundle_sha256"] = sha(args.output)
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        print(
            json.dumps(
                {
                    "identity": manifest["identity"],
                    "bundle_sha256": manifest["bundle_sha256"],
                }
            )
        )


if __name__ == "__main__":
    main()
