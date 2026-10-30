# Registered owned alert and in-app notification history

Source `f7d5946` registers authenticated alert/notification reads and explicit
delivered-notification acknowledgment through Core's sole Database worker. The
dashboard exposes the stored history. Fresh collection, configured-policy binding
and the production scheduler remain unregistered. The
[handoff](../../../p04-b1-in-app-history-handoff.md) describes the runtime boundary.

| Final local check | Result |
| --- | --- |
| API, Core and domain runtime tests | 617 passed: 22 API, 41 Core unit, seven Core integration and 547 domain |
| Domain compile-fail doctests | Seven passed |
| Strict Clippy | Domain, API, Core and schema, all targets, warnings denied |
| Generated contract drift | Actual regeneration passed |
| Frontend tests and checked production build | 235 passed, no skips; TypeScript and Vite passed |
| Closed browser fixture | Desktop/mobile history, acknowledgment, owner disposal, paging, stale and forbidden states passed |
| Formatting and repository checks | Rust formatting, repository hygiene, scoped Ruff and diff checks passed |

The Core tests use actual file-backed SQLite, its production database worker and
the authenticated TCP HTTP router. Explicit normalized fixture health facts enter
through reservation, private lease, settlement and publication. They establish
owner isolation, current-grant refusal, CSRF, delivered-only acknowledgment,
revision replay, persistence after reopening and unchanged rows/events on GET.
Controlled worker delays cover expired queued mutations and committed replies
held beyond the original deadline. They do not establish genuine health collection.

The browser intercepts every request to a closed fixture domain. It does not
exercise live Core or protected workers. The production bundle is bound to its
125 frontend/contract input hashes and three asset hashes; the browser's served
asset hashes match. Screenshots were visually reviewed at desktop and mobile
sizes, including the mobile table's right-hand columns. Literal keys and escaped
summaries, delivered-only acknowledgment, GET reconciliation after a lost POST
reply, late replies from a disposed owner and absence of schedule/send requests
are checked. No whole-page horizontal overflow is observed; the table has its
own labeled scroll region and a mobile hint.

Four final guards (Rust, Clippy, contracts and browser) contain identical
699-input before/after maps. Every hash was checked against the committed blob
and current file. Their recorded HEAD is the preceding design commit because
the checks ran before the feature commit; content hashes establish the join.
The final frontend guard predates only an additional browser-driver assertion in
`tests/frontend/health_history.py`. Its frontend/contract inputs match the feature
commit, and the later final browser run tests the committed driver. The schema
generation and initial render runs retain their own earlier input identities.

Three failed Rust drafts are retained. They exposed a stale route expectation,
fixture acknowledgment revisions that ignored publication, and reopening SQLite
before the previous worker released its lock. Corrected fixtures pass in the
final full affected run. The initial passing browser/build records are also kept;
visual review prompted narrower table columns and explicit mobile scrolling
before the final build and browser run. Earlier results are not counted twice.

All guards finished with unchanged inputs and no sampled memory stop. The lowest
sampled host availability was 4,456,062,976 bytes in the final Rust run, above the
unchanged 3-GiB floor. Final guards observed no external compiler. Compilation
used one Cargo job and one Rust test thread, with all scratch/cache/logs on
repository disk. No production Pi, service activation, installed package test,
ARM64 test, external notification or push occurred.

`checks/` preserves compressed original guard records and full outputs, including
failed drafts. `launch-and-local/` preserves admissions, detached launch records,
full frontend step logs and local checks. Browser results/screenshots and initial
render records remain separate. `reproduction-sources.tar.gz` contains the exact
changed feature sources and validation drivers; `source.patch.gz` contains the
feature commit. `source-match.json` records committed source and UI joins.
`SHA256SUMS` covers every other file. Packages, binaries, browser profiles,
private keys and VM disks are excluded.

This closes the local registered-history slice only. Genuine collection,
configured policies and silence metadata, resource replacement, supervised
reservation/collection/publication/retention, installed restart/concurrent-tick
and idle-write checks, ordinary external notification parity, A1 reliability,
broader P04 parity and P05/P06 cutover gates remain open.
