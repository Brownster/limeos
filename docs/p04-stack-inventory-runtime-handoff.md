# Registered stack inventory: local runtime handoff

Current acceptance, 5 November: the [installed Stack API/UI packet](rewrite-evidence/p04/2026-11-05-b2-installed-stack-ui/README.md)
passes the signed standard package with eight registrations, sixteen real
containers, eight running and thirty-two bind declarations. Empty/mixed/maximal
cardinality layouts, unsafe-source refusal/recovery, actual Core crash and reader
restart, and browser logout/relogin pass. The complete reader cgroup peaks at
6,250,496 bytes under unchanged limits. Desktop displayed states pass review;
narrow member-table readability and a clipped mobile heading remain open.
This is current AMD64 read-flow evidence, not complete B2/P04 or Pi qualification.
The sections below retain their original source and historical scope.

Source `e5e45caa31db2062ef118e28ce2ad419a944000c`, based on local main `3190286`.
The [runtime design](plans/2026-10-20-stack-inventory-runtime-design.md) now has
registered collection, transport, caching, authorization and a mounted Stacks
screen. Local qualification passes. Installed acceptance remains open; this
does not close B2, RW-042, P04 or the Pi cutover gates.

## Registered path

The dormant `limeos-storage-reader.service` collects one cohort every thirty
seconds when its independent `allow_stack_read` ceiling is enabled. A fixed
package-owned `limeos-stack-read-worker` uses the existing sealed image,
guardian, original four-second deadline, parent-death handling and verified
cleanup. It calls the genuine UID-0 catalog and fixed `/run/docker.sock` Compose
readers. Each source passes its original owner revalidation before projection.
Neither JSON nor a cached cohort reconstructs retained ownership or authority.

The worker and the existing storage inventory share one admission gate. The
supervisor retains that gate until cleanup completes, including after waiter
timeout. An uncertain cleanup permanently blocks subsequent expensive work,
including the older storage request, until service restart.

The standard profile selects `/etc/limeos/management-catalog.json`; shadow uses
`/etc/limeos-shadow/management-catalog.json`. Worker image selection accepts only
the corresponding fixed installed executor path. Existing protected catalog
rules still require 1–8 registrations, each with declared services. Missing or
invalid configuration stays unknown. An empty declared stack means a registered
stack without Engine members, rather than an inferred empty registry.

Fresh package policies enable the read ceiling, but package installation still
leaves this service dormant. Upgrades preserve existing operator ceilings; an
older policy without the new flag keeps stack collection disabled. Core uses its
existing configured storage-reader socket. No production/Pi service was started.

Core's background proxy reads the root cache every ten seconds. RPC and HTTP
GETs read caches only. Source failures retain separately marked stale facts;
original timestamps, identity and remaining monotonic lifetime survive cache
relays. Retries, reconnects and clock rollback cannot renew an old sample.
Nested wire decoding enforces fields, cardinalities, text and report bounds.

`GET /api/v1/deployment/stacks` resolves the current stored session and grants
before cache access. It requires `HealthRead` on `deployment:stacks`, each
registered stack ID, each project name using `stack:<name>`, and every container
ID represented in the cohort. A partial grant refuses the complete view.
Administrator role alone grants no access. Revocation rejects subsequent reads.

The native Stacks view uses generated contracts and shows services, replicas,
one-offs, stopped members, paths and source uncertainty. It remains reachable
when general observations fail. Principal, grant revision and session identity
key a new owner; layout cleanup disposes abandoned requests at browser commit.
The browser fixture caught and regressed an unbound timer receiver that Node's
timer implementation accepts. Source failures withhold counts; 403 clears facts.
Whole-stack effects and actual Compose source inspection remain pending.

## Resource and qualification boundary

All service limits remain 64 MiB, 256 descriptors and sixteen tasks. The narrower
model reserves 12 MiB manager RSS, 8 MiB guardian RSS, 20 MiB worker RSS, two
8 MiB image allowances and an 8 MiB margin. Worker admission reserves 67
descriptors within the existing 226 soft limit: sixteen baseline, eighteen
catalog, one Engine and thirty-two transient. Original owner ledgers remain in
force. Samples and component models do not establish whole-service acceptance.

[Evidence packet](rewrite-evidence/p04/2026-10-20-stack-inventory-runtime/README.md):

