# HOST-001 prerequisite assessment and repair proposals

Engineer B, 15 October 2026. Independent leaf based on main `b37c718`.

## Scope

Provide one bounded, pure Rust rule for the frozen application's memory-cgroup
and explicit journal-cap prerequisites. The host executor owns the leaf under
`bins/executor/src/updates/`. Installer, preflight and authenticated repair paths
must eventually call the same registered rule. This slice performs no reads,
writes, daemon commands or reboot. HOST-001 remains open until central
registration and disposable-host interruption/verification qualification.

The frozen reference is pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`:
`host_prerequisites.py`, `pihealth_helper.py`, `setup.sh` and
`scripts/onboarding-preflight.sh`. Preserve the two boot flags, preferred firmware
boot path, fallback boot path and fixed 200M managed journal template. Correct
the conflicting empty-value, any-match and non-atomic installer behavior.

## Design choice

A pure executor leaf with borrowed, bounded snapshots permits useful focused
regressions before shared dispatch exists. Implementing an independent writer
would duplicate the integrator's authority, locking and recovery work. Editing
maintainer scripts now would leave another divergent rule. The chosen leaf
instead returns fixed-target proposals plus complete snapshot fingerprints for
freshness checks; these are comparison values, never authority or effect receipts.

Memory availability comes from root cgroup-v2 controllers. Missing, unreadable
or empty controller facts remain unknown. Availability satisfies this narrow
prerequisite; it does not prove container delegation/accounting health. When
memory is missing, prefer `/boot/firmware/cmdline.txt`; fall back only when that
file is confirmed absent. An unreadable preferred file blocks fallback. Append
only missing fixed flags while preserving every existing byte and line ending.
Reject multiline/non-ASCII/oversized boot input, conflicting or repeated relevant
parameters, and quoted syntax. Already-present flags with missing runtime memory
mean reboot/runtime verification is pending, with no repeated file proposal.

Journal assessment consumes a complete selected source set: main file followed
by unique, lexicographically sorted drop-in basenames. The trusted collector must
resolve directory precedence and `/dev/null` masks before constructing this set.
Missing files differ from failed reads; partial collection blocks assessment.
Parse `[Journal]` and the exact `SystemMaxUse` key, honoring the last assignment.
An empty final assignment means no explicit cap. Respect supported positive
operator limits. Unsupported relevant syntax, zero or malformed values are
unknown and receive no automatic repair. This conservative parser accepts
positive decimal integers with optional K/M/G/T/P/E binary suffix, bounded to u64.
It refuses continuations and malformed section/assignment syntax rather than
guessing how systemd interpreted them. Journald's built-in defaults are distinct
from this product requirement for an explicit cap.

Only propose creation of `/etc/systemd/journald.conf.d/limeos.conf`. Never replace
foreign content or a mask with that basename. Simulate the fixed template in
filename order and refuse a repair overridden by a later reset. Exact existing
managed content is idempotent. A file proposal requires reload and independent
verification; it does not establish activation or reclaimed disk space.

Bound input to 64 drop-ins, 64 KiB per journal source, 256 KiB total journal bytes,
4 KiB controller bytes and 4 KiB boot bytes including proposed output. Reject
oversized, malformed or incomplete snapshots with typed reasons. Proposals expose
no arbitrary paths or command text and redact byte payloads from Debug output.
Revalidation compares the whole relevant source snapshot, including masks,
origins, absence and the runtime controller observation.

## Integration and validation

The integrator registers the module and fixed typed operations, obtains trusted
complete read facts, checks current grants, acquires common claims/locks, recollects
facts, revalidates proposals and uses the shared before-image/atomic replacement/
directory-fsync/recovery path. Preserve boot metadata, refuse symlink/replacement
races and verify old-or-new content after interruption. Record daemon/reboot
verification separately from a successful file write. Package scripts call this
registered rule; they do not reproduce parsing or edits in shell.

Focused fixtures cover frozen Pi layouts, partial flags, byte preservation,
idempotence, cgroup unknowns, conflicting flags, journal empty reset, last-wins,
comments/sections/masks, later overrides, incomplete reads, bounds and stale
snapshots. An isolated harness compiles the exact leaf without changing shared
Cargo manifests or exports. Broad package/ARM64/Pi/installed API/UI qualification
belongs to the existing test owner after central integration.

Sources: [systemd 252 journald configuration and precedence](https://www.freedesktop.org/software/systemd/man/252/journald.conf.html),
[kernel cgroup-v2 controller availability](https://www.kernel.org/doc/html/v6.1/admin-guide/cgroup-v2.html),
[HOST-001 register](2026-10-04-rust-rewrite-defect-register.md),
[B ownership](2026-10-14-engineer-p04-deployment-scheduling-runtime.md).
