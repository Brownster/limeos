# VM qualification: mount flow under Docker churn, and whole-stack lifecycle

Disposable Debian 12 AMD64 KVM guests (1024 MiB, 2 vCPU, one at a time, host MemAvailable never below
18.6 GiB, so the 8 GiB admission and 3 GiB stop floors never triggered). Source `eebc394` (local main), offline
Debian build (`CARGO_BUILD_JOBS=4`), packages `0.4.15+stacklc.1`
([inputs](raw/package-inputs.json), [build](raw/build-record.json)). `frontend/dist` was copied from the shared
checkout (untracked build output, UI not under test). Nothing was pushed; no other host was touched.

## Part A: mount flow with real Docker

Unchanged `p04_approved_guest.main` (positive mount flow, real packaged dockerd). Seven runs; `mount-2/3` are
**invalid** (my own `/proc` sampler opened descriptors in the scanned process and caused `fds.link_failed`;
it was removed). Valid runs: 1, 4, 5, 6, 7. None reached a successful mount.

| Run | Plan, approval and queue | Refusal (executor journal) |
| --- | --- | --- |
| 1 | pass, job queued | `recheck.window.before` TimedOut (4 s window); `scan.recheck.kernel_tasks`: 7 kernel tasks removed (pids 23, 52, 259, 260...) |
| 4 | pass | `scan.namespace.open` errno 2 inside `scan.recheck.collection` (process vanished) |
| 5 | pass | same as 4 |
| 6 | pass | `recheck.configuration.after` TimedOut (4 s window); trace: executor itself ran 124 `blkid` execs in 1.5 s |
| 7 | **fail, queue POST 409** | `storage_mount_consumer_tasks_changed`, `scan.recheck.user_tasks`, +1 task: dockerd pid 7269 tid 9212 (122 vs 121 tasks) |

Answer: **yes, it still happens**, 1 of 5 valid runs, at `human_plan_approval_queue`, in the single-shot
`scan.recheck.user_tasks` with a new dockerd TID, exactly as on 4 November. `scan.stable.exhausted` never occurred; the
stable initial scan passed every time. All other runs got past queueing and died at the executor's exact
rechecks, each with a different cause. Run 7's trace (`raw/mount-7-output.log.gz`, `TRACE_EVENT`) shows the cause:
the executor's own Engine revalidation (`GET /info`, `process_evidence.rs:596,726`) makes dockerd fork `runc` and
`docker-init` helpers and spawn thread 9212 about 40 ms before the recheck at 21:38:02.533. The same
revalidation is followed immediately by the host-consumer recheck (`bins/executor/src/storage_mount_admission.rs`
`check()`), so a collision is likely. Runs 1, 4-6 add two more sources: kernel workers and `jbd2`/`ext4` threads
appearing and exiting, and the 4 s window being consumed by the executor's own `blkid` fork storm on 2 vCPUs.
Safety rules were not changed and nothing was filtered. The wrapper holds one open `trace_pipe` descriptor and prints
nothing during the flow.

## Part B: whole-stack lifecycle

`tests/privileged_vm/p04_stack_lifecycle_guest.py`: HTTP API with session and CSRF, packaged dockerd, 3-service
Compose-labelled busybox projects (labels only; `docker compose` never runs), ceiling enabled in
`/etc/limeos/system-policy/container.json`, independent `docker events` for every effect. Two full passes
(`stack-6`, `stack-7`, identical outcomes, ~13 min each). `stack-5` had two script bugs, fixed. `stack-1..4` failed
at setup and found a harness constraint (below).