| Local check | Result |
| --- | --- |
| Focused native Rust | 1,012 passed: domain 547, container 349, protocol 6, observations 39, API 19, executor 48, Core stack/HTTP 4 |
| Native Core/API | Actual HTTP route reads the shared cache and rejects a revoked session; partial resource grants refuse the complete view |
| Frontend | 222 passed; strict TypeScript and production build passed |
| Mounted browser | Six checks passed against closed API fixtures; screenshot inspected |
| Reference suite | 61 passed |
| Supplemental B harness | 68 passed; overlaps root API tests and stays separate |
| Static checks | Strict all-target workspace Clippy, real contract drift, Rust formatting, changed Python Ruff, Bash syntax and repository hygiene passed |

These are focused local checks, not a new full-workspace test total or native
ARM64 execution. The browser uses fixture responses, not installed worker data.
The initial memory stop, missing dependency, browser failures and lint failures
remain archived. Work, builds, caches and ordinary scratch stayed on repository
disk under the three-GiB sampled host-memory stop guard.

## Installed candidate follow-up

The [21 October installed packet](rewrite-evidence/p04/2026-10-21-stack-installed-read/README.md)
records fresh Debian binaries and signed standard/shadow packages from clean
main `54022bb`. Signed standard installation passes, but genuine collection
refuses: the catalog gets `EACCES` opening PID 1's namespace and the Engine gets
`ENOSYS` opening its fixed socket with `openat2`. The sealed worker starts and
exits normally. The subsequent diagnostic below identifies the filter conflict.
Service limits and source identity checks are unchanged.

The four terminal VM attempts retain the initial host-memory stop, an untraced
collection failure and two diagnostic traces. They provide no successful
installed collection or full resource qualification. The owned build container
and VMs were removed; the Pi remains untouched.

Source `7f35dd5` subsequently corrects the catalog's host-view proof. It retains
the actual current NSFS descriptor and compares both bounded genuine proc mount
tables with the held root descriptor's mount ID and device. Different mount roots,
cloned namespaces, malformed/ambiguous data and resource bounds refuse. It adds
no ptrace capability or serialized authority. The [new packet](rewrite-evidence/p04/2026-10-21-stack-host-view/README.md)
records 356 container unit tests, seventeen compile-fail doctests, strict Clippy,
fresh Debian packages and six passing installed catalog checks. Writable-catalog
and private-namespace refusal, foreign UID rejection, no-renewal reads and reader/
Core restart recovery pass under the unchanged service profile.

The diagnostic confirms `RestrictSUIDSGID` returns `ENOSYS` for `openat2`; an explicit
syscall allow alone does not override it. Source `5ef8897` now contains the scoped
reader profile exception and a namespace-root rendering correction. Empty
capabilities, `NoNewPrivileges`, closed read-only dispatch and all resource limits
remain unchanged. The SUID/SGID file-bit restriction itself is removed; the
[profile decision](plans/2026-10-21-stack-engine-profile-design.md) records that
distinction.

The [installed Engine packet](rewrite-evidence/p04/2026-10-21-stack-engine-profile/README.md)
binds a fresh Debian build and `.4` packages to 357 container unit tests, seventeen
compile-fail doctests, 65 reference tests, strict Clippy and nine passing installed
checks. Genuine empty and running/stopped/one-off layouts, independent source
refusal/recovery, no-renewal reads and Core/reader restart recovery pass. Actual
image hashes identify stable manager/guardian/worker capability samples. The
observed cgroup memory peak is 5,996,544 bytes for these layouts; full resource
and B2 acceptance remain open. Earlier failures and the host-memory stop remain
preserved alongside the passing result.

## Next acceptance work

The 5 November packet covers the current installed cardinality/API/UI flow and
whole-service cgroup observations. Correct narrow member-table readability and
recheck its capture. Extend resource qualification to worst-byte inputs and
failure paths within unchanged limits. Exercise
worker/guardian/manager death, cancellation, deadline, late output and cleanup
barriers through the registered path. Add installed principal/grant replacement
and interrupted browser ownership scenarios. Run native ARM64 qualification on
that candidate; the old `477b81f` CI result does not qualify this source.

The Engine's unsupported PID-1/socket-activation case remains explicit. Real
Compose filename/YAML/environment/include diagnostics and file/review binding
still need protected collection. Whole-stack lifecycle, shared claims/approval,
verified shutdown and recovery remain subsequent B2/B3 work. B1's genuine
container-only observation producer and in-app notification flow now pass
[installed timer/crash evidence](rewrite-evidence/p04/2026-11-05-b1-status-panels/README.md);
other sources/channels and recurring effects remain open. A1's
[userspace-task revalidation failure](p04-a1-namespace-budget-handoff.md) remains
open. A source audit also found the absolute-root assumption in the bounded
storage mount parser; that separate Stream A path needs its own regression and
consumer review. P05's minimal assistant and P06's migration/return rehearsal remain required
before replacing the frozen app.
