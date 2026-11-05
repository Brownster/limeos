# Owned observation supervision through Core and schedule views

The [supervisor design](plans/2026-10-31-b1-observation-supervisor-design.md)
extends [admission-first container reads](p04-b1-container-admission-recovery-handoff.md)
into an owned periodic Core producer in source `d1ba5f0`. It is registered in Core's actual startup
and shutdown selection. Source identity and local acceptance are recorded in the
[qualification packet](rewrite-evidence/p04/2026-11-01-b1-observation-supervisor/README.md).

One serial future uses a one-second interval with skipped missed ticks. Each
pass has an eight-second deadline and a separate 100-ms best-effort permit
cleanup. It publishes one existing 32-row in-app page and prunes one retention
page before protected collection, with independent cursors. Unexpected durable
failures stop the producer/Core. Cancellation clears runtime registration and
diagnostics; any unfinished permit retains only its original bounded expiry.
No detached worker, new service or resource ceiling is introduced.

Query-only Store hints rotate pages of at most 32 owners. Each owner first gets
its oldest existing queued observation job, otherwise its next schedule after a
bounded volatile cursor. Examining a schedule advances that owner's cursor even
when it is paused, not due or refused. This prevents one owner's first schedule
from starving its later schedules. The actual schedule bound remains 128.
Hints grant no authority. Admission and completion reload current Store state.

Ordinary observations use transition-only polls. Existing queued jobs recover
under their original identity. Unsupported effect adapters retain their existing
blocked/skipped settlement; automatic effects gain no authorization here.
One genuine protected read supplies all admitted owners, at most 32, within the
earliest original permit/pass deadline and existing four-second source window.
Each owner completes in a separate transaction checking every actual full-ID
scope, revisions, nonce, retained subjects and original freshness. Nothing
filters unauthorized members or renews the read lifetime. Earlier valid commits
can stand if a later owner refuses or the reply is lost. Fatal completion errors
stop the batch before processing another owner.

Authenticated owned schedule views now distinguish a running producer from
`awaiting_producer`. The new `scheduled` status includes bounded historical
diagnostics: waiting, checking, partial or unavailable, with attempt time.
There is no healthy phase. Diagnostics are volatile, capped at 128 entries and
bound to schedule revision. Paused, authority-suspended and unsupported schedules
receive no runtime details. Reads still authenticate before projection and
invoke no collector, permission change or schedule settlement. Pages remain
explicitly partial while container observation is registered.

Generated schemas/TypeScript and the existing UI validate these states. The UI
shows partial coverage, unavailable service/permission guidance and attempt time.
The existing stale-data, ownership, permission, CSRF and lost-reply controls stay
in force. The root reader's policy and services remain operator-controlled;
this change does not install/start them or grant concrete container permissions.

Local runtime fixtures exercise owner and same-owner fairness, one fresh read
for multiple owners, original-job recovery/publication, current concrete grants,
source failure/retry, cancellation/single flight, bounded diagnostics and fatal
durable failure. Thirty-two stable real runtime passes advance simulated time
by 960 seconds with identical database/WAL bytes and zero process write-byte
delta. This establishes no installed 24-hour soak.

The source-bound packet records 1,727 workspace runtime passes, 54 compile-fail
doctests, strict all-target Clippy, actual contract drift, 244 frontend passes,
checked build and repository checks. Eight final guards agree on 738 committed
input hashes. All fifteen attempts remain retained, including two failed test
fixtures and earlier source maps. The final native observation finds 34 recorded
guard/command/Core process identities absent. Scratch stays on repository disk,
with one Cargo job/test thread and the unchanged sampled 3-GiB stop floor.

The actual Core binary starts and restarts against an isolated Store and serves
authenticated HTTP schedule status. An unprivileged listener is refused by the
production root-peer check before any request frame; status is visibly partial
and unavailable. This test uses no production credential/clock overrides and
establishes no successful installed root-positive collection. Separate closed
browser fixtures use exact built assets, with desktop/mobile visual review;
they establish no installed API/UI journey.

