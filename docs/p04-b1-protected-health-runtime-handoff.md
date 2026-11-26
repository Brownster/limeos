# Protected container health through Core and the Store

Source `1219844` connects the
[retained genuine Engine reader](p04-b1-container-health-evidence-handoff.md)
to the [transition-only Store seam](p04-b1-transition-polling-handoff.md).
Source qualification and exact commits are recorded in the
[local evidence packet](rewrite-evidence/p04/2026-10-31-b1-protected-health-runtime/README.md).
The [design](plans/2026-10-31-b1-protected-health-runtime-design.md) separates
this internal partial pass from normal scheduling and installed acceptance.

The root storage RPC accepts a version and random 64-hex nonce under the new
`allow_container_health_read` ceiling. Kernel Core UID authentication precedes
frame parsing. The flag defaults false for existing policies; it can coexist
with stack/storage reads and cannot coexist with advisory host/container/log
or effect authority. The request selects no socket, path, namespace, credential,
clock, command, resource filter or numeric profile.

The fixed installed/shadow `limeos-stack-read-worker` image supplies sealed
health guardian/worker roles. The roles reject mixed stack/configuration
selection and verify the current image digest/seals. Collection uses the real
root-only Engine reader, its complete double inspection and fresh revalidation.
The existing independent supervisor retains the shared read gate and image
until confirmed descendant cleanup; unconfirmed cleanup still poisons admission.
Existing manager/guardian/worker limits, service ceilings and Engine budgets
remain unchanged. The nonce-bound plain report caps 64 sorted full-ID members
(the product's Engine bound; sixteen until 26 November 2026) at 56 KiB, under
the 64-KiB executor frame, plus a `truncated` count of containers past the
bound, and contains only sanitized optional container signals. A truncated
report is never complete, and core never resolves, retires or silences
anything about a container it left out. A refused read carries a closed
reason (`HealthReadFailure`), which the reader logs when it changes and
`limeosctl reader-status --read container-health` prints.

Core authenticates a retained root-owned socket, unchanged descriptor/name
identity and kernel root peer before sending the frame. It accepts only the
expected ready, versioned, error-free health payload and matching nonce/time
interval. A private nonserialized owner carries the original monotonic deadline
of at most four seconds, starting before the RPC. The genuine Engine's
five-second lease starts later. Copying or receiving a report therefore cannot
renew its lifetime. Synchronous kernel calls are not a hard latency guarantee.

The internal bridge obtains current grants for every actual full-ID member and
`host:system`, then passes the report through the sole database worker. Store
completion takes the minimum of the original poll deadline and read deadline,
rechecks it after the SQL write lock and before commit, and rechecks source wall
age inside the transaction. Private nonce/active-lease matching still uses the
original admission deadline. Stable polls retain the existing zero-write path;
material transitions commit their existing occurrence, result, audit and outbox
atomically. Lost or late replies do not imply that no commit occurred.

Container-only health always produces a globally incomplete sample. Missing
member grants refuse the entire sample. Existing container silences or retained
alerts must bind current full IDs; legacy names, removed IDs and replacement or
retirement without review refuse. This slice introduces no silent rebinding,
fake recovery, periodic scheduler, external notification transport or browser
route. The next runner must preflight due/current authority, recover old queued
work, coordinate fair passes and supply fresh host/SMART/mount/protection facts.

The workspace run also exposed a source-reader defect on this Btrfs disk.
Storage fix `245d53d` authenticates the current bounded proc mount table before
accepting Btrfs per-subvolume inode devices, corroborates a mismatched final
descriptor with the kernel Btrfs filesystem type and preserves both device
numbers. Physical dependency and destination joins retain their exact matching;
physical Btrfs binding remains unavailable. The directory regression now accepts
any live positive link count and still requires zero-link detachment refusal.
The [separate design](plans/2026-10-31-source-btrfs-device-design.md) records this
boundary and the native owned probe. No mount or filesystem operation was needed.

Local tests exercise owned Unix listeners with private unit credentials/clocks,
actual Store queue/SQL-lock expiry, whole-sample authorization, partial incident
deduplication and retirement refusal. An opt-in owned observer invokes the actual
sealed production worker roles and confirms a refused collection with cleanup
under nonroot credentials. Only that observer lowers its own soft descriptor
limit to the existing guardian requirement; the parent runner and production limits stay
unchanged. A refusal alone does not identify its precise collection stage;
these tests establish no successful installed UID-0 Engine read.

The source-bound packet records 1,701 workspace runtime passes, 53 compile-fail
doctests, 242 frontend tests and checked build, thirteen package/profile tests,
strict all-target Clippy, actual generated-contract drift and explicit ShellCheck.
Seven final guards share 730 committed input hashes. All sixteen attempts,
including the Btrfs and worker fixture failures and missing/skipped linter checks,
remain distinct. Scratch stays on repository disk; one Cargo job/test thread
and the unchanged sampled 3-GiB floor remain in force. Original owned processes
are absent after qualification. No installed footprint or soak is inferred.

New package-created storage policies include the flag, while existing files are
preserved. The storage-reader unit remains dormant. Older binaries reject the
new policy field, so a qualified return path must preserve and restore policy
before-images alongside the pre-schema-14 database and matching package assets.
No package install, policy mutation or service activation occurred locally.

Fresh installed API/UI/restart, complete health and ordinary notification parity,
safe retention/compaction, packages/upgrades/return paths, resource/performance
and real 24-hour idle-write evidence remain open. A1 reliability, wider A/B/P04,
P05's minimal assistant and P06's actual-state migration/return rehearsal remain
required for cutover. Historical inventory-performance failures are unchanged.
Holly's Pi and the frozen app were untouched; nothing was pushed.

Source `2505191` now moves Store admission ahead of the protected read and adds
original queued-job recovery under the same fresh-report deadline. The
[admission/recovery handoff](p04-b1-container-admission-recovery-handoff.md)
records current scope binding, zero-read refusals, cancellation and restart
acceptance. Fair supervision and timer activation remain pending.
