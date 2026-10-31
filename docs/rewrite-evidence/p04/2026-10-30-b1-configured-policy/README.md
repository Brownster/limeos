# Registered configured health policy

Source `a5c83784f9b84ea61c66e0b0d2f9f9a2eb41d1d0` registers owned health-policy
configuration through the sole Store worker, authenticated API and dashboard.
Immutable revisions bind observation reservation, private leases, settlement and
stream freshness. Atomic in-app publication checks the latest category/silence
predicate and records its revision. The
[handoff](../../../p04-b1-configured-policy-handoff.md) specifies the boundary.

| Final local check | Result |
| --- | --- |
| API runtime tests | 25 passed |
| Core runtime tests | 44 unit and seven integration passed |
| Domain runtime tests | 562 passed |
| Persistence runtime tests | 153 passed |
| Runtime total | 791 passed, zero failed or ignored |
| Compile-fail doctests | Seven domain and two persistence passed |
| Strict Clippy | Domain, API, Core, persistence and schema, all targets, warnings denied |
| Actual contract regeneration | Generated schemas, SQL, OpenAPI and TypeScript match |
| Frontend tests and checked build | 242 passed, no skips; TypeScript and Vite passed |
| Closed browser fixture | Final production assets, desktop/mobile, reply recovery and owner disposal passed |
| Source hygiene | Rust formatting, repository checks, scoped Ruff and diff checks passed |

Eighteen new native tests exercise actual file-backed SQLite and Core's existing
worker/router. They cover query-only reads, explicit default versus implicit
revision zero, current session/grants/CSRF, owner isolation, concurrent CAS,
no-op/replay, trusted deadlines, expired queued work and a committed response
held beyond its original deadline. Configuration creates no observation/job or
delivery. Normalized fixture health facts enter through genuine reservation,
private lease, settlement and publication methods; no protected collector runs.

Configured thresholds and mutes preserve incident facts. A native Store case
announces an unannounced incident only after category re-enablement or exactly
at silence expiry on a later fixture observation. Another changes policy after
collection admission, refuses completion and settles the original job as
precondition changed without replacement. Publication checks current policy in
one transaction and preserves incident-before-recovery ordering. Equal wall
times use durable event order. Current-policy freshness remains partial until
another observation settles.

Actual compiled schema-11/12 upgrades preserve queued/canceled jobs, default
history, publication and audit with revision zero. Older migration suites also
retain approval bytes, mount receipts, uncertain resource claims and atomic
failure rollback. Corruption cases restore compiled triggers after injecting
row/event changes, so startup refusal exercises data/proof checks rather than
merely detecting a missing guard. The 1,024-version bound refuses further policy
history without partial audit or row changes; compaction is not qualified.

The browser intercepts every request to a closed fixture domain. It tests the
actual production bundle with an unavailable overview, literal/case-sensitive
paths, escaped metadata, draft-only silence changes and preserved creation text.
It performs exactly three explicit saves: verified reply, actual fixture commit
with HTTP 403 `expired`, and actual fixture commit with transport failure. The
latter two reconcile by GET without repeating POST. A disposed owner's late
401 cannot expire its replacement. Unavailable data disables save; forbidden
access removes retained configuration. Desktop/mobile screenshots were visually
reviewed, with no whole-page horizontal overflow. This fixture does not exercise
live Core, protected collection or installed services.

Five final guards contain identical before/after maps for all 718 code inputs.
Every hash was checked against the actual committed blob and current file.
Recorded HEAD predates the feature commit; content hashes establish the join.
The frontend build's 133 source/contract inputs match the commit, and its three
asset hashes equal the final browser's served assets. `source-match.json`
preserves each draft's separate source differences; earlier results are not
added to final test counts or treated as final-source acceptance.

Fifteen original guard attempts remain retained. The first Rust draft exposed
an API future borrowing the request across an await. The second hit the compiled
schema-12 integrity reference, which correctly rejected schema-13 tables. The
third passed current behavior but found seven older migration fixtures whose
shared teardown left the new policy table behind. Corrections own the request,
extend the compiled schema reference without weakening it, and remove the later
fixture table. Final full affected suites pass. Clippy drafts exposed a large
private lease variant and a test initializer; boxing the policy and using the
initializer fix both, without changing public admission shape or service caps.

Earlier passing frontend/render attempts also remain distinct. Review added
invalid UTF-8 refusal and corrected deadline-envelope handling: HTTP 403 expiry
must reconcile a possibly committed save, while forbidden/malformed permission
bodies must clear access. Final unit/browser cases cover both. Initial executable
mode/format preflight corrections are recorded separately.

Final Rust ran 23:54:17–23:57:40 UTC on 30 October. Its minimum sampled host
availability was 5,887,909,888 bytes. All attempts remained above the unchanged
3-GiB floor; compilation admission required 4 GiB. No guard triggered, no input
changed during a run and no external compiler was observed. Checks used pinned
Rust 1.88.0, one Cargo job, one Rust test thread and repository-disk scratch/cache/
logs. The five final owned guard PIDs were observed absent after completion.
No host `/tmp`, production Pi, package/service activation, external send or push
was used. These observations do not establish installed footprint or ARM64/Pi
performance, upgrade, soak or return-path acceptance.

`checks/` preserves compressed original records and full logs. `launch-and-local/`
retains admissions, detached launch records, diagnoses, local checks and tool
versions. Earlier UI builds and both browser attempts remain separate.
`reproduction-sources.tar.gz` preserves exact feature sources and validation
drivers with executable modes; `source.patch.gz` preserves the feature commit.
`SHA256SUMS` covers every other file. Packages, binaries, browser profiles,
private keys and VM disks are excluded.

Genuine collection, concrete resource replacement, protected actual-state
migration, production supervision/retention, external destination revisions and
ordinary external delivery remain open. This completes the local configuration
slice, not B1, A1 reliability, broader A/B/P04 parity or P05/P06 Pi cutover.