SQL schema 14, dependencies, package policies and service ceilings are unchanged.
The [4 November installed attempt](rewrite-evidence/p04/2026-11-04-b1-installed-observation/README.md)
adds a standalone fixture at `f3be3b3` with 18 passing scoped reference cases and
lint/format/repository checks. It reuses the source-bound `e417751` packages with
all 740 product inputs unchanged and exactly two new fixture/test files. The
real default root reader answers Core-UID storage inventory, and the actual
authenticated API creates a registered observation schedule. The first incident
wait times out with no occurrence/job/result/alert/notification. This supplies
no successful container-health, crash/restart or notification acceptance.
The failure packet cannot yet separate protected health collection from Core
admission/completion: add bounded final runtime and independent Core-UID health
reply observations before choosing a product fix. All seven recorded original
processes and VM scratch are absent, with no sampled memory stop. The installed
reader's policy and resource ceilings remain unchanged.

The [subsequent boundary-observation guest](rewrite-evidence/p04/2026-11-04-b1-observation-boundary/README.md)
at fixture `0705a79` passes 22 scoped reference cases and preserves the same
packages/production inputs. The ordinary incident wait still reaches no
settlement; the final authenticated schedule view reports unavailable. One later
independent read under actual Core UID 104 returns the genuine expected health
cohort in 69 ms, with matching nonce, Engine and full-ID member. This establishes
that installed root-reader path, while normal Core read/admission/completion
remains unresolved. The cohort is never inserted into Core or credited to its
timer. Five original native qualification PIDs and VM scratch are absent, with
no sampled memory stop. Core's `RestrictSUIDSGID=yes` versus its protected
`openat2` socket lookup is now a specific profile hypothesis; verify it under
the exact Core profile before changing confinement. No Core errno was captured.

The [installed Core-profile matrix](rewrite-evidence/p04/2026-11-04-b1-core-profile/README.md)
at fixture `afc7a43` now confirms that incompatibility under the complete copied
Core profile and actual UID/groups. The direct control and no-restriction/both
variants open the exact socket; current and explicit-allow-only profiles return
`ENOSYS`. The running Core unit hash/PID/authority counts stay unchanged. All 27
scoped reference cases and the canonical guest pass. This justifies the narrow
Core profile exception in the design, while the normal timer still needs a fresh
signed-package run. It is not the original timer's recorded errno or completed
B1 acceptance. The packet explicitly records the missed original QEMU identity;
four known native PIDs are absent and later inventory has no qualification VM.

Source `ce2e916` now applies that narrow Core profile exception. The
[5 November ordinary installed scenario](rewrite-evidence/p04/2026-11-05-b1-core-reader-profile/README.md)
passes with distinct signed `0.4.13+coreopen.1` standard packages, actual Docker/
Root reader and the normal timer, without a Core override or probe unit. One
incident is delivered, the exact history survives actual Core SIGKILL/restart,
the same container's recovery is delivered once, and acknowledgment/pause survive
a second real restart. Exactly two occurrences/jobs/results and notifications
remain. All 34 scoped reference cases pass. The packet separates current package/
fixture source from reused unchanged `e417751` executable/UI bytes and explicitly
corrects an inherited frozen-manifest field label without rewriting its inputs.
Seven recorded native PIDs and exact owned VM scratch are gone; no memory stop.
Every other Core setting and resource stays unchanged. This deliberately removes
the SUID/SGID file-bit restriction; NoNewPrivileges remains its separate exec
barrier. The covered standard AMD64 API/runtime flow passes, while installed
browser/UI, current ARM64/Pi/return/soak and remaining B1 parity still need evidence.

The existing package return requirement still includes policy before-images with
the matching pre-schema-14 database and package assets. Remaining host/SMART/
mount/protection sources, reviewed replacement/retirement, safe history compaction,
ordinary external channels and successful normal-timer/current-package/API/UI/
ARM64/Pi/return/footprint/soak acceptance remain open. A1/B1, wider A/B/P04 and
P05/P06 cutover remain unfinished; the historical inventory-performance failure
is unchanged. Holly's Pi and the frozen app were untouched. The later installed
attempt changes only its disposable guest. Nothing was pushed.
