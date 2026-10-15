# HOST-001 host prerequisite planner handoff

Engineer B, 15 October 2026. Branch `engineer/p04-host-prerequisites`, based on
main `b37c718`. Planner source: `92417d02735118f74e26a588625fb471bd73782f`.
This branch is independent of B1's automation branch. Neither B1 nor HOST-001 is
complete: shared Store/runtime work for B1 and registered collection/effects for
this leaf remain with the integrator.

## Delivered interface

`bins/executor/src/updates/mod.rs` exports the leaf `prerequisites`. The shared
executor root does not declare it yet. Existing executor dependencies already
include sha2; no shared manifests, lockfile, dispatch or package scripts changed.

- `assess_memory(MemoryFacts)` checks the root cgroup-v2 controller observation.
  Available memory satisfies only controller availability. Empty, absent or
  unreadable observations remain unknown. A positively identified ARM64 Pi may
  receive a boot proposal; other/unknown platforms are blocked. Preferred firmware
  file absence permits fallback; failed reads do not. Preserve every original
  byte while appending only missing frozen flags. Reject conflicting/repeated
  flags, hyphen aliases, quoted/multiline input and the init `--` separator.
- `assess_journal(JournalFacts)` checks explicit configured SystemMaxUse, not
  daemon activation, actual usage or systemd's default limits. Empty final values
  reset the explicit cap. Exact key, section and last-assignment rules replace
  the frozen any-match checks. Respect supported positive operator caps. Never
  overwrite foreign managed destinations/masks or propose a lower-priority file
  defeated by a later reset. Zero, decimal/quoted/unsupported values, malformed
  syntax, bare carriage-return separators and continuations remain unknown,
  preserving operator files. CRLF remains supported.
- `Assessment` distinguishes satisfied, unknown and missing. A missing repair
  is proposed, blocked with a reason, or pending reboot/runtime investigation.
  `Proposal` has private fields, a closed target enum, before-image digest/absence,
  bounded replacement bytes and required post-write verification. It cannot be
  constructed/deserialized from API text. Debug output redacts file bytes.
- `matches_memory` / `matches_journal` compare whole relevant snapshot fingerprints,
  including runtime/platform, all source bytes, absence, drop-in origin and masks.
  These compare supplied facts; they authenticate no reads or callers and are
  not durable recovery records. Borrowed facts do not retain protected read owners.

Inputs: 4 KiB each controller/boot file; output boot file at most 2047 bytes,
conservatively reserving ARM64's terminating NUL; 64 drop-ins, 64 KiB per journal
source and 256 KiB total. Root/executor time, memory and reply limits remain
separate central budgets. Repairs select only the two fixed boot paths or the
fixed `/etc/systemd/journald.conf.d/limeos.conf` 200M template. No caller-selected
shell, path, flags, cap or file text is an operation parameter.

## Required central registration

1. Declare the leaf in the executor root and define closed detection/repair
   operations in the common protocol. The trusted collector supplies root cgroup
   facts, positive supported hardware/architecture/boot-layout identity and stable
   boot-file facts. Detect systemd/default-journal support before journal repair.
   Refuse unsupported bootloader/kernel limits even when the leaf size bound fits.
2. Supply a complete effective journal source set. Select the main configuration
   according to supported systemd-version search rules. Select `.conf` drop-ins
   across vendor, local vendor, runtime and administrator directories; resolve
   same-basename precedence and `/dev/null` masks, then sort unique basenames.
   Include every selected source. Missing directories/files require trustworthy
   absence; enumeration/read errors yield incomplete/unavailable. The leaf does
   not perform resolution and `complete: true` is not proof of trusted collection.
3. Preserve protected read owners, source/directory identity and metadata checks.
   Authenticate current grants; use shared claims/locks, durable jobs, reviews and
   executable authority. Recollect all facts under those claims and revalidate.
   Do not let package scripts, API callers or serialized fingerprints supply
   trusted facts or authority. No parallel registry/lock/SQL scheme is proposed.
4. Apply through shared before-image/recovery and atomic replacement facilities.
   Preserve boot ownership/mode and relevant metadata, validate supported link/
   filesystem behavior, write/flush/fsync a same-directory temporary file, replace
   atomically and fsync the parent as appropriate. Protect creation/absence races
   and directory changes. Central recovery owns interruption/uncertain effects;
   file hashes alone do not capture protected file identity or metadata.
5. Verify persisted bytes independently. A boot write is pending until reboot and
   root memory-controller verification. A journal write is pending until supported
   reload and effective configuration verification. The leaf executes no restart,
   reboot or vacuum; a future fixed vacuum action needs separate bounded outcome
   reporting. Failed activation/reclaim cannot become success from file existence.
6. Make installer/preflight/repair call this registered rule. Remove duplicate
   shell/Python rules only in the centrally owned package follow-up. Register
   authenticated API/UI status and explicit job outcomes through shared routes.

## Focused evidence and remaining acceptance

[Local packet](rewrite-evidence/p04/2026-10-15-host-prerequisites/README.md): 13
Rust tests, including 17 memory and 25 journal fixture scenarios, strict Clippy,
Rust formatting and Python harness lint/format pass on x86_64 Rust 1.88. The
isolated harness includes exact source files and retains its own Cargo.lock;
this is not a whole-workspace/package/hardware qualification result.

| Acceptance | Supplied scenario | Still required |
| --- | --- | --- |
| Empty cap / last assignment | Fixture main and drop-in reset/positive ordering | Installed canonical collection and rule entry point |
| Safe Pi repair | Firmware/fallback, byte preservation, partial flags, repeat planning, conflicts, aliases, separator and output bound | Supported real boot layout, fixed repair, metadata and reboot verification |
| Preserve operator state | Positive caps, foreign managed file/mask, unsupported syntax, later override | Directory/link race refusal and installed UI outcomes |
| Unknown stays unknown | Partial/unreadable collection, invalid source set and byte bounds | Protected read-budget/error propagation |
| Freshness | Changed full bytes, platform, controllers, masks, origins and absence refuse reuse | Current authority, lock/claim and real stale-fact effect refusal |
| Atomicity/recovery | No effect implementation or atomicity claim | Crash cuts before/after write, fsync and replace leave old or new bytes; reconciliation and restart/lost-response outcomes |

Qualification owner should run disposable-host/package interruption cases after
integration, including startup/preflight/repair agreement on the same observations,
failed journal reload/vacuum, already-present flags with missing runtime memory,
and unsupported hosts. Current production Pis and the frozen Python source remain
untouched. See the [design](plans/2026-10-15-host-prerequisites-design.md) for
primary documentation and conservative parsing limits.
