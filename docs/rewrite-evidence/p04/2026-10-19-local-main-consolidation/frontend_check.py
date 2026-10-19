# /// script
# dependencies = []
# ///
"""Check existing frontend inputs sequentially with a bounded Node heap."""

import json
import re
import subprocess
from pathlib import Path

root = Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/local-main-consolidation/frontend'
state = root / '.cache/main-consolidation/frontend'
state.mkdir(exist_ok=True)
commands = [('typescript', ['node', '--max-old-space-size=256', str(work / 'node_modules/typescript/bin/tsc'), '--noEmit', '-p', str(work / 'tsconfig.json')])]
commands += [(path.name, ['node', '--max-old-space-size=256', '--experimental-strip-types', str(path)]) for path in sorted((work / 'tests').glob('*.mjs'))]
results = []
for name, command in commands:
    with (state / f'{name}.log').open('wb') as log:
        result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT, check=False)
    output = (state / f'{name}.log').read_text()
    counts = {key: int(value) for key, value in re.findall(r'^# (tests|pass|fail) (\d+)$', output, re.M)}
    entry = {'name': name, 'argv': command, 'exit_code': result.returncode, 'counts': counts}
    results.append(entry)
    (state / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(entry), flush=True)
    if result.returncode:
        print(output[-8000:], flush=True)
        raise SystemExit(result.returncode)
print(json.dumps({'passed': sum(row['counts'].get('pass', 0) for row in results), 'test_files': len(results) - 1}), flush=True)
