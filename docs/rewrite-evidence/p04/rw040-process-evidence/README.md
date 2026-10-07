# RW-040 process evidence: local handoff

Implemented on `engineer/container-process-evidence` in
`/tmp/limeos-container-process-evidence`, starting exactly from
`d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4`. Production implementation, Rust tests
and raw fixture identity are all
`ad5ff6e2ae75c98dc261c951b084a14f59c3b5e1`. Later commits contain local evidence
helpers and documentation only. The final response and the separate
`/tmp/limeos-container-process-evidence-handoff.json` identify the final branch
tip without embedding a commit's own hash into its contents. Main publication,
merged exact-source CI and installed qualification belong to the integrator.
The feature branch is committed locally for review.

| Commit | Purpose |
|---|---|
| `0a7e9a7c7b320385b9d93402ea53b1362147d444` | Enable only the existing pinned rustix event feature |
| `99faf8da1c3f61805defc3528211e155506eb112` | Export only the process module |
| `ad5ff6e2ae75c98dc261c951b084a14f59c3b5e1` | Retained kernel observations and regression fixtures |
| `76bac90579cd15ccce52da4c3371adeb9f4f2524` | Local command recorder and hash verifier |
| `334bf629a581136a3d9d69b3ce1e03a20330e39b` | Interface and fixture-scope documentation |
| Final evidence commit | This handoff, raw attempts and hash/resource inventory |

Changes stay within the five assigned ownership paths and the two explicit
shared-file exceptions. `Cargo.lock`, package versions, workspace dependencies,
domain/IPC/API/CLI, service/capability policy, Docker acquisition, storage source
and filesystem binders, archive code, packaging and historical proof remain
outside this slice. See [the interface](../../../p04-container-process-evidence.md)
for the API, failure semantics and integration obligations.

| Frozen-source check | Result and raw output |
|---|---|
| `cargo fmt --all -- --check` | [Pass](logs/fmt-final.log) |
| Strict workspace/all-target clippy, locked/offline | [Pass](logs/clippy-final.log) |
| Workspace tests, locked/offline | [320 unit/integration + 2 compile-fail tests pass](logs/workspace-tests-final.log); 29 new process unit tests |
| `cargo deny --locked --offline check` | [Pass](logs/deny-cached-corrected.log); three existing duplicate warnings: base64, getrandom, syn |
| `cargo audit --no-fetch --no-yanked --db /tmp/limeos-advisory-db` | [Pass](logs/audit-cached.log), cached scope; live yanked-package checks excluded |
| `python3 scripts/check_repository.py` | [Pass](logs/repository-final.log) |
| `python3 scripts/check_contracts.py` | [Pass](logs/contracts-final.log); generated contracts match |
| `git diff --check` against assigned base | [Pass](logs/diff-final.log) |
| Isolated maximum-row allocation test | [Pass](logs/resource-frozen-source.log) |
| Actual 64-child FD retention and Drop | [Pass](logs/real-fds-frozen-source.log) |

Each listed check has adjacent JSON metadata binding its command, working
directory, elapsed time, exit code, raw output hash and frozen source/fixture
hashes. `qualification.json` adds compiled test artifact hashes, environment,
package counts and command timing totals. `SHA256SUMS` covers the handoff files,
interface, fixtures and frozen inputs; it excludes itself. Run:

The directory's `.gitattributes` exempts only `logs/*.log` from whitespace
diagnostics so original tool padding/newlines are preserved. Source and
documentation retain normal whitespace checks. The pre-exemption failure is
retained in `logs/diff-raw-output-failed.log`.
The full handoff check also [passes](logs/diff-handoff-final.log); its metadata
identifies the documentation parent and the same frozen source hashes.

```text
uv run --offline --no-project docs/rewrite-evidence/p04/rw040-process-evidence/verify.py
sha256sum --check docs/rewrite-evidence/p04/rw040-process-evidence/SHA256SUMS
```

The lockfile has 227 packages: 211 with an external `source`, 16 local. Both
cached advisory databases are revision
`ef6173cbc5c50ec8166f9a5b28f07834144373ee`, committed
2026-10-03T10:14:03+02:00; audit loaded 1,290 advisories. This proves the cached
scope, not the latest online advisory or registry state. No package/version was
introduced; only rustix 1.1.5's existing event feature was enabled. Local
qualification used x86-64 Linux 7.0.14-101.fc43 and Rust 1.88.0. No frontend,
native/ARM harness, installed package or Pi workload was rerun for this library.

Real kernel proof covers same-UID owned child pidfds, retained proc/namespace/
root descriptors, start and numeric credential observations, complete mount
tables, exit after collection, exit between pidfd/proc opens, dead held proc
reads, actual descriptor release, ordinary-file substitution refusal and
detached private directory refusal. A separate private adapter substitutes host
authentication because UID 1000 cannot open PID 1's namespace on this host; all
child kernel operations delegate to the production Linux implementation. This
does not qualify the UID-0 public success path or cross-UID service confinement.
The public unprivileged API refusal is tested. Only owned children and private
scratch directories are mutated.

