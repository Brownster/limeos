# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Add synchronous before/after task diagnostics outside the actual RPC scan."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

state = Path(__file__).resolve().parent
guest = state / "proc-witness-v2-guest"
guest.mkdir()
root = state.parents[1]
for path in (root / "tests/privileged_vm").glob("*.py"):
    shutil.copyfile(path, guest / path.name)
shutil.copyfile(state / "namespace_census.py", guest / "namespace_census.py")
(guest / "namespace_review_guest.py").write_text('''# No thread or monitor runs during the actual production RPC.
import p04_approved_guest as approved
import p04_mounts_guest as mounts
from namespace_census import snapshot

witness = {}
original_pre = mounts.pre_effect_refusals
original_after = mounts._after_refusal_facts

def pre_effect(*args):
    witness["before"] = snapshot()
    try:
        return original_pre(*args)
    finally:
        witness["after"] = snapshot()

def diagnostics(service):
    facts = original_after(service)
    facts["namespace_census"] = snapshot()
    facts["pre_effect_task_witness"] = witness
    return facts

mounts.pre_effect_refusals = pre_effect
mounts._after_refusal_facts = diagnostics
approved.main(package_version="0.4.4+namespace.2")
''')
manifest = {"executor_source": "0934eb421d5f4d91f20484d9e543d02cf800059b", "fixture_source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), "packages": "diagnostic-candidate/inputs.json", "scope": "synchronous diagnostic snapshots before/after existing A1 pre-effect assertions; corrected earlier target peer-UID probe; no concurrent monitor or changed runtime", "guest": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(guest.glob("*.py"))}}
(state / "proc-witness-v2-inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"guest": str(guest), "files": len(manifest["guest"]), "scope": manifest["scope"]}))
