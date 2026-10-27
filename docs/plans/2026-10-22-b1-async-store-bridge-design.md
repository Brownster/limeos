# B1 asynchronous Store bridge

Implemented in `7b44a01` and locally qualified on 26–27 October; see the
[handoff](../p04-b1-async-store-bridge-handoff.md) for exact evidence and remaining
collector/runtime/installed gates.

The accepted Store settlement is synchronous on the sole database worker. The
historical runner leaf instead proposes synchronous orchestration traits, and
Core has no way to rediscover observation jobs after a lost reservation reply.
The next slice connects bounded orchestration to the actual worker before a
protected collector or production timer is registered.

Use a compiled Core bridge around the existing `Database`, with a fixed trusted
production clock. One scheduling pass reads at most 32 advisory schedules,
validates the complete page, computes due decisions and submits each reservation
through `automation_reserve_current`. SQLite admission samples current time;
the page timestamp never becomes execution authority. Concurrent passes share
the same Store and occurrence uniqueness. Unsupported effects create blocked
records through the existing reservation path.

Preserve a single original monotonic deadline across worker queue admission,
lock waits, response waits and each step of the page. Expired queued work checks
that deadline before calling Store. Cancellation or timeout after a commit is a
lost reply, not rollback evidence: previously committed rows remain discoverable.
A multi-row pass is bounded orchestration, not one all-or-nothing transaction.

Add an internal query-only queued-observation page, sorted by original job ID
and limited to 32 rows with a continuation. Return only existing queued
`AutomationObserveHealth` jobs; no replacement occurrence, running state, lease
or other write is created. Bound borrowed intent bytes before decoding. These
are advisory hints: actual lease admission still reloads authority, checks the
oldest queued job of its owner and binds the selected concrete read resources.
Fair repeated pages allow a later supervisor to rediscover committed work after
restart or lost replies without trusting a tick's returned vector.

The bridge also transfers the private lease through the worker for begin and
completion, and exposes the accepted derived discard, in-app publication and
retention paths. Keep no parallel queue, Store, sequence counter, authority or
recovery ledger. Collection remains outside database work. Publication and
retention return their existing fair cursors; reads never perform them.

Keep the production timer unregistered. The overview cache omits the frozen
container restart-policy/healthcheck facts and is not a fresh protected
observation capability. Do not synthesize successful health observations from
that cache or treat default policy as migrated settings. Genuine fresh
collection, policy/resource replacement mapping, supervision, API/UI and
installed acceptance follow this bridge.

Verification uses the actual file-backed Store worker: concurrent passes and
restart rediscovery; private-lease completion and in-app replay; current grants,
pause and original deadline checks; a genuinely blocked worker whose abandoned
queued request must not write; bounded cursor fairness; unavailable/effect
handling; and query-only queued-job pagination. Run focused Core/persistence
checks, strict Clippy and repository checks serially with disk caches and the
unchanged 3-GiB memory floor. No Pi window or package/service activation follows
from these local checks.
