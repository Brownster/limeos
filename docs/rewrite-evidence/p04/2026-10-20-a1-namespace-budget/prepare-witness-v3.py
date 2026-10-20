# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Preserve the witness with current command diagnostics and kernel errors."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

state = Path(__file__).resolve().parent
root = state.parents[1]
guest = state / "proc-witness-v3-guest"
shutil.copytree(state / "proc-witness-v2-guest", guest)
for name in ["guest.py", "p04_approved_guest.py", "p04_storage_guest.py"]:
    shutil.copyfile(root / "tests/privileged_vm" / name, guest / name)
wrapper = guest / "namespace_review_guest.py"
text = wrapper.read_text()
old = 'approved.main(package_version="0.4.4+namespace.2")'
assert text.count(old) == 1
text = text.replace(old, '''try:
    approved.main(package_version="0.4.4+namespace.2")
except Exception:
    import subprocess
    result = subprocess.run(["dmesg", "--level=emerg,alert,crit,err"], text=True, capture_output=True, timeout=5)
    print("POST_FAILURE_KERNEL_DIAGNOSTIC " + result.stdout[-16000:], flush=True)
    raise''')
wrapper.write_text(text)
manifest = {"executor_source": "0934eb421d5f4d91f20484d9e543d02cf800059b", "fixture_source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(), "packages": "diagnostic-candidate/inputs.json", "guest_memory_mib": 768, "assertions_or_runtime_changed": False, "guest": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(guest.glob("*.py"))}}
(state / "proc-witness-v3-inputs.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps({"guest": str(guest), "files": len(manifest["guest"])}))
