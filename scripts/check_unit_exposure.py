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
   base revision (by default the merge base with the default branch, for a pull
   request and a push alike), plus the unit's recorded ALLOWANCE if it has one.
   A unit new since the base has only check 2.
2. Backstop: each unit scores no higher than its recorded ceiling plus MARGIN.

On the default branch itself (HEAD is its own merge base with it) there is
nothing to compare with, so the gate judges by check 2 alone: the recorded
ceilings, with no relative comparison. It never compares with HEAD's parent or
a push's previous tip: a worsening pushed once (its red run ignored) would then
be the baseline the next push is compared with. The ceilings, unlike a parent,
change only in a reviewed edit of this file. ALLOWANCES are relative to a base,
so they do not apply there; a ceiling records the score with any accepted
loosening already in it. A unit that is deliberately looser than on the default
branch gets an explicit, commented ALLOWANCE, and the gate says when one is no
longer needed.

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
    # PR-32: 3.5 as root with no capability; 3.4 as limeos-storage-reader
    # with CAP_DAC_READ_SEARCH, the docker group and RemoveIPC (systemd 258).
    "limeos-storage-reader.service": 34,
    "limeos-storage-ready.service": 32,
    "limeos-storage-targets.service": 36,
}
# Tenths a unit may score above the same file at the base, for a loosening that
# was reviewed and accepted but is not on the default branch yet. Each entry
# says what costs it and where it was accepted; remove it once the change is
# on the default branch (the gate prints a reminder when it goes unused).
ALLOWANCES: dict[str, tuple[int, str]] = {
    # RestrictSUIDSGID=no (0.2 under systemd 258): its seccomp filter rejects
    # openat2, which core's protected socket lookup needs (ce2e916, "permit
    # protected reader lookup in Core profile"; the reason is in the unit).
    "limeos-core.service": (2, "RestrictSUIDSGID=no for openat2 (ce2e916)"),
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
    allowances: dict[str, tuple[int, str]] | None = None,
) -> list[str]:
    """Problems with the unit files in `units`, compared with those in `base`
    (the same files at the base revision; None skips the comparison), each
    allowed its entry in `allowances` (tenths, reason) above the base."""
    allowances = {} if allowances is None else allowances
    problems = []
    present = sorted(path.name for path in units.glob("*.service"))
    for name in sorted(set(ceilings) - set(present)):
        problems.append(f"{name}: ceiling recorded for a unit that does not exist")
    for name in sorted(set(allowances) - set(present)):
        problems.append(f"{name}: allowance recorded for a unit that does not exist")
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
        allowance, reason = allowances.get(name, (0, ""))
        if exposure > before + allowance:
            allowed = f" plus its {allowance / 10:.1f} allowance" if allowance else ""
            problems.append(
                f"{name}: exposure {exposure / 10:.1f} is worse than the base's "
                f"{before / 10:.1f}{allowed} under the same systemd-analyze"
            )
        elif exposure > before:
            print(f"  {name} is {(exposure - before) / 10:.1f} above the base, allowed: {reason}")
        elif allowance:
            print(f"  {name} no longer needs its allowance ({reason}); remove it.")
        if exposure <= before and exposure < ceiling:
            print(f"  {name} scores below its ceiling; lower the ceiling to keep the gain.")
    return problems


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=False
    )


# resolve_base's answer when HEAD is on the default branch: no base exists, so
# the recorded ceilings alone apply.
ON_DEFAULT = "the default branch"


def resolve_base(explicit: str | None) -> str | None:
    """The base revision: `explicit`, else the merge base of HEAD with the pull
    request's target (GITHUB_BASE_REF) or the default branch (DEFAULT_BRANCH,
    else main; its origin copy first). ON_DEFAULT when that merge base is HEAD
    itself (HEAD is on the default branch: nothing to compare with). None when
    no merge base can be found."""
    if explicit:
        return explicit
    target = os.environ.get("GITHUB_BASE_REF")
    default = os.environ.get("DEFAULT_BRANCH") or "main"
    candidates = ([f"origin/{target}"] if target else []) + [f"origin/{default}", default]
    head = git("rev-parse", "HEAD").stdout.strip()
    for candidate in candidates:
        found = git("merge-base", "HEAD", candidate)
        if found.returncode != 0:
            continue
        merge_base = found.stdout.strip()
        return ON_DEFAULT if merge_base == head else merge_base
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
            if revision is ON_DEFAULT:
                print(
                    "HEAD is on the default branch: judging by the recorded ceilings "
                    "alone, with no relative comparison."
                )
            else:
                print(f"Comparing with {revision}.")
                base = base_units(revision, Path(directory))
        problems = check(args.units, CEILINGS, base, allowances=ALLOWANCES)
    for problem in problems:
        print(problem, file=sys.stderr)
    if not problems:
        compared = "no worse than the base and " if base is not None else ""
        print(f"Unit exposure scores are {compared}within their ceilings.")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