| Case | Result (2 runs) | Observed |
| --- | --- | --- |
| 1 stop, start, restart | pass, pass | each `succeeded`, receipt `verified`, one event per member in ID order, new StartedAt, catalog stack `cat-sl1` != project |
| 2 replay and second key | pass, pass | same job id, no events; second key with consumed approval: 409 |
| 3 drift before queue | pass, pass | 409, no job, no events |
| 3b drift after queue | pass, pass | `precondition_changed`, no effect, locks released |
| 4a Core SIGKILL right after queue | pass, pass | `succeeded`, one restart per member, nothing replayed in the next 35 s |
| 4a' Core SIGKILL mid-executor | pass, pass | recovered by receipt, `succeeded`, one restart per member |
| 4b executor SIGKILL mid-restart | pass, pass | `needs_intervention`; m1 and m2 restarted once (verified), m3 never touched but recorded `outcome_unknown`/`state_not_durable`; stack and all member locks held; no replay in 40 s |
| 5a stack lock vs member job | pass, pass | member job accepted (202) but stays `queued` 20 s, no effect, cancel 204 |
| 5b member lock vs stack job | pass, pass | stack job accepted (202), stays `queued`, no effect, cancel 204 |
| 6 shadow, task token, scoped human | pass, pass | shadow executor exits on `allow_stack`; shadow plan 503; task RPC `propose/queue/approve_stack` `invalid_input`; scoped human 403; anonymous 401; GET 405; unknown field 400/422 |
| 7 member cannot start | pass, pass | m1 verified, m2 `outcome_unknown` (Engine: OCI create failed, container left `created`), m3 `precondition_changed`; job `needs_intervention`, never success; Engine state and events unchanged for 40 s |

Single-container regression (`p03_lifecycle_guest` via `p04_approved_container_guest`, unchanged): 38 PASS
([result](raw/lifecycle-1-result.json)).

Resources (`stack-7`, after all cases): Core PSS 9.8 MiB, executor 3.4 MiB, reader 1.7 MiB, swap 0, guest
MemAvailable 585 MiB of 973 MiB; no OOM. Setup took 106 s, whole script about 13 min.

### Findings (no product code changed)

1. **Stack inventory needs at most 16 Engine containers in total.** With 25 containers (0 running) the Engine source
   stayed `unavailable` and every plan returned 503 (`stack-3`, `stack-4` logs); with at most 16 it is fresh.
   `crates/executor-container/src/deployment/discovery.rs:132` (`MAX_MEMBERS`, `crates/domain/src/deployment/discovery.rs:12`).
   Any host with more than 16 containers cannot plan a stack action. The fixture creates stacks on demand to stay under it.
2. Executor death leaves provably unsent members `outcome_unknown` (4b), so `partly_done` is not reached by either
   failure I could construct (executor death, Engine rejection). No unlock path exists (known).
3. Lock exclusion shows as a queued, never-dispatching job, not a refusal at queue time (5a, 5b).

### Untested

Cancelling a running stack job (409), plan expiry and grant revocation for stack jobs, ceiling flags off
(`allow_stack`, per-action, `managed_stacks`), the `partly_done`/`failed` aggregate, shadow refusal other than the
executor exit and a 503 (the 503 is cohort unavailability, not proof of read-only denial), 8-running-container
reader bound, UI, a real `docker compose up` project, ARM64/Pi.

## Reproduce and cleanup

```
python3 docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount/harness/build_binaries.py   # Docker Desktop on
python3 .../harness/prepare_packages.py                                                      # then stop Docker Desktop
mkdir -p .cache/q/guest-a && cp tests/privileged_vm/*.py .cache/q/guest-a/ && cp .../harness/mount_churn_wrapper.py .cache/q/guest-a/
python3 .../harness/launch_vm.py mount-N .cache/q/guest-a/mount_churn_wrapper.py --storage-disks
python3 .../harness/launch_vm.py stack-N tests/privileged_vm/p04_stack_lifecycle_guest.py
mkdir -p .cache/q/guest-c && cp tests/privileged_vm/*.py .cache/q/guest-c/ && cp .../harness/lifecycle_regression_wrapper.py .cache/q/guest-c/
python3 .../harness/launch_vm.py lifecycle-1 .cache/q/guest-c/lifecycle_regression_wrapper.py
python3 .../harness/collect_evidence.py   # raw/ and SHA256SUMS; verify with: sha256sum -c SHA256SUMS
```

Docker Desktop was inactive; I started it for the build and signing only and stopped it before any VM. Every run's
[record](raw/) shows `qemu_left: []` and `scratch_removed: true`. Final check: no qemu process, scratch directory empty.
