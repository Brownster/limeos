# Retained Engine-declared Compose membership

`limeos_executor_container::docker::compose_evidence::inspect_compose_membership`
adds an unregistered read-only prerequisite for stack-aware coordination. It
requires actual effective UID 0 before touching the canonical `/run/docker.sock`,
then applies the existing root socket/peer policy and closed CombinedV1 admission.
No caller-selected endpoint, UID, profile, clock or numeric ceiling exists. No
executor command, RPC, route, schema, service or package activation invokes it.

`EngineComposeEvidence` owns the genuine authenticated Engine collector and its
retained socket, original lifetime and spent input/payload ledger. `snapshot()`
returns selected Compose metadata; `storage()` borrows the complete original
storage/PID owner from the same collection. Both validation entry points freshly
compare all selected facts, including Compose. A serialized snapshot cannot
recreate the owner or grant stack/file authority.

Each complete membership row keeps full container resource, image, original start
timestamp and running state. Stopped, mountless and non-Compose consumers remain
visible. A Compose row has the declared project name, service, one-off flag,
working directory and ordered config-file names. Multiple replicas and one-off
containers remain distinct members. Environment, unrelated labels, credentials,
config hashes and environment-file labels are omitted from reports.

The selected label conventions follow pinned Docker Compose v2.29.7:
[label definitions](https://github.com/docker/compose/blob/v2.29.7/pkg/api/labels.go)
and [project-label construction](https://github.com/docker/compose/blob/v2.29.7/cmd/compose/compose.go).
This is a supported metadata grammar, not installed-version qualification. Project
names use lowercase ASCII alphanumeric/underscore/hyphen and service names use
ASCII alphanumeric/underscore/hyphen/dot, both at most 64 bytes with an alphanumeric
first character. One-off values must be exactly `True` or `False`.

Working directories and parsed file names must be canonical absolute UTF-8 paths,
at most 512 bytes and 16 components, without controls, commas, empty/dot/dot-dot
components or trailing separators. One to four distinct config-file names retain
their declared order. The comma-delimited label is not a trusted argv/file
selection and cannot prove paths with literal commas; a later managed-file
binding must establish the actual approved selection independently.

Explicit null/empty labels, or only unrelated string labels, produce `non_compose`.
Missing Config/Labels, nonstring label values or any partial Compose namespace
metadata refuse the whole request. Hook/relay markers are outside this initial
subset and refuse. One project name cannot merge different working directories
or ordered file selections; those conflicts return unavailable rather than
guessing a stack. No trusted filesystem stack or Store resource key is inferred.

The complete container list and Engine identity bracket two inspections per
member. Project metadata is compared in both passes, then in every fresh
revalidation along with the original storage/PID facts. Source/mount, PID,
image/start/state, membership, Engine or selected-label changes refuse. Original
wall and monotonic five-second leases do not renew. Successful checks observe
selected facts and cannot establish a transaction against other Docker clients.

The new owner permanently invalidates after validation failure, dropped in-flight
work or reentrancy. A concurrent refusal also invalidates the first result; a
later successful comparison cannot restore readiness. Borrowing the base owner
cannot bypass this guard. Old storage callers retain their prior behavior.

Bounds stay 16 complete consumers, eight running, 32 selected mounts, 64 KiB
response bodies except the smaller version response, and 2 MiB lifetime HTTP
bytes/shared counted live payload. The original storage and Compose reports
together must fit 48 KiB of compact JSON. Strings/vectors and temporary comparison
overlap use the existing ledger; data drops before its reservation. No limit is
raised. The Compose path rejects duplicate JSON object keys, including decoded
escape equivalents, before reading the repeated value. Old standard/CombinedV1
JSON decoding remains unchanged. Peak allocator/RSS/cgroup headroom is unqualified;
the supported counts do not qualify this Pi's larger running membership.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-membership/README.md)
covers pure metadata and validation-state cases plus owned Unix-listener fixtures.
Successful listeners use explicitly private unit credential/clock substitutions;
they qualify no production UID-0/installed Engine. The public nonroot refusal
precedes canonical endpoint access. Root-policy and wrong-kernel-peer fixtures
send no HTTP before refusal. Existing storage fixtures and reports still pass.
See the [design](plans/2026-10-14-compose-membership-evidence-design.md).

This supplies authenticated declarations in the caller's context, not independent
daemon-PID identity, host namespace proof, actual protected Compose files, empty
stack discovery, managed-stack membership or approved lifecycle actions. The
[protected Compose file reader](p04-compose-file-evidence.md) now supplies retained
actual-file identities/hashes for one recognized operator and optional fixed
managed file. Approved catalog/management and genuine Engine-to-file binding are
still required; neither owner infers a trusted stack resource. The
[storage impact planner](p04-storage-impact-planning.md) still lists stack ownership
and complete physical/runtime/claim evidence as unresolved. Combined-worker/probe
integration, actual stack files/claims/locks/approvals/effects/recovery, independent
review, root/installed qualification, performance and cutover remain open.
Synchronous kernel work still requires independent supervision. Service guards
and the frozen signed ARM64 candidate are unchanged; no Pi, SSH, real Docker or
guest was used.
