# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture frozen local gates losslessly. No test host access."""
import datetime
import gzip
import hashlib
import json
import pathlib
import subprocess
import sys
import time

root = pathlib.Path('/home/marc/Documents/github/lime-os')
tree = root / '.worktrees/arm64-ci-inputs'
ready = root / '.cache/arm64-qual/integrated-2026-10-12/ci-ready-2026-10-13'
out = ready / 'local-validation'
out.mkdir()
files = [str(path.relative_to(tree)) for path in sorted((tree / 'tests/qualification/arm64').glob('*.py'))]
commands = {
    'debian-python311-harness': ['docker', 'run', '--rm', '--network', 'none', '--user', '1000:1000', '-v', f'{root}:{root}:ro', '-v', f'{root}/.cache/tmp:/scratch', '-e', 'TMPDIR=/scratch', '-w', str(tree), 'limeos-arm64-repo-tools:local', 'python3', '-m', 'unittest', 'discover', '-s', 'tests/qualification/arm64', '-p', 'test_*.py'],
    'ruff-check': ['uv', 'tool', 'run', '--offline', 'ruff', 'check', *files],
    'ruff-format': ['uv', 'tool', 'run', '--offline', 'ruff', 'format', '--check', *files],
    'repository-hygiene': [sys.executable, 'scripts/check_repository.py'],
    'offline-build-validation': [sys.executable, '-c', 'from qualification import load_build; import json; b=load_build("' + str(ready / 'prepared/build-result.json') + '", "' + str(ready / 'repo') + '"); print(json.dumps({"identity":b["identity"],"packages":b["packages"],"installed_qualification":"untested"}))'],
}
records = []
for name, command in commands.items():
    started = time.monotonic()
    result = subprocess.run(command, cwd=tree, capture_output=True, check=False)
    raw = result.stdout + result.stderr
    target = out / (name + '.log.gz')
    target.write_bytes(gzip.compress(raw, mtime=0))
    records.append({'name': name, 'command': command, 'exit': result.returncode, 'seconds': round(time.monotonic()-started, 3), 'log': target.name, 'raw_sha256': hashlib.sha256(raw).hexdigest(), 'compressed_sha256': hashlib.sha256(target.read_bytes()).hexdigest()})
    print(f'{name}: exit {result.returncode}', flush=True)
    if result.returncode:
        print(raw.decode(errors='replace'))
        raise SystemExit(result.returncode)
result = {'fixture_commit': subprocess.check_output(['git','rev-parse','HEAD'], cwd=tree, text=True).strip(), 'recorded_at': datetime.datetime.now(datetime.UTC).isoformat(), 'pi_accessed': False, 'rust_rebuilt': False, 'checks': records}
(out / 'checks.json').write_text(json.dumps(result, indent=2) + '\n')
