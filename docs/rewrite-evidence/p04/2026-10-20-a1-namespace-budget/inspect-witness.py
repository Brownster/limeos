# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Extract the completed A1 failure and its observational task differences."""
import json
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[2]
label = sys.argv[1]
directory = root / ".cache/main-consolidation/attempts" / label
record = json.loads((directory / "record.json").read_text())
assert "exit_code" in record, "Attempt remains live"
rows = [json.loads(line.removeprefix("A1_FAILURE ")) for line in (directory / "output.log").read_text().splitlines() if line.startswith("A1_FAILURE ")]
assert len(rows) == 1, "No complete A1 failure packet"
value = rows[0]
target = Path(__file__).resolve().parent / f"{label}-failure.json"
target.write_text(json.dumps(value, indent=2) + "\n")
witness = value.get("after_refusal_facts", {}).get("pre_effect_task_witness", {})
def identities(sample):
    return {(row["pid"], row["tid"], row["start"], tuple(row["namespace"])): row for row in sample.get("user_tasks", [])}
before = identities(witness.get("before", {}))
after = identities(witness.get("after", {}))
summary = {"stage": value["stage"], "error": value["error"], "phases": re.findall(r'"phase":"([^"]+)"', value.get("unit_diagnostics", "")), "witness_complete": {name: sample.get("complete_without_observed_read_or_membership_errors") for name, sample in witness.items()}, "removed_userspace_tasks": [before[key] for key in sorted(before.keys() - after.keys())], "added_userspace_tasks": [after[key] for key in sorted(after.keys() - before.keys())]}
print(json.dumps(summary, indent=2))
