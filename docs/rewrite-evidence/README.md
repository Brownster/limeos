# Rewrite evidence

Each directory here records one qualification run: a README that says what
was run and what it showed, hash lists, and small result files. Summaries and
hashes stay in git. Raw logs, captures and archives are kept as GitHub release
assets, one bundle per run, and each bundled run keeps a tracked
`EVIDENCE-BUNDLE.json` that lists every moved file with its size and SHA-256.

## What stays in git

- Markdown: READMEs, trackers, reviews. Up to 128 KiB a file.
- Hash lists: `SHA256SUMS`, `*.sha256`, `EVIDENCE-BUNDLE.json`. Up to 128 KiB.
- Any other file of 8 KiB or less that is not a raw type: result and summary
  JSON, short text, small scripts.
- Files that code reads, rewrites in place or cites, whatever their size. They
  are listed with the reason in `PINNED` in `scripts/evidence_bundles.py`, for
  example `p03/lifecycle-progress.png`, which the CI browser check rewrites,
  and every file the `tests/privileged_vm/record_*_evidence.py` recorders read
  or write in their default and documented invocations (inputs, and the hash
  and result summaries they rewrite). A recorder declares these in
  `evidence_inputs()` and `evidence_outputs()` and touches evidence only
  through them; `tests/evidence` fails if a recorder reads or writes any other
  evidence path, if a declared file is bundled or not pinned, or if code names
  a bundled path.

## What is raw

Everything else: compressed files and archives (`.gz`, `.tar`, `.zst`, `.xz`,
`.zip`), logs (`.log`, `.jsonl`, `.stdout`, `.stderr`, `.out`), screenshots and
images, disk and package images, and any other file over 8 KiB. The full list
is `RAW_SUFFIXES` in `scripts/evidence_bundles.py`.
`python3 scripts/check_repository.py` fails when a tracked file here breaks
these rules.

## Bundles

A unit is one run directory, `<area>/<run>/` with everything below it, or the
loose files directly in `<area>/`. Its bundle is
`limeos-evidence-<area>-<run>.tar.gz` (or `limeos-evidence-<area>.tar.gz`;
a `-` inside an area name is doubled, so `current-build` gives
`limeos-evidence-current--build.tar.gz`, and run names may not start with `-`):
a gzip-compressed tar with members sorted by path, stored under their
repository paths, with modification time 0, owner and group 0 and the git file
mode. The same files give the same tar bytes. The manifest records the release
tag, the bundle's SHA-256 and the SHA-256 of its uncompressed tar stream (the
compressed bytes can differ between zlib builds; the tar stream does not).

The first batch, built on 2026-11-19, is release `evidence-bundles-2026-11-19`.

## Fetch, verify and restore

Read the release tag and bundle name from the run's `EVIDENCE-BUNDLE.json`, then:

```bash
gh release download evidence-bundles-2026-11-19 --dir dist/evidence-bundles \
  --pattern limeos-evidence-p04-2026-10-20-a1-namespace-budget.tar.gz
python3 scripts/evidence_bundles.py verify --bundles dist/evidence-bundles \
  docs/rewrite-evidence/p04/2026-10-20-a1-namespace-budget
python3 scripts/evidence_bundles.py restore --bundles dist/evidence-bundles \
  docs/rewrite-evidence/p04/2026-10-20-a1-namespace-budget
```

Without unit arguments, `verify` and `restore` handle every manifest. Restore
checks every bundle and destination before writing, never follows a symbolic
link, and refuses to replace a file with different content unless given
`--force`, which lists each file it replaced. If a write fails, restore
removes the files it created and puts replaced files back, and names any
entry it could not undo. It holds each file it writes or replaces, and each
directory on the way, open until it finishes, so a file someone puts in place
of one of its own is never mistaken for it, even where the filesystem reuses
inode numbers. A full restore therefore needs a few thousand descriptors;
restore raises the soft limit as far as the hard limit allows and stops before
writing if that is not enough. Only one build or restore runs at a time. If an
interrupted run left `.<name>.evidence-restore-…` (unfinished copy) or
`.<name>.evidence-backup-…` (what `--force` replaced) files, restore lists
them and stops; it never deletes them itself. A run's
`SHA256SUMS` then verifies as it did before the move. Scripts kept inside a
run directory (such as a `verify.py`) read that run's raw files, so restore
its bundle first. Restored files are for reading; do not commit them.

## Recording a new run

Write the run into a new directory, add its README and hash list, then bundle
it before committing:

```bash
python3 scripts/evidence_bundles.py build --release evidence-bundles-YYYY-MM-DD \
  --out dist/evidence-bundles docs/rewrite-evidence/<area>/<run>
```

This moves the raw files into the bundle and writes `EVIDENCE-BUNDLE.json`.
Commit the directory, upload the bundle to that release, and keep the bundle
file unchanged: its SHA-256 is in the manifest. CI steps that write evidence
here (VM results, browser checks, gate probes) write to the working tree only
and are not committed.
