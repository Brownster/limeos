# RW-041 Samba planning handoff

Branch `engineer/samba-planning`, disk-backed worktree
`/home/marc/Documents/github/lime-os/.worktrees/samba-planning`.
Base `1401c255d4a05b8621c2dcd497d6f7f0cbe91d56`; qualified code
`79c29053ab8cc3306045078f96d7c8d8c82d2f85`; design
`8ec68a8` precedes implementation. Subsequent documentation/evidence commits do
not change this runtime identity. Main source, the backup recovery worktree and
the frozen ARM64 qualification source remain untouched. This branch is not pushed.

The slice ports the frozen Samba saved configuration into checked Rust section
previews and declared device/pool dependencies. The [feature mapping and public
interfaces](../../../p04-samba-planning.md) describe the supported subset and
remaining parity. The [design](../../../plans/2026-10-12-samba-planning-design.md)
records ownership and limits; the [case matrix](case-matrix.md) identifies all
22 new regression cases.

The pure domain module accepts closed configuration, validates the existing
storage contract and pool plan, preserves numeric ownership and literal paths,
and returns full device requirements with direct/pool routes. Disabled/read-only
shares and disabled pools remain dependencies; branches retain meaningful order.
Unknown paths remain explicitly unmapped. More than 512 expanded requirements
refuses the whole preview. The bounded byte importer reports frozen defaults
and returns no partial configuration for unknown, malformed or unsupported input.

Local Rust 1.88.0/x86-64 acceptance passes **466 cases**: 460 unit/integration
cases and six compile-fail doctests. This is the 444-case main baseline plus
22 Samba unit cases, independent of the unmerged before-image, protected
SnapRAID-source and configured pool/protection dependency branches. Workspace
tests excluding executor-storage pass 382 cases (378 unit/integration plus four
doctests); executor-storage separately passes 84 (82 plus two). Strict workspace
Clippy/all targets, fmt, generated-contract drift and repository checks pass.
The [check record](checks.json) binds commands, environments, exits and logs.
The retained [capture script](run-local-checks.py) documents scratch isolation
and the sampled size calculation; its paths describe this local worktree/cache.

Worktree, build target, caches, logs and general scratch stay on disk below the
repository. Existing executor-storage tests alone use small owned, auto-cleaned
tmpfs fixtures. Their stat/mount-table and directory-link assumptions were
already found incompatible with workstation Btrfs in the prior slice. No source
or guard was changed to accommodate the filesystem. The check metadata records
the sampled fixture peak and successful cleanup; it is not a universal memory
bound. Compiler concurrency stays at one, Rust test concurrency at two.

The [source manifest](source-manifest.json) verifies every recorded Rust source,
manifest/lockfile, test fixture, generated contract and check script against the
qualified Git commit. It also verifies 21 frozen reference source hashes against
pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204` using Git objects only.
The Samba fixture manifest describes manual derivation from the frozen tests;
neither Python plugin code nor its smb.conf parser was executed. The
[evidence hashes](evidence-sha256.json) bind this packet's files except the hash
index itself. Raw successful transcripts are preserved, including terminal
blank lines. Preliminary pre-commit checks remain in ignored local cache and
are excluded from these acceptance totals.

Integration can merge this code independently from the other pending planning
branch. Only additive exports in domain/observations are shared-file changes.
No dependency, Cargo lock, generated wire type, schema migration, operation,
service policy, API, CLI or UI change is needed. Match each share's requirements
by complete expected device metadata when composing a configured preview; retain
its desired flags, each route and every unmapped share. Do not infer complete
live dependencies from an empty configured row.

The integrator still needs protected bounded configuration capture/revisions,
retained descriptor/host-namespace traversal and UUID/backing composition for
share sources and pool members, plus service/account/global-inheritance
qualification. Before writes, bind all configuration and evidence into the
authorized plan, acquire complete shared claims/locks, revalidate at effect time
and use the durable recovery path. Saved JSON import does not reproduce complete
smb.conf semantics; includes, global inheritance, custom directives and direct
smb.conf import remain unsupported and must never be silently discarded.

No Samba service, account, managed configuration/destination, package, Docker
engine, mount topology, Pi or remote host was changed. Samba cases use supplied
bytes/types only; existing kernel tests use owned scratch and child processes.
These tests prove deterministic local logic, not installed config acceptance, service
availability, physical dependencies, unmount safety, runtime-loss shutdown or
crash-safe share effects. DSK-001, MNT-001/MNT-002, RT-001, UI-003, SRA-001/SRA-002,
BKP-001, P04 and cutover remain open. Inventory p95 61.401 ms versus 20 ms is
unchanged. The frozen ARM64 bundle excludes this branch until review/integration
and a new source/package identity.

Estimate 12 human engineering hours; slice review at 18. P04 remains 320 hours
with a separate 480-hour review. Actual human and agent effort are unmeasured;
recorded tool durations do not establish either review threshold.
