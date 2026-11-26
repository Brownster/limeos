#!/usr/bin/env python3
"""Build the cutover's state list and live list from a `limeosctl migrate scan --json` report.

Runbook step 1 (docs/p06-cutover-runbook.md). Reads only the report; writes only the two lists.

The state list holds the frozen app's own files, which must not change while the cutover runs
(preflight.sh --state-list): every regular file the scan found, except
  * the Compose files (location stacks),
  * what the LimeOS package itself owns and changes (core.json, management-catalog.json and
    system-policy/ under etc; core/, executors/ and assistant/ under state),
  * anything inside a Python virtual environment (the scan reports one as a single `virtualenv`
    entry; an older scan listed its files, which are skipped by their pyvenv.cfg),
  * anything a --live entry covers.
The live list (preflight.sh --live-list) holds the --live entries, each after a comment line with
its reason: state that services the cutover leaves running write. return.sh reports a change
there and never fails on it. A summary of what went where is printed, never file contents.
"""

import argparse
import json
import sys
from collections import Counter
from pathlib import PurePosixPath

PACKAGE_OWNED = {
    "etc": ("core.json", "management-catalog.json", "system-policy/"),
    "state": ("core/", "executors/", "assistant/"),
}


def parse_live(values):
    entries = []
    for value in values:
        path, sep, reason = value.partition("=")
        if not sep or not path.startswith("/") or not reason.strip() or "\n" in value:
            raise SystemExit(f"--live needs ABSOLUTE_PATH=REASON (a path ending in / for a directory): {value!r}")
        entries.append((path, reason.strip()))
    return entries


def covered(path, entries):
    for entry, reason in entries:
        if entry.endswith("/") and path.startswith(entry):
            return reason
        if path in (entry, entry + "-wal", entry + "-shm", entry + "-journal"):
            return reason
    return None


def build(report, live):
    locations = {item["label"]: item["path"].rstrip("/") for item in report["locations"]}
    files = report["files"]
    venvs = {(f["location"], f["path"]) for f in files if f["kind"] == "virtualenv"}
    venvs |= {(f["location"], str(PurePosixPath(f["path"]).parent)) for f in files if PurePosixPath(f["path"]).name == "pyvenv.cfg"}
    state, skipped, kept = [], Counter(), Counter()
    for f in files:
        location, relative = f["location"], f["path"]
        if location == "stacks":
            skipped["Compose files under the stacks directory"] += 1
            continue
        if f["kind"] == "virtualenv":
            skipped[f"virtual environment {locations[location]}/{relative} (one entry, not descended)"] += 1
            continue
        if f["kind"] != "file":
            skipped[f"{f['kind']} (not a regular file)"] += 1
            continue
        if relative.startswith(PACKAGE_OWNED.get(location, ())):
            skipped["files of the LimeOS package"] += 1
            continue
        inside = next((f"{locations[location]}/{root}" for loc, root in venvs if loc == location and relative.startswith(root + "/")), None)
        if inside:
            skipped[f"files inside the virtual environment {inside}"] += 1
            continue
        path = f"{locations[location]}/{relative}"
        if "\n" in path:
            raise SystemExit(f"a path holds a newline, which a list cannot record: {path!r}")
        reason = covered(path, live)
        if reason:
            skipped[f"live ({reason})"] += 1
            continue
        state.append(path)
        kept[location] += 1
    return sorted(set(state)), skipped, kept


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--scan", required=True, help="the limeosctl migrate scan --json report")
    parser.add_argument("--state-out", required=True, help="the state list to write")
    parser.add_argument("--live-out", required=True, help="the live list to write")
    parser.add_argument("--live", action="append", default=[], metavar="PATH=REASON",
                        help="state a service the cutover leaves running writes (repeatable; PATH/ for a directory)")
    args = parser.parse_args(argv)
    with open(args.scan, encoding="utf-8") as handle:
        report = json.load(handle)
    live = parse_live(args.live)
    state, skipped, kept = build(report, live)
    with open(args.state_out, "x", encoding="utf-8") as handle:
        handle.write("".join(p + "\n" for p in state))
    with open(args.live_out, "x", encoding="utf-8") as handle:
        for path, reason in live:
            handle.write(f"# {reason}\n{path}\n")
    print(f"state list {args.state_out}: {len(state)} file(s) of the frozen app: " + ", ".join(f"{k} {v}" for k, v in sorted(kept.items())))
    print(f"live list {args.live_out}: {len(live)} entr{'y' if len(live) == 1 else 'ies'}: " + ", ".join(p for p, _ in live))
    for what, count in sorted(skipped.items()):
        print(f"left out: {count} {what}")
    if not report.get("complete", False):
        print("WARN: the scan report is incomplete (see its problems): the state list may miss files", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
