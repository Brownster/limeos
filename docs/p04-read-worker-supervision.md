# Independent read-worker supervision

`limeos_executor::read_supervision` supplies unregistered process mechanics for
the [retained collector](p04-combined-read-collection.md). No executor command,
RPC, route, service or production launch selection invokes it. The standalone
qualification probe's combined case remains blocked.

A trusted Rust caller selects a dedicated guardian command and calls
`Supervisor::start`. It returns a `Ticket`; `wait` accepts a complete report or
typed refusal, `cancel` requests cancellation, and dropping the ticket cancels.
One background monitor owns the child and single-flight guard. There is no queue.
The original four-second deadline applies to launch, completion and ticket
consumption. Cancellation/deadline can return while cleanup continues: new work
is refused as Busy until the guardian acknowledgment and exit are confirmed.
Unexpected guardian exit or unconfirmed monitor completion poisons admission;
there is no public reset.

The dedicated guardian entry calls `guardian(trusted_worker_command)` before any
other work. It requires one thread, no pre-existing children and piped stdin.
It establishes a new session and child-subreaper status, lowers only its own soft
NOFILE to 226, then spawns the worker in its own group. The worker must call
`WorkerBootstrap::establish(expected_guardian_pid)` before initialization or
collection. Bootstrap checks parent, session, group and inherited limit around
setting SIGKILL on parent death. `WorkerBootstrap::collect` invokes the genuine
public collector with parent-liveness cancellation; it supplies no UID, namespace,
clock, proc/sys-root or owner-authentication override.

Guardian stdin is the manager's exclusive liveness pipe. EOF detects manager
death or cancellation. Independent nonblocking reads and monotonic checks can
stop a worker blocked in synchronous work. Cleanup signals only pinned pidfds
whose tasks are checked as direct children; it never signals reusable numeric
group IDs. Subreaper adoption brings orphaned grandchildren into this child set,
including descendants that create another session. Nonblocking `wait(NOHANG)`
reaps all children across groups. Cleanup continues until ECHILD confirms none
remain; a kernel task that cannot yet exit keeps admission occupied.

The transport has a fixed five-byte status/length header and at most 256 KiB of
JSON. A successful worker exit, complete JSON body, EOF with no extra bytes,
complete descendant cleanup and successful guardian exit are all required.
Malformed/partial/trailing/oversized output and nonzero exit refuse the whole
request. A valid guardian refusal frame still confirms cleanup; transport failure
does not. `SupervisedReport::as_bytes()` exposes past observations, never retained
storage facts or effect authority. Neither JSON nor frame status authenticates
an arbitrary executable: trusted fixed image/configuration selection is a later
launcher responsibility.

Local process fixtures are enabled only by `read-worker-probe`. The helper binary
lives under tests/support, is disabled by default and is absent from packaging's
explicit executable list. `default-run` preserves the existing executor choice.

```sh
cargo test -p limeos-executor --features read-worker-probe --locked --offline
```

Run that suite as a nonroot user, using disk TMPDIR/build/cache paths. It creates
only owned local processes and marker files. Tests cover blocked-work deadline,
cancellation/drop, contention, manager death, adopted children with changed
sessions, output bounds including a full 256 KiB frame, nonzero exits and poisoned
admission. An isolated observer reaps the parent-death fixture's orphaned guardian;
the ordinary test runner's child policy is unchanged. The bootstrapped genuine
collector passes own-process resource admission and refuses at the public nonroot
configuration guard; no positive host or Docker/storage collection is run.

See the [design](plans/2026-10-13-read-worker-supervision-design.md) and
[local acceptance](rewrite-evidence/p04/2026-10-14-read-worker-supervision/README.md).
The cleanup regression distinguishes `wait(NOHANG)` from `waitpid(None, ...)`,
which selects only the caller's own group and cannot confirm this descendant set.

Installed root/capability/namespace behavior and service syscall compatibility
remain unqualified. Command spawning is synchronous in the monitor: the ticket
deadline can refuse while launch blocks, but terminating blocked exec is not
qualified. Guardian SIGKILL cannot confirm descendant cleanup; future service
cgroup teardown is the final failure guard. No strict scheduler-latency or
unkillable-task termination guarantee is claimed. Peak memory/FD/task headroom
and performance remain open under the unchanged service restrictions. This slice
is outside the frozen signed ARM64 candidate. Claims, locks, effects, recovery,
P04/defect closure and Pi cutover remain separate work.
