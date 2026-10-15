# Media activity browser evidence

Tested source `814e9a23e41186e56c162f7f3a4bde4fe5607091`, based on activity leaf
handoff `e1d8b04`. This packet qualifies local frontend admission and request/cache
behavior only. See the [handoff](../../../p04-media-activity-client-handoff.md).

- `inputs.sha256`: 31 tracked inputs, including all frontend sources/configuration,
  lockfile, generated shared types and the referenced Rust model/wire fixture.
- `checks.json`: exact commands, source SHA, times and exit statuses. Product
  checks pass: strict TypeScript, 29 client cases, ten view cases and whitespace.
- `client-qualified.log.gz`, `view-qualified.log.gz`, `typescript-final.log.gz`:
  final product results. There are no skipped/cancelled tests.
- `npm-tree-real.json.gz` and version logs: actual local dependencies. The
  dependency directory's package.json/package-lock.json match the worktree.
- `client-first.log.gz`, `client-test-first.mjs.gz` and
  `body-bound-diagnosis.log.gz`: the test's initial character/byte boundary error.
  Earlier corrected 27-case and reviewed 29-case logs are retained separately;
  they are not additional tests to add to the final count.
- `npm-tree-symlink-error.log.gz`, `qualification.log.gz` and
  `metadata-retry.log.gz`: failed npm traversal through the dependency symlink
  followed by successful metadata-only correction. Product suites were not
  rerun for this reporting correction; their source/input hashes stayed exact.
- `check.mjs.gz`: source-bound sequential check helper. Its `--metadata-only`
  recovery validates previous passing product results and unchanged inputs.
- `artifacts.sha256`: packet artifact hashes, excluding itself.

Reproduce with matching installed dependencies from the checked-in frontend
lockfile, then run from the worktree:

```sh
./frontend/node_modules/.bin/tsc --noEmit -p frontend/tsconfig.json
node frontend/tests/media-activity-client.test.mjs
node frontend/tests/media-activity.test.mjs
sha256sum -c docs/rewrite-evidence/p04/2026-10-15-media-activity-client/inputs.sha256
```

The local run borrowed main's existing dependency directory through a temporary
symlink; it was removed afterward. No new packages or production files were
installed. If using such a link, run `npm ls --all --json` in the actual package
directory rather than the symlinked worktree.

No Rust test result is reassigned to this SHA. Rust activity/API files are
unchanged; their [previous packet](../2026-10-15-media-activity/README.md) retains
107 Rust tests at its own qualified source. Static render and controller tests
do not constitute installed/browser, collector, ARM64, hardware, performance or
production acceptance. Shared registration and independent qualification remain
required.
