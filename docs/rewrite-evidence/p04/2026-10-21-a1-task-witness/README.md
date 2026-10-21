# Exact A1 task witness, 21 October 2026

Product source `c6932b291d2583114b3d4b122e75d94b4db9be93` adds bounded,
failure-only journal diagnostics to the existing exact host-consumer task
comparison. Design: `8257007`. There is no admission, expiry, namespace, resource,
locking or poisoned-owner exception. The original conflict remains fatal.

## Verified result

The current installed reproduction fails in `pre_effect_api_refusals`: the first
genuine plan returns HTTP 409, expected 201. The actual compared vectors report
two removed tasks and no additions. Both sampled removals have full numeric
identity matches in the independent before-RPC witness:

| PID/TID | Start ticks | Namespace device/inode | Before-witness cgroup |
| --- | --- | --- | --- |
| 7137 / 7137 | 21331 | 4 / 4026532297 | `/system.slice/systemd-udevd.service/udev` |
| 7138 / 7138 | 21331 | 4 / 4026532297 | `/system.slice/systemd-udevd.service/udev` |

Both independent snapshots completed without observed read/membership errors;
neither identity remains afterward. `vm-4-analysis.json` joins those exact
fields without inferring them from task counts. This identifies the new
reproduction's disappearing udev workers; the older run's identities remain
unproven. This run has no approved mount result or effect retry.

## Checks and attempts

| Check | Result |
| --- | --- |
| Host-consumer unit module | 12 passed, including three new difference/bound cases; 216 filtered, not a full storage run |
| Storage all-target Clippy | Passed, `-D warnings` |
| Rust formatting | Passed |
| Offline Debian build | All five binaries from the frozen current source; source hashes unchanged; no OOM/guard stop; owned container removed |
| Package assembly/signing | Standard/shadow packages built, immutable handoff hashes matched, new two-day disposable key; assembly alone is not installed acceptance |
| VM 1, 512 MiB | Failed at first planning inventory HTTP 503, before A1 |
| VM 2, 512 MiB | Same early failure; later CLI and authenticated reader RPC succeeded and unit remained active |
| VM 3, 512 MiB | Host memory guard stopped it before the readiness diagnostic |
| VM 4, 384 MiB | Read-only readiness probe succeeded in 0.120 s; preceding planning/claims/target recovery checks passed; exact A1 task conflict above |
| VM 5, 384 MiB | Host guard stopped it before testing the private udev-quiescence setup correction |

VM 4's first diagnostic RPC was already successful. The exact cause of the
earlier HTTP 503s is not captured; startup timing/shared-gate contention is an
inference from code and later observations. No unavailable result is counted as
successful inventory. Readiness probing is outside planning/effect attempts.
The original planning, approval, cancellation, effect, replay and recovery
assertions are unchanged. Prior-version upgrade qualification was explicitly
disabled in these diagnostic wrappers. Standard is installed; shadow payload
and removal checks retain their narrower preceding-suite scope.

All runners keep the 3-GiB sampled host stop floor. The smaller runner differs
from the tracked runner only in non-build guest RAM; service ceilings and test
assertions remain unchanged. VM 4's minimum available host memory was
3,305,410,560 bytes. VM 3/5 stops remain failures, not acceptance. No new whole
worker, worst-case layout, ARM64, performance, soak or production Pi claim follows.
Builds, caches, logs and VM scratch stay on repository disk. No push occurred.

## Reproduction and next action

`fixture-objects/` retains the common guest modules. Each `guest*-inputs.json`
binds its exact wrapper under `wrappers/` and common input hashes. VM 4 uses
`guest-3`; VM 5 uses `guest-4`. The latter adds `udevadm settle` and a bounded
wait for the genuine daemon's workers to exit naturally before the first RPC,
with no service stop, task filter or operation retry. Its execution is still
unqualified because VM 5 stopped earlier. Promote and test these fixture
preconditions before claiming an actual approved mount result.

Build and packaging scripts, image identities, source verification records,
binary/package hashes, public fixture key and signed repository metadata are
retained. Private keys, binaries and Debian disk images are excluded; their
local artifact paths and hashes remain in the records. Failed/interrupted
stdout and available guest diagnostics are preserved separately.

The remaining A1 effect/failure/restart matrix, A/B parity, P05 and P06 cutover
requirements remain open. No production host or frozen application was touched.
`SHA256SUMS` covers every other packet file.
