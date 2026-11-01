# Local observation supervisor qualification

Qualified source: `d1ba5f0f052621e29073519682aa52af7c9e7bd3` on local main.
Design `ab30c44` precedes implementation. The
[handoff](../../../p04-b1-observation-supervisor-handoff.md) describes the owned
Core producer, fair owner/schedule cursors and authenticated runtime diagnostics.
This extends the separately qualified
[admission and original-job recovery path](../2026-10-31-b1-container-admission-recovery/README.md).

## Final acceptance

| Check | Result |
| --- | --- |
| Full Rust workspace with `limeos-executor/read-worker-probe` | 1,727 runtime passes, 54 compile-fail doctests |
| Focused final Core supervisor tests | 10 passes |
| Actual Core binary startup and restart | Pass; authenticated partial/unavailable status, production root-peer refusal |
| Strict workspace/all-target Clippy with the same feature | Pass, warnings denied |
| Actual generated-contract drift | Pass; four intended generated files checked in |
| Frontend tests and checked production build | 244 passes; TypeScript and Vite pass |
| Closed desktop/mobile browser fixture | Pass; served HTML/JS/CSS hashes match the passing build |
| Rust formatting, repository boundaries and diff hygiene | Pass |

Fifteen new runtime regressions cover three Store hints, ten Core supervisor
cases, one domain diagnostic contract and one actual Core binary scenario.
Three pre-existing outer ignored child entries are invoked by passing isolated
parents. Child summaries are counted once using the last result in each outer
Cargo group; the focused checks are not added to the workspace total.

Eight final guards (`rust-1`, `core-2`, `integration-3`, `clippy-1`, `contracts-1`,
`frontend-1`, `browser-2`, `sanity-1`) share identical before/after maps covering
738 committed inputs. Current worktree bytes match the qualified commit.
[Source-match metadata](source-match.json) records exact commands, Cargo groups,
all fifteen attempts and differences in earlier input maps.

Two failed fixtures are retained. `focused-1` attempted to alter an immutable
job intent: the existing trigger correctly refused it. The corrected test
asserts that refusal and tests the private bounded parser directly.
`integration-1` used the wrong cookie name; the fixture was corrected to the
existing production `__Host-limeos` cookie. Neither production guard was weakened.
`core-1` predates the strengthened actual-run fatal-stop regression; `schema-1`
generated output before its four changed files were installed. `integration-2`
passed before that installation. The final full run covers all final test bytes.
`browser-1` passed the same UI source before the fixture recorded actual served
asset hashes; `browser-2` adds that binding and fresh visual review. The initial
browser captures remain alongside the final captures.

Every run uses repository-disk scratch/cache, one Cargo build job and Rust test
thread, a 4-GiB admission threshold and the unchanged sampled 3-GiB stop floor.
No guard fired or external compiler overlapped. The lowest final sample is
4,941,295,616 available bytes. The final native observation finds all 34 recorded
guard/command/Core PIDs absent. Every attempt's guarded inputs stayed unchanged.

## Observed behavior

Query-only hints page 32 owners at a time and select each owner's oldest queued
job or next schedule. Real Store tests enforce query-only access, unchanged
SQLite total changes, bounds, cursor wrap, queue priority and immutable intent
guards. Hints provide no authority; existing admission reloads current state.

Owned Unix RPC fixtures prove 33 owners progress over two pages with one fresh
root-reader request per page. Three schedules belonging to one owner rotate
without repeatedly selecting its first row. Only the material baseline creates
a durable result; stable schedules use transition-only polling. Empty/paused
work contacts no protected reader. One owner's missing concrete scope refuses
its completion while another owner succeeds from the same live cohort.

The runtime preserves oldest original queued identities after source failure,
retries with a fresh acquisition and publishes through the existing in-app path.
Recovery returns an already stored original receipt without another request.
The source lifetime and earliest original permit deadline bound separate
per-owner transactions; no cross-owner atomicity or cached-source lifetime is
introduced. Fatal durable failure stops the actual owned run and the batch
before another owner is completed. Cancellation clears registration and the
bounded diagnostic map, leaving only original volatile permit expiry.

After one material baseline, 32 stable real runtime passes advance injected
time by 960 seconds. Actual database and WAL bytes remain identical, ledger
counts remain unchanged and `/proc/self/io` write-byte delta is zero. The
`core-2` log retains the measurement marker. This is an isolated runtime
measurement, not installed CPU/footprint qualification or a real 24-hour soak.

Authenticated typed Core calls project only current owned schedule authority.
Pause removes runtime details, explicit resume resets the revision-bound view
to waiting, and producer exit restores awaiting-producer semantics. Diagnostic
eviction loses no durable state or authority. Partial/checking/unavailable are
historical runtime diagnostics; no phase claims healthy or complete coverage.

The actual compiled Core starts and restarts against an isolated Store and
loopback HTTP endpoint. A genuine unprivileged Unix listener is refused by the
production root socket/peer check before any request frame. The API reports
`scheduled`, partial coverage and unavailable checks, with no observation result
created. No production unit credential/clock override is used. Recorded final
Core PIDs are 193720 and 193726; both were killed and waited by the owned test
guard and later observed absent. Earlier integration PIDs are retained too.
Successful reader fixtures use private unit credentials/clocks and establish
no successful installed root-positive Engine read.

The closed browser fixture uses exact built assets and synthetic route replies.
It exercises creation, cross-midnight UTC windows, revision-checked pause/resume,
permission enablement/reauthentication, all four runtime phases, stale/unavailable
write refusal and forbidden-row clearing. It records six intended POSTs, no
automatic mutation retry, no unexpected alert/notification reads and no page
errors. Desktop/mobile captures were visually inspected; the mobile table stays
inside its existing keyboard-focusable horizontal scroll region. This is
separate from the actual Core HTTP test and establishes no installed API/UI
journey with a successful protected reader.

## Reproduction and scope

`checks/` contains complete gzip-compressed guard records and logs, including
failed fixtures and earlier maps. `launch-and-local/` preserves launch/admission
identities, frontend build/asset hashes, visual review, final source comparison
and native process absence. `browser/browser-final/` contains final captures and
the served-byte hash record; `browser/browser-current/` retains the initial run.
`source.patch.gz` contains the qualified commit. `reproduction-sources.tar.gz`
contains changed source, design and qualification helpers/unchanged guard.
`SHA256SUMS` covers every other packet file. Reproduction assumes this repository
and its locked cached toolchain/dependencies; scratch stays on repository disk.

The normal owned Core producer is registered. A one-second serial interval
skips missed ticks; each pass is bounded to eight seconds plus one 100-ms
best-effort permit cleanup. In-app publication and existing retention progress
before root collection. Unsupported effects retain their blocked/skipped path.
The new optional runtime view and `scheduled` status change four generated
contract files. SQL schema 14, dependencies, package policies, service ceilings
and protected service activation are unchanged. Current concrete grants remain
required. Return rehearsal still needs policy before-images with the matching
pre-schema-14 database and package assets.

Remaining host/SMART/mount/protection sources, reviewed replacement/retirement,
safe history compaction and ordinary external channels remain open. Installed
root-positive collection/API/UI/restart, current packages/upgrades/return,
ARM64/Pi, resource/performance and real idle-write soak gates remain required.
A1/B1, wider A/B/P04 and P05/P06 cutover remain unfinished; the historical
inventory-performance failure is unchanged. No Pi, real Docker, package/service
activation, external recipient, frozen-app mutation or push ran.
