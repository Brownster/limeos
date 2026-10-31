# Collect fresh container health through the retained Engine owner

Baseline `df84fca` supplies transition-only Store polling but has no genuine
health producer. The overview cache lacks restart policy and cannot establish
the frozen container-monitoring predicate. Extend the authenticated Engine
collector with an additive health projection, then connect that owner through
the protected worker in a subsequent runtime registration slice.

Capture health in the existing complete double inspection instead of issuing
a second independent collector. A separate collector would duplicate endpoint,
budget, membership and lifetime machinery. Advisory cached inventory remains
useful for display; it does not supply this health evidence.

## Fixed acquisition and retained comparison

Add `docker::health_evidence::inspect_container_health()` with the same fixed
effective-UID-0 requirement and `/run/docker.sock` root socket/peer policy as
the genuine Compose reader. There is no public endpoint, UID, clock or numeric
profile selector. Private listener/credential/clock seams remain unit-test only.

Keep complete membership, Engine identity, original storage/PID facts and the
health projection from the same responses. A stopped or mountless consumer is
still present. Health selects full container resource, canonical display name,
running boolean, restart policy and healthcheck result/configuration state.
Both passes and each subsequent revalidation compare these selected facts.
Policy/name/health changes refuse just as original identity changes do.

Share the existing original five-second wall/monotonic lifetime, four-second
collection/two-second request deadlines, CombinedV1 lifetime input/payload
ledger and collection exclusion. Complete membership remains capped at 16
consumers/eight running/32 selected mounts. Combined original/health reporting
must fit the existing 48-KiB limit. No ceiling changes or installed headroom
claims follow. Reject duplicate JSON keys under the existing unique decoder.

Factor the existing Compose validation gate into a shared private admission
helper. Health has the same permanent failure/cancellation/reentrancy poisoning,
including when its original storage owner is borrowed. Existing standard and
Compose selections/report shapes keep their current behavior. No new health
comparison is imposed on storage-only callers.

## Selected health semantics

Require an object HostConfig/RestartPolicy with a string Name. Recognize
`always`, `unless-stopped`, `no`, `on-failure` and the older empty string meaning
disabled; unknown or malformed policy refuses the complete capture. The empty
case follows [Moby's restart policy definition](https://github.com/moby/moby/blob/master/api/types/container/hostconfig.go).
The frozen Python reader and existing Rust domain rules monitor only always
and unless-stopped policies; this does not add monitoring for batch containers.

Require a canonical slash-prefixed bounded container name and typed running
state. Use the existing pure `container_signal` rules with the display name,
then bind its key and concrete scope to the full immutable container ID.
Preserve the frozen starting/stopped/unhealthy behavior and severity. Legacy
name/silence migration still needs explicit identity binding; do not pretend
these new keys have already migrated old silences or retired resource state.

Absent/null healthcheck configuration or an explicit `["NONE"]` check means
unreported when no health result exists. A configured/inherited check without
a result is unavailable. Unknown result text is unavailable; malformed shapes
refuse. A disabled/no-check declaration with a contradictory result refuses.
Health commands, log output, environment, labels and credentials never enter
the selected snapshot or signals. The bounded decoder still admits their input
bytes and structure before selection; this adds no total-heap guarantee.

The Serialize-only snapshot contains normalized restart/health state and
optional resource-bound signals. Completeness describes container signals
only: unavailable monitored health makes it incomplete. Future aggregation
must separately establish SMART, mounts, host and protection completeness.
Snapshots and signals cannot reconstruct retained Engine authority. Public
constructors/deserialization for the opaque owner are absent.

## Qualification and remaining work

Use owned local Unix listeners to exercise the actual production HTTP collector,
double inspection and fresh revalidation. Cover the policy/health matrix,
stopped/mountless membership, configured missing/unknown status, second-pass
changes, permanent invalidation, original time, cancellation, root-peer refusal
before request bytes, bounds, sanitization and legacy selection compatibility.
Run affected executor-container/domain tests, strict Clippy, contract drift,
formatting and repository checks under disk scratch, one Cargo job/test thread
and the unchanged 3-GiB sampled floor/4-GiB start threshold. Preserve failed
attempts and final source bindings.

Successful listeners use the private test credential seam; they do not qualify
an installed UID-0 Engine, Pi host namespace, package or service budget. Fresh
SMART/mount/protection facts, concrete replacement and legacy-key migration,
protected health worker/RPC registration, fair supervision, installed idle
soak, external notifications and actual-state migration remain open. B1, A1
reliability, A/B/P04, P05/P06 and cutover gates remain unchanged. No production
Pi access, package activation, external send or push is authorized here.
