# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Keep the complete VM harness, changing only the non-build guest RAM."""
import hashlib
import json
from pathlib import Path
import shutil

root = Path(__file__).resolve().parents[2]
state = Path(__file__).resolve().parent
original = root / "tests/privileged_vm/run.py"
text = original.read_text()
old = '"2048" if args.build_bundle else "1024"'
new = '"2048" if args.build_bundle else "768"'
assert text.count(old) == 1
runner = state / "run_768.py"
assert not runner.exists()
runner.write_text(text.replace(old, new))
helper = root / "tests/privileged_vm/current_authority_schema.py"
shutil.copyfile(helper, state / helper.name)
(state / "lowmem-runner.json").write_text(json.dumps({
    "original_runner": str(original.relative_to(root)),
    "original_sha256": hashlib.sha256(original.read_bytes()).hexdigest(),
    "runner": str(runner.relative_to(root)),
    "runner_sha256": hashlib.sha256(runner.read_bytes()).hexdigest(),
    "sole_change": {"from": old, "to": new},
    "guest_memory_mib": 768,
    "host_memory_floor_bytes": 3 * 1024**3,
    "service_ceilings_or_fixture_assertions_changed": False,
}, indent=2) + "\n")
print("768 MiB runner prepared; all fixture assertions and service ceilings preserved")
