# A1 mount UI lifetime local evidence

Tested product source: `7bdc36b4bd297ce80e4837c72ac96c07e9938ddd` on
`engineer/p04-a1-ui-lifetime`, based on main `7df3f75`. See the
[handoff](../../../p04-a1-ui-lifetime-handoff.md) for behavior and actual remaining
acceptance. The registered component/export/props and Dashboard wiring are
preserved. No central, guest or production host changes are included.

**129 frontend tests pass** across eight files, including 40 new mount cases.
The production UI build and its pinned TypeScript check pass. Source and fixture
hashes, pinned dependency versions, protected-path comparisons, compressed logs,
runner scripts and the actual production bundle are retained. Each test file ran
sequentially; counts are native Node cases rather than one count per file.

Two expected baseline failures execute confirmation/history callbacks extracted
from the actual original component. A late approval still queued after its owner
was marked dead; an older history reply erased the newer result. The baseline
source hash is checked against main and its source/script/log are preserved.
These are callback regressions with supplied APIs, not browser reproductions.
Two additional intermediate failures demonstrate synchronous disposal during
state publication before the final guards/local promise preservation. All final
40 mount cases pass. A test syntax typo and reused runner's wrong scratch path
are retained as separate unsuccessful attempts.

Final cases cover independent read coalescing, stale history across mutations,
abandoned/restarted owners, pre-POST and approval-to-queue checks, duplicate
clicks, exact uncertain retry including expiry, explicit queued cancellation,
retained cancellation failures, malformed/oversized rows and HTTP 401/403 in all
six operations. Transport cases cover fixed routes/POST/CSRF, same-origin/no-store,
refused redirects, deadline/caller abort, strict UTF-8/JSON, streamed limits and
request refusal before fetch. Local fixtures are synthetic reviewed-intent
shapes; they never produce physical authority, a mount, a receipt or a DB commit.

`inputs.sha256` binds the source/contracts/fixture superset and unchanged Rust
intent/replay references. `metadata.json` records exact scope, counts, versions,
bounds and browser availability. `production-ui/` is the actual UI build using
the existing Dashboard, not an isolated mock component bundle. Its sizes/timings
are not performance qualification. `artifacts.sha256` covers this packet except
itself. No Rust test result is rerun or credited.

Restore the archived runner under its original scratch path and use the pinned
frontend dependencies to reproduce:

```text
uv run --offline --no-project .cache/a1-ui/check-frontend.py
npm run build                       # frontend directory
```

Set `TMPDIR`, `UV_CACHE_DIR` and `npm_config_cache` to repository disk caches as
recorded in metadata. Suites run sequentially; the temporary
`frontend/node_modules` link was removed afterward. The browser tool returned
empty apps/browsers; no preview server, screenshot, modal/focus/Strict Mode or
installed browser check ran. Qualification should use the owner's separately
accepted `8c5b498` fixture isolation correction when assembling the A1 candidate.

No package/guest/ARM64/Pi, physical admission, real mount, crash/receipt/recovery,
performance/soak, defect closure, A1/Stream-A/P04 completion or cutover gate closes
from this local packet. No production host was contacted.
