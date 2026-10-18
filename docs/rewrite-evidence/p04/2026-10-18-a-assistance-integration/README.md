# Stream A assistance — combined local evidence

Tested source `8b31f990103bdc7ce959048abba1b6c051fc847b`, branch
`engineer/p04-a-assistance`, base `7df3f7530865c31b7faefd1f7b12a25aa7655860`.
See the [integration handoff](../../../p04-a-assistance-integration.md) for
included heads, central ownership and remaining installed acceptance.

| Check | Passed |
| --- | ---: |
| Actual domain unit suite | 178 |
| Actual archive library | 87 |
| Archive admission integration | 32 |
| Before-image integration | 2 |
| Custody integration | 11 |
| Preparation integration | 11 |
| Archive compile-fail doctests | 5 |
| Domain compile-fail doctests | 5 |
| Rust total | 331 |
| Frontend original API assertions | 4 |
| Automation controller / client-view | 14 / 10 |
| Media activity client / view | 29 / 10 |
| Observations / stack discovery | 4 / 18 |
| Mount lifecycle | 40 |
| Frontend total | 129 |

All final Rust cases passed with zero failed/ignored/filtered cases. These are
selected-crate checks, not all workspace or installed executor tests. Strict
all-target Clippy, changed-source Rustfmt, TypeScript and the actual main UI
production build passed. Four source equivalence comparisons prove the 14
product paths are unchanged from the included branch heads after the merges.
No conflict resolution or product edit was needed.

Rust commands use `--offline --locked`, selected packages `limeos-domain` and
`limeos-backup-archive`, one job and one test thread. Clippy uses `--all-targets
-- -D warnings`. Frontend files run directly and sequentially through the command
tool, preserving real TAP case counts. The API scratch entry retains every
original assertion and substitutes only its native TypeScript import with the
actual API source transpiled by pinned TypeScript 5.7.3. Its generator and source/
fixture hashes are retained. This follows the compiler-based module loading of
the existing frontend test helpers; no production or test assertion was edited.

`npm run build` ran TypeScript then Vite on the registered production entry.
It includes the real mount component/controller, 22 modules, producing 174.52 kB
JavaScript and 5.39 kB CSS in this local build. Artifact sizes are not installed
footprint or performance measurements. Browser/guest/package/ARM64/physical/
resource/recovery qualification is unrun in this packet. Other active owners'
worktrees/harnesses were not modified or executed.

`inputs.sha256` binds selected Rust source, frontend/type inputs, all tracked
fixtures and TOML manifests/settings as a conservative superset, plus Cargo.lock.
`manifest.json` records source heads, checks, toolchains, pinned frontend
dependencies and environment. `changed-paths.txt.gz` records the full delta;
filtering out documentation yields exactly the 14 allowed product paths.
`artifacts.sha256` binds logs, scripts, lockfile copy, metadata and actual bundle.
No product source changed after checks. The temporary dependency symlink was
removed after metadata capture; scratch and targets remain on repository disk.

This candidate changes no shared runtime, authority, Store/schema/protocol,
worker budgets, root exports, generated contracts or activation. Old leaf
packets remain bound to their own individual sources; their checksum lists do
not describe this combined source. Current installed A1 acceptance and complete
A2–A5/A3b remain open, as do P04/P05/P06/cutover requirements and the historical
61.401 ms / 20 ms inventory failure. No production host was contacted.

Verify packet artifacts here with `sha256sum -c artifacts.sha256`. From the
worktree root verify this packet's `inputs.sha256`.
