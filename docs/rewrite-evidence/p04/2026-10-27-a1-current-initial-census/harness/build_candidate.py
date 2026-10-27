# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""One source-bound offline build with a host memory/disk stop and owned cleanup."""
import datetime
import hashlib
import importlib.util
import json
import re
import shutil
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/stack-installed-qualification'
OUT = STATE / 'build-6'
IMAGE = 'sha256:af684f63adae359e95f9082cc3235bcc76208baaa9cd1a48b553784e226d5ba5'
spec = importlib.util.spec_from_file_location('guards', ROOT / '.cache/main-consolidation/guard.py')
guards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guards)
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
assert not [r for r in subprocess.check_output(['ps', '-eo', 'comm'], text=True).splitlines() if r.strip() in ('cargo', 'rustc', 'rustdoc')]
floor, disk_floor = 3 * 1024**3, 8 * 1024**3
assert guards.available() >= floor and shutil.disk_usage(ROOT).free >= disk_floor
OUT.mkdir(exist_ok=False)
before = guards.snapshot()
(OUT / 'inputs.sha256').write_text(''.join(f'{value}  /build/{name}\n' for name, value in sorted(before.items())))
record = {'source': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), 'image': IMAGE, 'started': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'before': before, 'memory_floor_bytes': floor, 'disk_floor_bytes': disk_floor, 'minimum_available_bytes': guards.available(), 'minimum_disk_free_bytes': shutil.disk_usage(ROOT).free, 'stop_reason': None, 'builder_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), 'inside_sha256': hashlib.sha256((STATE / 'build_inside.py').read_bytes()).hexdigest()}
metadata = OUT / 'record.json'
container = child = None
def docker(*args, check=True):
    return subprocess.run(['docker', *args], text=True, capture_output=True, check=check)
def interrupt(signum, frame):
    raise KeyboardInterrupt(signum)
signal.signal(signal.SIGTERM, interrupt)
signal.signal(signal.SIGINT, interrupt)
command = ['create', '--name', 'limeos-stack-runtime-build-6', '--label', 'limeos.qualification=stack-runtime-build-6', '--network=none', '--read-only', '--memory=2g', '--memory-swap=2g', '--pids-limit=64', '--cpus=1', '--cap-drop=ALL', '--security-opt=no-new-privileges', '--security-opt=label=disable', '--user', '1000:1000', '--volume', str(ROOT)+':/build:ro']
for relative in ('.cache/p04-a1-integration/cargo-home', '.cache/a1-namespace-review/debian-target', '.cache/stack-installed-qualification'):
    command += ['--volume', str(ROOT / relative)+':/build/'+relative]
command += ['--workdir', '/build', '--env', 'CARGO_HOME=/build/.cache/p04-a1-integration/cargo-home', '--env', 'CARGO_TARGET_DIR=/build/.cache/a1-namespace-review/debian-target', '--env', 'CARGO_BUILD_JOBS=1', '--env', 'CARGO_INCREMENTAL=0', '--env', 'TMPDIR=/build/.cache/stack-installed-qualification/tmp', IMAGE, 'python3', '/build/.cache/stack-installed-qualification/build_inside.py', '/build/.cache/stack-installed-qualification/build-6']
record['create_command'] = ['docker', *command]
try:
    container = docker(*command).stdout.strip()
    assert re.fullmatch('[0-9a-f]{64}', container)
    actual = json.loads(docker('inspect', container).stdout)[0]
    assert actual['Image'] == IMAGE
    record['container'] = container
    (OUT / 'container-before.json').write_text(json.dumps(actual, indent=2)+'\n')
    metadata.write_text(json.dumps(record, indent=2)+'\n')
    with (OUT / 'output.log').open('wb') as log:
        child = subprocess.Popen(['docker', 'start', '--attach', container], stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic()+1200
        while child.poll() is None:
            memory, disk = guards.available(), shutil.disk_usage(ROOT).free
            record['minimum_available_bytes'] = min(record['minimum_available_bytes'], memory)
            record['minimum_disk_free_bytes'] = min(record['minimum_disk_free_bytes'], disk)
            if memory < floor or disk < disk_floor or time.monotonic() >= deadline:
                record['stop_reason'] = 'memory floor' if memory < floor else 'disk floor' if disk < disk_floor else 'build deadline'
                docker('stop', '--time', '1', container)
                break
            time.sleep(0.25)
        record['attach_exit_code'] = child.wait(timeout=15)
    actual = json.loads(docker('inspect', container).stdout)[0]
    record['exit_code'] = actual['State']['ExitCode']
    record['oom_killed'] = actual['State']['OOMKilled']
    (OUT / 'container-after.json').write_text(json.dumps(actual, indent=2)+'\n')
finally:
    if container:
        actual = json.loads(docker('inspect', container).stdout)[0]
        assert actual['Image'] == IMAGE and actual['Config']['Labels']['limeos.qualification'] == 'stack-runtime-build-6'
        if actual['State']['Running']:
            docker('stop', '--time', '1', container)
        if child:
            child.wait(timeout=15)
        docker('rm', container)
        record['container_removed'] = docker('inspect', container, check=False).returncode != 0
    record['after'] = guards.snapshot()
    record['inputs_unchanged'] = record['before'] == record['after']
    record['ended'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    metadata.write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps({key: record.get(key) for key in ('source', 'exit_code', 'stop_reason', 'oom_killed', 'minimum_available_bytes', 'minimum_disk_free_bytes', 'container_removed', 'inputs_unchanged')}), flush=True)
raise SystemExit(0 if record.get('exit_code') == 0 and record['stop_reason'] is None and record['inputs_unchanged'] and record.get('container_removed') else 1)
