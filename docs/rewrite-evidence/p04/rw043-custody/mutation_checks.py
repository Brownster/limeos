#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Weaken one custody guarantee at a time and record which tests catch it.

uv run --offline mutation_checks.py --target-dir /scratch/target --output /fresh/dir

Each mutant is an exact text replacement that must match once. Sources are
restored byte-for-byte after every run, and the script refuses to start with
uncommitted changes, so a crash cannot leave a mutant behind silently. The
64 MiB lifecycle test is skipped to bound run time; it has no unique guard.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
CUSTODY = "crates/backup-archive/src/custody.rs"
IDENTITY = "crates/backup-archive/src/custody/identity.rs"

# (name, guarantee, expected, [(file, old, new), ...])
MUTANTS = [
    ("recover-ignores-policy", "recovery compares full current policy", "killed", [
        (CUSTODY, "    if record.policy != policy\n        || identity::policy(&policy, limits.max_record_bytes)? != expected.policy_identity\n    {",
         "    if false {"),
    ]),
    ("recover-policy-identity-only", "full equality and identity are independent guards", "survives", [
        (CUSTODY, "    if record.policy != policy\n        ||", "    if false\n        ||"),
    ]),
    ("recover-policy-equality-only", "full equality and identity are independent guards", "survives", [
        (CUSTODY, "        || identity::policy(&policy, limits.max_record_bytes)? != expected.policy_identity\n    {",
         "    {"),
    ]),
    ("stage-ignores-policy", "staging rechecks full policy before replay", "killed", [
        (CUSTODY, "        if policy != self.admitted.policy_snapshot().policy()\n            || identity::policy(policy, self.limits.max_record_bytes)?\n                != self.binding.policy_identity\n        {",
         "        if false {"),
    ]),
    ("record-not-canonical", "records must be the exact canonical encoding", "killed", [
        (CUSTODY, "        || encode_record(&record.binding, &record.policy, &record.manifest)? != bytes\n", ""),
    ]),
    ("record-binding-unchecked", "the saved binding must equal the trusted binding", "killed", [
        (CUSTODY, "Ok(record.binding == *expected", "Ok(true"),
    ]),
    ("foreign-entries-allowed", "record directories hold only owned names", "killed", [
        (CUSTODY, "    exact_entries(&directory)?;\n", ""),
    ]),
    ("hard-links-allowed", "sealed objects have exactly one link", "killed", [
        (CUSTODY, "        && stat.st_nlink == 1\n", ""),
    ]),
    ("copy-unbounded", "the copy stops one sentinel byte past its limit", "killed", [
        (CUSTODY, "            .filter(|c| *c <= limit)\n", ""),
    ]),
    ("ambiguous-durability-hidden", "a failed post-rename root fsync is reported", "killed", [
        (CUSTODY, "    if let Err(source) = io.sync(&pending.root, Barrier::Root) {\n        return Err(CustodyError::AmbiguousDurability {\n            binding: Box::new(binding),\n            source,\n        });\n    }",
         "    let _ = io.sync(&pending.root, Barrier::Root);"),
    ]),
    ("readmission-compares-sorted-entries-only", "recovery requires exact compressed identity", "killed", [
        (CUSTODY, "    if *manifest != record.manifest\n        || manifest.archive_sha256 != expected.archive_sha256\n",
         "    if manifest.entries != record.manifest.entries\n        || false\n"),
        (CUSTODY, "        || identity::manifest(manifest, limits.max_record_bytes)? != expected.manifest_identity\n    {\n        return Err(CustodyError::BindingMismatch);",
         "    {\n        return Err(CustodyError::BindingMismatch);"),
    ]),
    ("held-identity-unchecked", "staging is bound to the held, still-published inode", "killed", [
        (CUSTODY, "        self.check_held()?;\n        let catalog", "        let catalog"),
        (CUSTODY, "        if let Err(failure) = self.check_held() {", "        if let Err(failure) = Ok::<(), CustodyError>(()) {"),
    ]),
    ("discard-unlinks-any-inode", "cleanup removes only owned inodes", "killed", [
        (CUSTODY, "        Ok(stat) if same_inode(&stat, identity) => {\n            rustix::fs::unlinkat(directory, name, AtFlags::empty())",
         "        Ok(_) => {\n            rustix::fs::unlinkat(directory, name, AtFlags::empty())"),
    ]),
    ("pending-never-cleaned", "pre-publication failures remove owned state", "killed", [
        (CUSTODY, "    fn cleanup(&mut self) -> Result<(), CleanupFailure> {\n        if !self.armed {",
         "    fn cleanup(&mut self) -> Result<(), CleanupFailure> {\n        if true {"),
    ]),
    ("rename-replaces", "publication never replaces an existing name", "killed", [
        (CUSTODY, "rustix::fs::renameat_with(root, from, root, to, RenameFlags::NOREPLACE)",
         "rustix::fs::renameat_with(root, from, root, to, RenameFlags::empty())"),
    ]),
    ("identity-without-length-prefix", "identity strings are length-prefixed", "killed", [
        (IDENTITY, "        self.u64(value.len() as u64);\n", ""),
    ]),
]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if subprocess.run(["git", "diff", "--quiet", "HEAD"], cwd=ROOT).returncode:
        raise SystemExit("refusing: uncommitted changes")
    args.output.mkdir(parents=True, exist_ok=False)
    env = {**os.environ, "CARGO_TARGET_DIR": str(args.target_dir), "CARGO_NET_OFFLINE": "true"}
    command = ["cargo", "test", "-p", "limeos-backup-archive", "--locked", "--offline",
               "--lib", "--test", "custody", "--", "--skip", "64_mib"]
    result = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "command": command,
        "mutants": [],
    }
    originals = {}
    for name, guarantee, expected, edits in MUTANTS:
        for path, _, _ in edits:
            originals.setdefault(path, (ROOT / path).read_bytes())
        for path, old, new in edits:
            text = (ROOT / path).read_text()
            if text.count(old) != 1:
                raise SystemExit(f"{name}: target text matched {text.count(old)} times in {path}")
            (ROOT / path).write_text(text.replace(old, new))
        try:
            log = args.output / f"{name}.txt"
            with log.open("w") as stream:
                run = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT)
            output = log.read_text()
            failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED$", output, re.M)))
            compiled = "error[" not in output
            outcome = "killed" if run.returncode and compiled else "survives" if not run.returncode else "build-failed"
            result["mutants"].append({
                "name": name, "guarantee": guarantee, "expected": expected, "outcome": outcome,
                "as_expected": outcome == expected, "exit_code": run.returncode,
                "failed_tests": failed, "log": log.name,
                "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
            })
            print(f"{name}: {outcome} ({len(failed)} failing tests)", flush=True)
        finally:
            for path, data in originals.items():
                (ROOT / path).write_bytes(data)
    if subprocess.run(["git", "diff", "--quiet", "HEAD"], cwd=ROOT).returncode:
        raise SystemExit("sources not restored")
    result["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    result["all_as_expected"] = all(m["as_expected"] for m in result["mutants"])
    (args.output / "mutations.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    raise SystemExit(0 if result["all_as_expected"] else 1)


if __name__ == "__main__":
    main()
