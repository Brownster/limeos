# P03 execution tracker

Status: implementation in progress. Started 2026-10-05 after P02 local qualification. P02's reference-host comparison and hardware gates remain pending; the overnight continuation request authorizes local progress while wybie stays untouched.

Estimate: twelve implementer days (96 hours) for RW-030 through RW-033. Review the cutover scope at 144 hours. Approximate allocation: 24 hours for typed plans/approvals and durable queueing; 32 hours for the restart executor, receipts and crash reconciliation; 24 hours for start/stop, logs and the Compose plan foundation; 16 hours for disposable-host setup and qualification. Record actual effort separately from elapsed time and approval waits.

The first slice implemented typed restart preconditions, plan-bound single-use approval, current-grant authorization, idempotent queueing and cancellation in the core authority store. The integration continuation below connects these to the installed executor and native container control. All mutations remain confined to a disposable VM; wybie and the workstation's real Docker socket remain untouched.

The phase cannot be signed off until its complete interruption matrix passes on a separate test host, the browser/assistant authority paths agree, and the narrow executor's independent ceiling and protected receipts have been exercised.

## First slice validation

The [workspace test log](rust-tests.txt) records 59 passing Rust tests; [validation metadata](validation.json) identifies the checked source. Local commits are `53bc472` (observation deadline), `9b81fae` (restart authority) and `48642e8` (executor receipts). New authority regressions cover changed plans, wrong approvals, expiry, fresh container incarnation/state, revoked/scoped grants, forged roles, ownership of plans/events, racing duplicate requests, approval consumption rollback, queued cancellation and uncertain dispatch locks across restart. Transactional v1-to-v2 migration preserves existing identity; an injected migration failure rolls back the new table and schema version.

Executor fixtures prove that wrong peer UIDs, disabled/unmanaged ceilings and changed resource state prevent effects. A prepared receipt is committed before dispatch. The test process exits immediately after writing a separately durable simulated effect, before recording its result. Reopening the store returns the prepared receipt and performs no second effect. Timeout, acknowledgement failure and failure to persist a result retain uncertainty and the per-resource barrier. Accepted effects remain distinct from independently verified success.

Formatting, Clippy with warnings denied, generated contracts, repository boundaries, frontend tests, strict TypeScript and the production build pass. The dependency policy checks pass and the cached advisory audit finds no vulnerabilities in 214 dependencies; the new executor library adds no third-party dependency. All thirteen [release-gate rejection probes](gate-probes.json) pass.

The full socket fixture suite needs execution outside the sandbox, which prohibits its temporary Unix listener. The protected-receipt tests initially rejected the test directories because Rust tempfile uses the caller's default directory permissions. Fixtures now explicitly create private `0700` directories; the production permission checks remain strict.

Review also reproduced a P02 collector defect: 64 stats requests timing out in groups of four took 32 seconds and exhausted the enclosing 15-second inventory deadline. Optional statistics now have an independent eight-second budget. The regression preserves all 64 inventory rows and leaves unavailable metrics unknown. This fix is source-tested; it is not present in the inherited P02 Debian package used for the earlier VM measurement.

The first slice took approximately 40 minutes of elapsed agent work on October 5, excluding the earlier approval wait. This is not a human implementer-day measurement and does not revise the 96-hour phase estimate.

## Restart integration qualification

The restart path now runs end to end: current-session plan, separate human approval, durable queue, bounded dispatcher, independent executor preflight, protected receipt, fresh verification and stored owner-scoped progress. Task callers are bound to kernel UID/task/resource/current revision and can propose or submit an already approved plan, but their protocol has no approval command. Session revocation during inspection is rechecked before a plan or job is committed. A duplicate queue response can be recovered without contacting Engine.

Authority schema v3 migrates v1/v2 transactionally and records receipt plus verification with the terminal event. Executor receipt schema v2 keeps the resource barrier through accepted-but-unverified effects. Timestamp checks validate and normalize calendar fields and fractional precision. Missing, prepared or ambiguous receipts retain `needs_intervention`; reconciliation only reads receipts. It never resubmits an interrupted effect.

The [real Engine VM result](vm-result.json) records fifteen passing acceptance groups against Docker 20.10.24 on a disposable Debian 12 host. It includes a verified restart, duplicate queueing, changed plans/incarnation, the independent managed-ID ceiling, real kernel account separation, browser/task policy parity and immediate grant revocation. Explicit expiry and queued cancellation cause no effect; in-flight cancellation returns a conflict. All six combinations of killing core or executor before preparation, during the Engine request and after the effect but before its response passed without another effect. Fresh standard/shadow AMD64 packages were built with the pinned toolchain inside the same isolated guest; package/image hashes identify those artifacts. There are no host mounts or production container images/data.

