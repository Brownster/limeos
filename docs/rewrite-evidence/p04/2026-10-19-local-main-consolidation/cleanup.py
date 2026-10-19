# /// script
# dependencies = []
# ///
"""Remove only clean, merged worktrees and branches; preserve ignored evidence caches."""

import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'


def git(*args, check=True, cwd=ROOT):
    return subprocess.run(['git', '-C', str(cwd), *args], capture_output=True, text=True, check=check)


def ancestor(head, target):
    result = git('merge-base', '--is-ancestor', head, target, check=False)
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr)
    return result.returncode == 0


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--candidate', default='main')
parser.add_argument('--apply', action='store_true')
args = parser.parse_args()
if args.apply and args.candidate != 'main':
    parser.error('Apply requires actual main ancestry')
if git('status', '--porcelain=v1', '--untracked-files=all').stdout:
    raise RuntimeError('Main must be clean before cleanup')
result = {'target': git('rev-parse', args.candidate).stdout.strip(), 'apply': args.apply, 'eligible': [], 'retained': [], 'removed_worktrees': [], 'removed_branches': []}
for block in git('worktree', 'list', '--porcelain').stdout.strip().split('\n\n'):
    fields = dict(line.split(' ', 1) for line in block.splitlines() if ' ' in line)
    path = Path(fields['worktree'])
    if path == ROOT:
        continue
    dirty = git('status', '--porcelain=v1', '--untracked-files=all', cwd=path).stdout
    merged = ancestor(fields['HEAD'], args.candidate)
    item = {**fields, 'dirty': dirty, 'merged': merged}
    if dirty or not merged or 'locked' in fields:
        result['retained'].append(item)
        continue
    result['eligible'].append(item)
    if args.apply:
        cache = path / '.cache'
        if cache.exists() or cache.is_symlink():
            destination = STATE / 'retired-caches' / path.name
            destination.parent.mkdir(exist_ok=True)
            if destination.exists():
                raise RuntimeError(f'Retained cache already exists: {destination}')
            cache.rename(destination)
            item['retained_cache'] = str(destination)
        git('worktree', 'remove', str(path))
        result['removed_worktrees'].append(item)
        (STATE / 'cleanup-result.json').write_text(json.dumps(result, indent=2) + '\n')
if args.apply:
    branches = git('for-each-ref', '--format=%(refname:short)', 'refs/heads').stdout.splitlines()
    live = git('worktree', 'list', '--porcelain').stdout
    for branch in branches:
        if branch == 'main' or f'branch refs/heads/{branch}\n' in live:
            continue
        head = git('rev-parse', branch).stdout.strip()
        if not ancestor(head, 'main'):
            raise RuntimeError(f'Unmerged branch preserved: {branch}')
        git('branch', '-d', branch)
        result['removed_branches'].append({'branch': branch, 'head': head})
        (STATE / 'cleanup-result.json').write_text(json.dumps(result, indent=2) + '\n')
(STATE / ('cleanup-result.json' if args.apply else 'cleanup-plan.json')).write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'apply': args.apply, 'eligible_worktrees': len(result['eligible']), 'removed_worktrees': len(result['removed_worktrees']), 'removed_branches': len(result['removed_branches']), 'retained': [{'path': item['worktree'], 'entries': len(item['dirty'].splitlines()), 'merged': item['merged']} for item in result['retained']]}, indent=2))
