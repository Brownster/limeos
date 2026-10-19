# /// script
# dependencies = []
# ///
"""Relocate archived preview assets without changing their tested bytes."""

import gzip
import hashlib
import json
from pathlib import Path

root = Path('/home/marc/Documents/github/lime-os/.worktrees/local-main-consolidation')
for name in ('2026-10-17-network-read-ui', '2026-10-17-tools-read-ui'):
    packet = root / 'docs/rewrite-evidence/p04' / name
    original = (packet / 'artifacts.sha256').read_bytes()
    readme = (packet / 'README.md').read_bytes()
    mappings = []
    for line in original.decode().splitlines():
        digest, path = line.split('  ', 1)
        assert hashlib.sha256((packet / path).read_bytes()).hexdigest() == digest, path
        if path.startswith('preview/dist/'):
            mappings.append({'before': path, 'after': path.replace('preview/dist/', 'preview/production-ui/', 1), 'sha256': digest})
    assert len(mappings) == 3
    (packet / 'preview/dist').rename(packet / 'preview/production-ui')
    (packet / 'artifacts-before-relocation.sha256.gz').write_bytes(gzip.compress(original, mtime=0))
    (packet / 'README-before-relocation.md.gz').write_bytes(gzip.compress(readme, mtime=0))
    notice = '\nIntegration relocation, 19 October 2026: published preview assets now live in\n`preview/production-ui/` to satisfy repository hygiene. All three payload hashes\nare unchanged. `relocation.json` records the mapping; the original README and\nartifact manifest remain compressed alongside this record. Reproduction still\nuses the original archived build configuration and a scratch output directory.\n'
    (packet / 'README.md').write_bytes(readme + notice.encode())
    (packet / 'relocation.json').write_text(json.dumps({'date': '2026-10-19', 'original_manifest_sha256': hashlib.sha256(original).hexdigest(), 'original_readme_sha256': hashlib.sha256(readme).hexdigest(), 'mappings': mappings, 'payload_bytes_changed': False}, indent=2) + '\n')
    entries = []
    for line in original.decode().splitlines():
        _, path = line.split('  ', 1)
        path = path.replace('preview/dist/', 'preview/production-ui/', 1)
        entries.append(path)
    entries += ['artifacts-before-relocation.sha256.gz', 'README-before-relocation.md.gz', 'relocation.json']
    (packet / 'artifacts.sha256').write_text(''.join(f'{hashlib.sha256((packet / path).read_bytes()).hexdigest()}  {path}\n' for path in sorted(entries)))
    print(f'{name}: verified three unchanged payloads and retained original manifest/README')