The shadow unit has a separate receipt directory and a fixed `read-only` argument. It refuses a write-enabled root policy. Removing shadow preserves the standard core PID, both executors and authenticated reads. Standard fresh installs default to writes disabled and require explicit full-ID membership. The package adds a private `0700` executor state directory; core and assistant cannot read its receipts or open Engine.

The [local Rust log](rust-integration-tests.txt) records 69 passing tests. Clippy, formatting, generated contracts, repository boundaries, ShellCheck, frontend tests/build and cached dependency checks pass; the graph remains at 214 dependencies with no reported vulnerabilities. All thirteen [integration release-gate probes](integration-gate-probes.json) reject their deliberate violations.

The migrated restart label/control comes from the frozen container list. A native preview binds the selected identity and expiry; the user explicitly approves before submission. The [browser result](browser-result.json) and [progress screenshot](restart-progress.png) prove keyboard focus stays in the preview, Escape restores its trigger, one POST queues the ordinary job and progress survives reload. After a lost response, an explicit retry retains the same plan, approval and request key, recovering one durable fixture job without another approval. The browser test found an end-of-tab focus escape; explicit containment fixed it. The final package contains those same browser assets. Browser transport uses a closed fixture; the real API and Engine were qualified separately against the same application payload.

The first disposable build completed, then its harness failed before any operation test because a local `http` helper shadowed the imported module. A later qualification attempt passed expiry, cancellation and five interruption cases, then exceeded the fixture's twelve-second after-effect barrier wait. Timing instrumentation confirmed that the BusyBox PID-1 fixture consumes Docker's fixed ten-second stop grace: Engine requests took 10.66–10.88 seconds before the barrier, in addition to dispatch and inspection. The test now waits up to 25 seconds; production deadlines are unchanged. The final fresh guest passed the complete matrix. [Failed-run diagnostics](vm-failure.txt) are retained, and all discarded guests and test-only private signing keys were destroyed.

After removing shadow, the three application services used 9,244 KiB (9.03 MiB) combined PSS: core 5,098 KiB, container executor 2,889 KiB, host executor 1,257 KiB. All had zero swap. Twenty cached authenticated overview requests measured 1.59 ms median and 2.55 ms p95. These fit local guardrails derived from the architecture's 30 MiB/20 ms budgets; they do not complete the reference-Pi gate. Docker, the test proxy, browser and build tools are excluded. The assistant is not configured. There is no comparable Python workload measurement and no improvement percentage claim.

[Integration metadata](integration-validation.json) identifies runtime commits `1edfd62` and `2de8e2c`, browser fixture `986e811`, qualification tooling `e461651`, binary/package hashes and the exact test/image scope. All 103 files in the qualified [source bundle manifest](qualified-source-sha256.json), including the compiled browser assets, matched the workspace before recording signoff. Artifacts remain under `dist/p03-debian`; its signed test-repository snapshot can be passed to the VM runner for a rerun without compiling again while its disposable signing key remains valid. CI now includes the browser regression and P03 VM suite, but the changed GitHub workflow has not run here.

This continuation used approximately three hours of elapsed agent work including isolated builds and one slow cache scan; it is not a human implementer-day measurement. The 96-hour phase estimate and 144-hour scope-review threshold remain unchanged.

| Work package | Current status |
| --- | --- |
| RW-030: approved restart path | Implemented and qualified on the disposable AMD64 host, including native control, task-shaped caller and durable progress. |
| RW-031: narrow executor foundation | Restart framing, peer credentials, ceilings, locks, cancellation and protected receipts are qualified; extend these to the remaining operations. |
| RW-032: container/deployment operations | Start/stop, bounded logs and approved Compose plan/diff remain. |
| RW-033: representative host qualification | Local real-Engine fixtures pass; a redacted representative reference-stack configuration remains. |

## Remaining phase work

1. Add start/stop, bounded logs and the approved Compose plan/diff foundation. Port the remaining container controls onto the same authority and recovery path.
2. Extend disposable-host qualification to those operations and a redacted representative stack configuration. Qualify the combined UI/package payload on ARM64; this continuation qualified AMD64.
3. Extend footprint measurements as those operations arrive; qualify reference hardware in an explicitly suitable window separately. P02's hardware comparison and reference-host shadow installation remain pending.

No defect-register row is closed solely by this restart milestone. P03 remains in progress until its remaining operation and qualification work passes.
