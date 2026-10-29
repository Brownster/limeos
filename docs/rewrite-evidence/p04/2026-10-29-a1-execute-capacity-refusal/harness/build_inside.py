# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Build the exact frozen candidate inside the existing offline Debian image."""
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1])
manifest = out / 'inputs.sha256'
with (out / 'source-before.log').open('wb') as log:
    subprocess.run(['sha256sum', '-c', str(manifest)], stdout=log, stderr=subprocess.STDOUT, check=True)
subprocess.run(['rustc', '--version', '--verbose'], check=True)
subprocess.run(['cargo', 'build', '--offline', '--locked', '--release', '-p', 'limeos-core', '-p', 'limeos-executor', '-p', 'limeosctl'], check=True)
with (out / 'source-after.log').open('wb') as log:
    subprocess.run(['sha256sum', '-c', str(manifest)], stdout=log, stderr=subprocess.STDOUT, check=True)
names = ['limeos-core', 'limeos-password-worker', 'limeos-executor', 'limeos-stack-read-worker', 'limeosctl']
binaries = out / 'binaries'
binaries.mkdir()
records = {}
for name in names:
    source = Path('/build/.cache/a1-namespace-review/debian-target/release') / name
    target = binaries / name
    shutil.copyfile(source, target)
    target.chmod(0o755)
    records[name] = {'sha256': hashlib.sha256(target.read_bytes()).hexdigest(), 'bytes': target.stat().st_size}
    subprocess.run(['ldd', str(target)], check=True)
(out / 'binaries.json').write_text(json.dumps(records, indent=2) + '\n')
assert records['limeos-stack-read-worker']['bytes'] <= 8 * 1024**2
print(json.dumps(records), flush=True)
