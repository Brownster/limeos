# Transition-only observation polling: local evidence

Source `05049bb1b79204392bd0e03e0ee00df661442206` implements the
[polling design](../../../plans/2026-10-31-b1-transition-only-observations-design.md)
and [handoff](../../../p04-b1-transition-polling-handoff.md). This packet
qualifies real file-backed Store logic and the sole async Core worker using
normalized fixture facts. It adds no production collector or scheduler and
establishes no installed, ARM64, package or Pi acceptance.

## Idle-write comparison

The retained queued path's baseline performs 32 unchanged healthy samples after
one initial observation. It records **480 SQLite row changes** and **8,200,192
process write bytes**, while no alert or notification transition occurs. WAL
size increases from 630,392 to 1,083,592 bytes. This is a library-path fixture,
not measured installed idle behavior and not an extrapolated daily rate.

The new path is measured in an isolated test process after its first committed
healthy baseline. **2,880 subsequent unchanged polls** advance simulated time
by one day at thirty-second intervals. SQLite changes, alert/outbox/audit
counters and durable sequence remain unchanged; database and WAL bytes are
identical. The process records **zero write bytes** during that interval.
Guard logs and harness output are outside the measured process interval.
This is a simulated day, not a real 24-hour installed soak or total service
resource/footprint measurement. The dedicated idle guard retains the raw marker.

The baseline's only modified source is archived as `baseline-tests.rs.gz`;
its recorded parent and full input map identify the rest. The final source keeps
that positive-write queued-path regression deliberately. Existing/recovery jobs
retain their old behavior; future normal supervision must use the new poll seam.

## Final checks

The final scoped suite passes **810 runtime tests and nine compile-fail
doctests**: 25 API, 47 Core unit, seven Core integration, 562 domain and 169
persistence runtime tests. Nineteen new native tests cover zero writes, actual
worker exclusion/replay/deadlines, thresholds, muted/re-enabled incidents,
recovery, incomplete observations, policy/grant changes, rollback, restart,
skips, origin corruption and legally pruned history. The compiled schema-13
fixture preserves configured policy, history and queued intent bytes. Its
injected partial-DDL failure rolls back the schema/version/generation, then the
real upgrade succeeds. Older compiled migration/recovery fixtures pass.

Strict Clippy over affected packages/all targets, actual contract regeneration,
all 242 frontend tests and the checked frontend build pass. Formatting,
repository hygiene and diff checks pass. No UI behavior changed, so this packet
makes no fresh browser journey claim. The new typed proof event and schema-14
SQL are generated from their real emitters; historical generated SQL is retained.

The five final guards bind identical maps of **722 committed inputs**.
`source-match.json` records exact source, check identities, memory minima and
hashes. All checks use repository-disk scratch, one Cargo job/test thread and
the unchanged sampled **3-GiB MemAvailable floor**; starts require 4 GiB.
Original records and logs are retained under `checks/`. The first Rust attempt
failed private-type re-export visibility. Preliminary passing runs belong to
their recorded inputs; the second run changed inputs during development and
does not qualify the final source. The generated-output and migration-test
changes are followed by the final frozen run. No failed/draft run is promoted.

`source.patch.gz`, `reproduction-sources.tar.gz`, launcher/local records and
`SHA256SUMS` preserve the changed source and closed local commands. Extract and
review the scripts before reproduction; their original repository paths must
be adapted on another machine. No broad installed or release suite ran.

## Remaining gates

Fresh protected collection, concrete resource replacement, bounded production
supervision, history/audit compaction, external destinations and transports,
protected actual-state migration and installed API/UI/concurrent-tick/restart
and real idle-write evidence remain open. Schema 14 requires fresh packages,
upgrades and a preserved pre-upgrade database for an older-code return path.
Current ARM64/Pi qualification remains separate. A1 reliability, wider A/B/P04
parity, P05's assistant and P06's rehearsal still block cutover. No phase or
defect closes and no inventory performance target changes. Holly's Pi, frozen
Python, deployed services and external recipients were untouched. Nothing was
pushed.
