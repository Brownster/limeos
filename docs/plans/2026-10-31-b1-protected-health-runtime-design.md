# Carry fresh container health through the protected worker and Store

Baseline `d92c989` has a genuine retained Engine health reader and transition-only
Store polling, but no protected runtime path connects them. Add the fixed worker
role, read-only RPC, Core freshness owner and bounded partial container polling
bridge. Keep full B1/P04/P05/P06 cutover acceptance separate.

## Protected acquisition

Reuse the existing packaged `limeos-stack-read-worker` sealed image with two
closed health guardian/worker roles. This avoids another executable/image budget
and shares the existing guardian, parent-death checks, single flight and verified
descendant cleanup. A direct asynchronous Engine call in the root RPC handler
would leave synchronous kernel work outside the independent deadline. A cache
would weaken fresh sampling and require another durable replay policy.

Add a root-read-only ceiling flag `allow_container_health_read`, default false
for existing policy files. It may coexist with storage/stack reads, never host,
advisory container, log or effect permissions. New package-created storage
ceilings include the flag; preserved existing ceilings are not silently rewritten.
The storage role accepts a fixed ContainerHealth request containing only version
and a 64-hex nonce. Kernel Core UID authorization precedes frame parsing.

The health launch selects only the existing installed/shadow protected package
image, preserves its original four-second preparation/monitor window and clears
inherited environment. A bounded nonce is its only health selection. Health
roles reject stack/configuration selectors, and stack roles reject the health
selector. Loaded roles verify current memfd seals/digest. Worker bootstrap and
genuine `inspect_container_health()` precede report construction; no endpoint,
UID, namespace, clock or numeric profile is exposed.

Use the same root-reader gate for stack, storage and health work. The monitor
retains the image and gate through confirmed cleanup even when a waiter expires.
Existing permanent cleanup poisoning remains in force. No service memory,
descriptor, task, capability or filesystem ceiling increases. Apply the current
manager/guardian/worker admission checks and the genuine Engine's unchanged
budgets before/after acquisition, revalidation and bounded serialization.

## Freshness without renewal

Add a bounded plain domain cohort: version, nonce, original Engine ID/wall time,
container-only completeness and at most sixteen full resource-bound optional
signals. It has no constructor/deserializer for retained Engine authority.
Bound strings and member count during decode, validate sorted unique full IDs,
container kind/key/resource consistency and exact completeness, and cap encoded
reports at 32 KiB. Commands, logs and raw inspect bodies do not cross the RPC.

Core starts one monotonic window of at most four seconds **before** opening or
sending the fresh RPC. A random nonce is bound into the protected worker's
report; the reply must match it and the original request/receive wall-time
interval. Root socket ownership/name identity and each kernel peer must match
the production root policy. Test substitutions remain private unit-only seams.
Successful framing/ready status and the exclusive expected payload are required.

The original Engine lease lasts five seconds and begins after Core starts its
four-second window. Completing before that original Core deadline therefore
dominates the Engine expiry; report transport cannot renew it. Keep that deadline
in a private nonserialized Core owner, including while waiting on the database
worker. Do not subtract whole collection time repeatedly or create a new TTL at
receipt. A caller may only shorten the window. Late framing, queueing, policy or
clock changes refuse; reconnect/replay requires a new nonce and genuine read.

Add a Store completion entry with an additional trusted monotonic deadline.
It uses the minimum of the original private poll lease and the supplied read
deadline, rechecking after the real transaction lock and before commit. Original
nonce/active-lease equality still uses the unchanged admission deadline. Existing
queued and transition-only APIs retain their behavior.

## Core consumption and scope

The internal bridge can obtain one fresh cohort, resolve all actual concrete
HealthRead resources through current Store admission and consume its eligible
signals before the original read deadline. Root/Core trusted read transport may
observe the complete inventory internally; no browser receives unchecked counts
or resources. Any missing member grant refuses the complete Store sample.

This path supplies container signals only. The resulting aggregate health sample
stays explicitly incomplete until fresh host/SMART/mount/protection sources are
registered. It must not silently reinterpret configured legacy container-name
silences or resource replacement; unsupported bindings refuse for later review.
No production timer starts in this slice. The future supervisor must preflight
current recurring authority/due state, settle old queued work, coordinate fair
passes and integrate the remaining sources and replacement/retirement rules.

## Local acceptance and remaining gates

Test bounded cohort decoding, nonce/receipt/peer refusal, original read deadline
through actual Store queue/lock waits, whole-sample grant checks and atomic
partial container transitions. Owned Unix listeners qualify the authenticated
transport under private credentials; owned sealed-image/guardian fixtures exercise
the real child bootstrap and public nonroot collector refusal. Existing opt-in
supervision/launch tests cover cancellation, parent death and descendant cleanup.
Successful installed UID-0 Engine composition remains a disposable-host gate.

Run affected/full shared-protocol Rust tests and opt-in read-worker fixtures,
strict Clippy, real contract generation/drift, relevant frontend contract build,
package/reference policy checks, shellcheck, formatting and repository hygiene.
Use repository-disk scratch, one Cargo job/test thread, 4-GiB start threshold and
unchanged sampled 3-GiB floor; preserve all attempts and exact source bindings.
No actual host policy installation, service activation, Pi, external send or push.
Installed packages/upgrade/return paths, resource/performance/real idle-write
evidence, complete health/notification parity and full cutover remain open.
