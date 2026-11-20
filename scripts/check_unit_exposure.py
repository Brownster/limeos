#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fail if a packaged systemd unit's exposure score gets worse.

Scores come from `systemd-analyze security --offline=true` (systemd 250 or newer),
which reads the unit file alone: no install, no root, no running manager.

Systemd versions weigh settings differently (the ceilings below were recorded
with systemd 258; Ubuntu 24.04 ships 255), so a fixed number alone would fail
spuriously or pass vacuously. Two checks, both with the same systemd-analyze in
the same run:

1. Must not get worse: each unit scores no higher than the same unit file at the
   base revision (by default the merge base with origin/main, or the previous
   commit when HEAD is on main). A unit new since the base has only check 2.
2. Backstop: each unit scores no higher than its recorded ceiling plus MARGIN.

A unit without a recorded ceiling fails, so a new unit is scored deliberately.
Ceilings are tenths of systemd's 0-10 exposure level (its --threshold scale).
Lower one when a unit is tightened; raising one needs the reason in the unit
file next to the setting that costs it.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
UNITS = "packaging/systemd"
CEILINGS = {
    "limeos-containerd.service": 21,
    "limeos-core.service": 25,
    "limeos-storaged.service": 4,
    "limeos-storage-mounts.service": 46,
    "limeos-storage-reader.service": 35,
    "limeos-storage-ready.service": 32,
    "limeos-storage-targets.service": 36,
}
# Tenths of slack on the backstop for scoring differences between systemd
# versions (a check one version adds or weighs differently moves a unit by a
# few tenths). The must-not-get-worse comparison has no slack.
MARGIN = 3
OVERALL = re.compile(r"Overall exposure level for \S+: (\d+)\.(\d)")


def score(unit: Path, analyze: str = "systemd-analyze") -> tuple[int | None, str]:
    """Return (exposure in tenths, output) for one unit file."""
    result = subprocess.run(
        [analyze, "security", "--offline=true", "--threshold=100", "--no-pager", str(unit)],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    output = result.stdout + result.stderr
    found = OVERALL.search(output)
    return (int(found[1]) * 10 + int(found[2]) if found else None), output


def check(
    units: Path,
    ceilings: dict[str, int],
    base: Path | None,
    analyze: str = "systemd-analyze",
    margin: int = MARGIN,
) -> list[str]:
    """Problems with the unit files in `units`, compared with those in `base`
    (the same files at the base revision; None skips the comparison)."""
    problems = []
    present = sorted(path.name for path in units.glob("*.service"))
    for name in sorted(set(ceilings) - set(present)):
        problems.append(f"{name}: ceiling recorded for a unit that does not exist")
    for name in present:
        if name not in ceilings:
            problems.append(f"{name}: no exposure ceiling recorded in {Path(__file__).name}")
            continue
        exposure, output = score(units / name, analyze)
        if exposure is None:
            problems.append(f"{name}: systemd-analyze gave no score:\n{output.strip()}")
            continue
        ceiling = ceilings[name]
        line = f"{name}: exposure {exposure / 10:.1f} (ceiling {ceiling / 10:.1f}"
        if exposure > ceiling + margin:
            problems.append(
                f"{name}: exposure {exposure / 10:.1f} is above its ceiling "
                f"{ceiling / 10:.1f} plus the {margin / 10:.1f} margin"
            )
        if base is None:
            print(line + ")")
            continue
        if not (base / name).is_file():
            print(line + "; new since the base, ceiling only)")
            continue
        before, output = score(base / name, analyze)
        if before is None:
            problems.append(f"{name}: systemd-analyze gave the base no score:\n{output.strip()}")
            continue
        print(line + f"; base {before / 10:.1f})")
        if exposure > before:
            problems.append(
                f"{name}: exposure {exposure / 10:.1f} is worse than the base's "
                f"{before / 10:.1f} under the same systemd-analyze"
            )
        elif exposure < ceiling:
            print(f"  {name} scores below its ceiling; lower the ceiling to keep the gain.")
    return problems


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=False
    )


def resolve_base(explicit: str | None) -> str | None:
    """The base revision: `explicit`, else the merge base of HEAD with the pull
    request's target or origin/main (main when there is no remote), or HEAD's
    parent when HEAD is already on that branch. None when none can be found."""
    if explicit:
        return explicit
    target = os.environ.get("GITHUB_BASE_REF")
    candidates = ([f"origin/{target}"] if target else []) + ["origin/main", "main"]
    head = git("rev-parse", "HEAD").stdout.strip()
    for candidate in candidates:
        found = git("merge-base", "HEAD", candidate)
        if found.returncode != 0:
            continue
        merge_base = found.stdout.strip()
        if merge_base != head:
            return merge_base
        parent = git("rev-parse", "--verify", "--quiet", "HEAD^")
        return parent.stdout.strip() if parent.returncode == 0 else None
    return None


def base_units(revision: str, destination: Path) -> Path:
    """Write the unit files as they are at `revision` into `destination`."""
    listing = git("ls-tree", "--name-only", f"{revision}:{UNITS}")
    if listing.returncode != 0:
        raise SystemExit(f"cannot read {UNITS} at {revision}: {listing.stderr.strip()}")
    for name in listing.stdout.split():
        if name.endswith(".service"):
            shown = git("show", f"{revision}:{UNITS}/{name}")
            if shown.returncode != 0:
                raise SystemExit(f"cannot read {UNITS}/{name} at {revision}")
            (destination / name).write_text(shown.stdout)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--units", type=Path, default=ROOT / UNITS)
    parser.add_argument("--base", help="revision to compare with (default: see above)")
    parser.add_argument(
        "--no-base",
        action="store_true",
        help="skip the must-not-get-worse comparison (the backstop alone)",
    )
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        base = None
        if not args.no_base:
            revision = resolve_base(args.base)
            if revision is None:
                print(
                    "No base revision found: fetch origin/main (CI: fetch-depth 0), "
                    "pass --base REV, or --no-base for the backstop alone.",
                    file=sys.stderr,
                )
                return 1
            print(f"Comparing with {revision}.")
            base = base_units(revision, Path(directory))
        problems = check(args.units, CEILINGS, base)
    for problem in problems:
        print(problem, file=sys.stderr)
    if not problems:
        print("Unit exposure scores are no worse than the base and within their ceilings.")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
