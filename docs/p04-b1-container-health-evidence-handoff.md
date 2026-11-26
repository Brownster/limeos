# Fresh retained container health for B1

Source `7f1256d` implements the
[design](plans/2026-10-31-b1-container-health-evidence-design.md), adding
`docker::health_evidence::inspect_container_health()` to the genuine Engine
collector. The fixed entry requires actual effective UID 0 before opening
`/run/docker.sock`, then authenticates root ownership and each connection's
kernel peer before any HTTP. It is a read-only library seam; no production
worker/RPC, service, timer or scheduler invokes it yet.

The returned `EngineHealthEvidence` retains the existing complete storage/PID
owner, socket descriptor/name identity, admission ledger and original lifetime.
Each complete inspection selects full container resource, display name,
effective running state, restart policy, configured healthcheck state and its
result. Complete enumeration and Engine identity bracket two inspections per
consumer. Revalidation repeats those authenticated GETs and compares every
selected health and original identity fact without renewing the clocks.

The health path uses EngineWideV1, the stack inventory's profile: up to 64
containers of any state (stopped ones count), all of which may run; 64-KiB
inspect bodies and a selective listing of up to 1,024 rows; 8-MiB lifetime
body and 4-MiB live payload allowances; and the existing
four-second collection/two-second request/original five-second lease. The
complete original and health report must fit 256 KiB. It reads no mounts: a
container's storage facts carry its identity and state only, so neither a
mount's shape nor the Engine's mount total (the 256 that bounds the stack
inventory) can refuse it. Past 64 containers the listing keeps every
container with a live process or restart loop (`running`, `restarting`,
`paused`, `removing`, or a state the Engine adds later) before the stopped
ones (`created`, `exited`, `dead`), each group by name, and counts the rest in
`truncated`; a truncated read is never complete. More than 1,024 listed rows
or an oversized answer still refuse the whole read. No service limit,
allocator/RSS guarantee or installed capacity acceptance is added.

Until 26 November 2026 the path used CombinedV1 (16 consumers, eight running,
32 mounts, a 48-KiB report) and refused anything larger. An Engine of 18
containers, 14 running, with 51 mounts, refused at its listing before any
inspection, so the
cutover's container-health read failed and the 26 November migration rolled
back.

The new projection and Compose share the existing validation state machine.
Failure, cancellation or reentrancy permanently invalidates both public health
validation and its borrowed storage entry. Restoring an earlier response cannot
restore readiness. Storage-only callers retain their previous selections and
reports; health fields are not required on their legacy fixtures.

Only `always` and `unless-stopped` produce monitoring signals, matching the
frozen domain rules. `no`, `on-failure` and the older empty disabled name remain
unmonitored. The empty-name interpretation follows the pinned
[Moby v28.0.0 definition](https://github.com/moby/moby/blob/v28.0.0/api/types/container/hostconfig.go),
not a claim about the installed Engine version. Unknown/malformed policy
refuses. Effective running excludes paused, restarting and dead states.
Stopped or unhealthy monitored containers fail; starting retains the frozen
rule. Full Engine IDs bind signal keys and concrete resources; selected names
appear in summaries. A name cannot become grant authority.

An absent/null healthcheck or explicit `["NONE"]` with no result is unreported.
A configured/inherited check without a result, or unknown result text, is
unavailable. Malformed shapes and contradictory disabled/result facts refuse.
Unavailable produced signals make the **container** snapshot incomplete; a
verified stopped failure can be known independently of its check result.
Future aggregate completeness must separately account for SMART, host, required
mounts and protection facts. Commands, health logs, environment, unrelated
labels and credentials never enter reports or signals. Snapshot serialization
cannot reconstruct its opaque owner.

Local qualification uses actual owned Unix listeners and the production HTTP
collection/revalidation code. Successful fixtures use private unit-only socket
credentials and clocks; production nonroot/root-peer refusals occur before any
HTTP. This establishes no successful installed UID-0 Engine, host namespace,
service resource/footprint or ARM64/Pi evidence. The
[source-bound local packet](rewrite-evidence/p04/2026-10-31-b1-container-health/README.md)
records 986 runtime passes and 33 compile-fail doctests, eleven new native health
tests and two new compile-fail owner/snapshot checks. Two existing child-entry
helpers are ignored by the outer harness and invoked by passing parent
regressions. Strict Clippy over dependent executor targets, actual contract
drift, formatting and repository checks pass. Three final guards match 724
committed inputs; the preliminary changed-input run remains distinct.

The next runtime slice must adopt this owner in an independently supervised
protected health worker, preserve original lifetime/remaining-time bounds across
the authenticated reply, resolve current concrete scopes and feed the
[transition-only Store seam](p04-b1-transition-polling-handoff.md). Legacy name
silences and replacement/retirement of old IDs still need explicit binding and
migration. Fresh SMART/mount/protection facts, fair scheduling, safe compaction,
external notifications, installed API/UI/restart/real idle-write evidence,
packages/upgrades/return paths and actual-state migration remain open. B1,
A1 reliability, wider A/B/P04 and P05/P06 cutover remain open. No Pi, real Engine,
external recipient or frozen app was touched; nothing was pushed.

Runtime adoption is now implemented by source `1219844`; the
[protected health runtime handoff](p04-b1-protected-health-runtime-handoff.md)
records its sealed worker/RPC and partial Store path. Normal scheduling and
installed root-positive qualification remain open.
