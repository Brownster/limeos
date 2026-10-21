# Mounted health schedule UI, 21 October 2026

Source `3e494a0` implements [the management design](../../../plans/2026-10-21-health-schedule-ui-design.md):
native saved-schedule reads/create/pause/resume and fixed password self-enablement
through the already-registered routes. [The handoff](../../../p04-health-schedule-ui-handoff.md)
records the remaining B1 runtime and installed gates. Saved metadata still starts
no timer, observation producer, alert delivery or effect.

| Check | Evidence |
| --- | --- |
| Strict TypeScript | Passed |
| Production build | Passed; 31 modules, 211.13-kB JS / 64.61-kB gzip |
| Focused automation/enablement tests | 51 cases passed before the presentation correction; source identity remains in original records |
| Final complete frontend test command | Twelve test files passed sequentially; parent reports file counts, so no invented inner-case total |
| Final reference tests | 73 passed on `3e494a0` |
| Core browser flow | Passed in Chrome 150.0.7871.46 against a closed API fixture |
| Desktop/mobile visual review | Passed after fixing long opaque references and table-local scrolling |
| Expanded already-enabled/close-pending browser flow | Host memory guard stopped it; no result or screenshot, not acceptance |

The core browser fixture supplies failed Overview/history sources, then verifies
the independently mounted workspace, visible permission refusal, password
enablement, required sign-in with a fresh revision/CSRF, cross-midnight UTC timing,
creation, revision-bound pause/resume and truthful waiting status. It also checks
unavailable data disables writes, malformed forbidden responses clear private
rows, no alert/notification request occurs, and no POST is automatically retried.
The fixture logs only request keys, never credential values. There is no real
API/Store, installed package or host effect in this test.

`browser-2-fixture.py.gz` preserves the exact tested fixture hash
`96e0bec50df14de28efd166bf7060f7fe23d3ab71e32699982acd205483bc841`.
The current committed fixture extends it with already-enabled and closing during
confirmation cases. The interrupted browser-3 attempt does not qualify those
extensions. Unit cases independently cover same-frame/reentrant admission,
aborting owners, ignored-abort late replies, unauthorized/ambiguous outcomes,
fixed request shape, response bounds and schedules-only traffic.

`source-match.json` binds the tested frontend inputs to the committed source.
The build, final test and core-browser records predate the code commit but retain
their full before/after hashes. The browser test's later Python-only extension
is identified separately. The exact generated UI assets and screenshots are
archived without reserved build-output directories. The earlier mobile capture
remains as a diagnosed presentation failure, not the accepted layout.

The sampled host stop floor remains 3 GiB. Expanded browser-3 stopped at
3,175,645,184 available bytes, exit 143; no owned Chrome/VM process remained in
the recorded post-run audit. No current package, native ARM64, resource/idle-write,
production Pi or cutover acceptance is claimed. Existing packages contain the
earlier UI. No production connection or push occurred.

`SHA256SUMS` covers every other file in this packet. Failed lint, memory stops and
earlier attempts remain separate from passing qualification.
