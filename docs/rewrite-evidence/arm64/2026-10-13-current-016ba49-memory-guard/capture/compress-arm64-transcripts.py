# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
import gzip
import hashlib
import json
import pathlib
import subprocess

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
OUT = ROOT/'docs/rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard'
old = json.loads((OUT/'evidence-sha256.json').read_text())
records = {}
for path in sorted(OUT.rglob('*')):
    if path.suffix not in ('.txt','.log'):
        continue
    name = str(path.relative_to(OUT))
    data = path.read_bytes()
    raw_sha = hashlib.sha256(data).hexdigest()
    assert raw_sha == old[name]
    target = path.with_name(path.name+'.gz')
    assert not target.exists()
    compressed = gzip.compress(data,mtime=0)
    target.write_bytes(compressed)
    assert gzip.decompress(target.read_bytes()) == data
    records[name] = {'stored_as':str(target.relative_to(OUT)),'original_bytes':len(data),'original_sha256':raw_sha,'compressed_bytes':len(compressed),'compressed_sha256':hashlib.sha256(compressed).hexdigest()}
    path.unlink()
summary = {'source_raw_packet_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'format':'lossless gzip, mtime zero; raw CR/LF, ANSI and trailing bytes preserved','files':records}
(OUT/'raw-transcript-compression.json').write_text(json.dumps(summary,indent=2)+'\n')
print(f'PASS: {len(records)} raw transcripts compressed with verified byte-for-byte decompression')
