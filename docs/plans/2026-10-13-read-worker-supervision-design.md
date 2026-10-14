# RW-040 independent read-worker supervision

Base: local main `e1faf39`. Retained collection is implemented but its window is
cooperative. Add reusable unregistered supervision mechanics and an opt-in local
fixture executable; do not register a production launcher or enable the combined
qualification probe.

## Process ownership

A manager admits one request without a queue. A background monitor owns its
single-flight guard, guardian child and exclusive stdin writer. A request ticket
can cancel or wait; dropping it cancels. Returning a deadline/refusal does not
release admission. Only a valid guardian completion frame plus successful exit
releases the guard. An unexpected guardian exit poisons manager admission because
descendant cleanup is unconfirmed. No public reset is provided.

The dedicated guardian establishes a new session and Linux child-subreaper status
using safe rustix calls before spawning the worker in its own process group.
It inherits only the manager liveness pipe, gives the worker null stdin/stderr,
and captures stdout nonblockingly. It checks a four-second monotonic deadline,
manager-pipe EOF/cancellation and worker output/exit independently of worker code.
It lowers its own soft NOFILE to 226 before spawning; the worker inherits it.
Worker bootstrap sets SIGKILL on parent death and brackets the setting with the
expected guardian PID/group/limit checks before permitting collection.

On completion or refusal, the guardian repeatedly kills/reaps its direct children
through pinned pidfds. It never signals a reusable numeric group ID. Subreaper
adoption brings orphaned grandchildren,
including a child that changed process group/session, into this owned child set.
Read only bounded `/proc/thread-self/children`, open pidfds for adopted children,
verify parentage around the handle, signal through pidfds and use nonblocking wait.
The guardian performs no other work or unrelated child launches. Keep cleanup
running if a kernel task cannot yet be reaped; no cleanup acknowledgment or new
admission is allowed while it remains. Service/cgroup teardown remains the final
guard for guardian failure; this slice cannot prove cleanup after SIGKILL of the
guardian itself.

The manager independently checks its original four-second request deadline. A
ticket can return refusal while monitor cleanup continues. Command spawning is
synchronous inside the monitor: the caller deadline remains bounded, but blocked
exec/launch termination is not qualified and admission stays held until resolution.

## Closed transport and launch boundary

Use a five-byte header: one fixed status byte and a big-endian body length. A
successful body is one complete JSON value, at most 256 KiB; refusal has no body.
Reject partial/trailing/oversized frames, malformed JSON, nonzero worker exits
and late completion. Never expose an intermediate body. Guardian output is a
cleanup acknowledgment only after all owned children are reaped. A completed
frame without successful guardian exit is insufficient.

Commands are trusted Rust-side selections supplied to the unregistered mechanics,
not request/wire image selectors. A separately integrated launcher must protect
the fixed executable/configuration selection and arrange role bootstrap. No
production command, RPC, route, schema, service, package or executable policy is
added here. The `read-worker-probe` Cargo feature exposes only a local fixture
binary under tests/support. It is disabled by default and never packaged.
The fixture's nonroot collector case uses the genuine public entry/root refusal;
no UID, namespace, clock, proc/sys root or owner-authentication override exists.

## Alternatives and local evidence

A dedicated guardian keeps subreaper/session changes out of Core and the caller.
An async timeout around collection cannot kill synchronous reads. Making Core a
subreaper would mix unrelated children with this request's cleanup. Releasing a
permit on ticket drop would allow overlap before old processes are gone.

Local owned-process tests cover deadline on blocked work, cancellation/drop,
manager death, descendant adoption/reaping, changed child groups, bounded and
malformed output, nonzero exits, successful exit plus cleanup, contention and
guardian-failure poisoning. A bootstrapped nonroot worker must pass resource
admission then fail the genuine configuration root guard. No positive host
collection, Docker, block probing, Pi, SSH or guest launch is authorized.
Preserve exact-source lossless logs; run workspace checks with the opt-in feature
and normal contracts/repository/ARM64 harness checks. Builds/general scratch stay
on disk; only precompiled existing storage fixtures use owned cleaned tmpfs.

Service caps and syscall/capability restrictions stay unchanged. Installed
subreaper/pidfd/session behavior, root-positive composition, peak resource
headroom, launch blocking, performance and cgroup failure teardown remain open.
The implementing agent reviews locally; independent review remains pending.
No P04, defect, footprint or cutover gate closes; locks, effects and recovery are
outside this slice.