Private injected proof covers complete no-mount membership, stopped exclusion,
full snapshot/digest changes, invalid PIDs/counts, PID replacement, unchanged
PID/start with UID/GID/root/namespace changes, identical-looking tables in a
different namespace, raw parent/propagation/options/source-only changes,
access/primitive/descriptor failures, future/expired declarations, read-time age
crossing, wall rollback, monotonic expiry, repeated revalidation, cooperative
timeout, bounded records/tables/rows/serialization and report amplification.
Namespace switching and PID reuse were injected, not performed on this host.
Serialization and external construction compile-fail tests reject report
deserialization and owner forging. Observations still require authenticated
Engine before/after acquisition and physical composition.

At the admitted 64-process/4,096-row-per-table maximum, the fixture has 262,144
rows and 8,059,904 raw table bytes per pass. Retained row vector capacities use
27,262,976 bytes and string capacities 5,242,880 bytes: 32,505,856 bytes (31 MiB)
of counted row heap payload. The report is 38,163 bytes. The isolated frozen
test's `VmHWM` is 67,356 KiB (65.78 MiB), including the Rust test harness, private
fixture copies and allocator overhead; this is process peak RSS, not a complete
allocator trace or installed executor PSS. Its private clock is frozen to test
allocation admission, so this is not a two-second production performance claim.

The private descriptor model retains 261 and peaks at 264 handles; production
record reads add one temporary descriptor. A separately isolated real 64-child
run measured baseline 4, 261 additional retained and peak 265 additional, then
returned to baseline after Drop. The FD directory sampler opens its own temporary
descriptor in both baseline and samples. Production retains five host and four
per process; fresh proc/namespace/root plus one record FD explain the peak.

The bounds are finite but maximum row retention is substantial. A concrete
proposal for the integrator is a worker-level aggregate ceiling of 32,768 parsed
rows, reducing row-vector payload from 26 MiB to 3.25 MiB. That tighter admission
rule is **not implemented here** and needs compatibility qualification against
real supported layouts before adoption. Raw byte/string ceilings and an
independent worker deadline/single-flight limit still matter. These library
measurements do not sign off P00 inventory footprint or latency.

Retained failures are deliberate review evidence. Pre-freeze attempts are not
qualified as the final source. [The initial 23/24 result](logs/targeted-initial-failed.log)
was transferred from the tool stream and has no separately measured timing or
source hash. Later attempts have raw outputs plus command/time metadata:

- [Host diagnosis](logs/real-host-diagnostic.log) isolated denied PID-1 namespace
  access; production retained the refusal and local tests explicitly substituted
  host authentication only.
- [Btrfs diagnosis](logs/real-root-device-diagnostic.log) recorded root device
  `0:39` versus mount-row `0:35` with matching mount ID 43.
  [The failing fixture](logs/btrfs-regression-before-fix.log) precedes removing
  the invalid device-number equality requirement.
- [Volatile proc link counts](logs/host-volatile-link-diagnostic.log) changed
  from 489 to 493 while owned children started. Identity now compares stable
  device/inode/mount/mode, with nonzero links checked live. A real scratch
  directory regression confirms link-count changes and detached refusal.
- [The metadata attempt](logs/targeted-stable-identities.log) exposed a test's
  incorrect assumption that maximum numeric fields alone overflow 48 KiB.
  Tests now preserve those fields and separately reject amplified serialization.
- [Clippy](logs/clippy-initial.log) rejected a manual range expression;
  [the deny invocation](logs/deny-cached.log) rejected an unsupported flag.
  Both corrected checks pass with raw outputs retained.
- [Artifact capture](logs/artifact-capture-failed.log) rejected an over-escaped
  path-extraction expression. Corrected capture binds 19 compiled test artifacts;
  this packaging attempt has no independently recorded duration.

The assignment's provisional 24 human engineering hours and 36-hour scope review
remain estimates. Agent wall time began 2026-10-07T19:33:33Z; the measurement
checkpoint is in `qualification.json` and final packaging time in the separate
handoff JSON. Measured human engineering time was not supplied. Recorded command
durations are accounted separately and overlap: the workspace check took
77.566124 seconds, including a 67-second build, and contracts took 51.912641
seconds including build-lock waiting. Pure infrastructure/approval wait time was
not separately instrumented; no user approval wait was recorded. Do not treat
agent elapsed time or overlapping check totals as human engineering hours. P04's
320-hour estimate and separate 480-hour cutover-scope review are unchanged.

No gate closes. Installed cross-UID/nondumpable and namespace-switch cases,
Engine ownership/reacquisition, destination/UUID/backing composition,
pool/protection/share propagation, approvals, complete claims/root ceilings,
bounded runtime admission, effect-time checks, mount/fstab/unmount execution and
runtime-loss shutdown remain integration work. DSK-001, MNT-001/MNT-002, RT-001
and applicable P04 rows remain open. The 61.401 ms/20 ms inventory failure is
unchanged. No Docker, root guest, setns/unshare, mount, block-device, production
write, SSH, Pi or frozen Python operation ran.
