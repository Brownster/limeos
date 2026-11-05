# Engine-wide container bound for stack inventory (finding 1 of the VM stack qualification)

Follows [2026-11-05-vm-stack-and-mount](../2026-11-05-vm-stack-and-mount/README.md) finding 1: the stack inventory
went `unavailable` and every plan returned 503 when the Docker Engine held more than 16 containers in total.
Wybie (18 containers, 14 running, two Compose projects plus standalone) would have had no stack view.

## Bound survey

| Bound | Value | Scope | Decision |
| --- | --- | --- | --- |
| `MAX_STACK_MEMBERS` (stack_operations.rs) | 16 | members of one stack: plan, queue, persistence, executor re-list, receipt | per stack, unchanged |
| `MAX_SERVICES`, `MAX_STACKS` | 16, 8 | services per stack, registered stacks | per stack/catalog, unchanged |
| `discovery::MAX_MEMBERS` (was 16) | 16 | used only as the **whole-Engine** member cap in the cohort wire (`EngineWire`), input admission and `engine_facts` | **bug**: renamed `MAX_ENGINE_CONTAINERS` = 64 |
| `CombinedV1` Engine read: consumers 16, running 8, selected mounts 32, list 64 KiB, retained report 48 KiB, 2 MiB body, 2 MiB live ledger, 2,048 nodes | as listed | `read_admission.rs`, `process_evidence.rs` `collect`, `check_report` | **second hidden Engine-wide cap** (found by a failing 64-container unit test); the storage-process path keeps it, the Compose projection gets `EngineWideV1` |
| `project_members` list rows | 64 | one Compose project's rows, including one-offs | now the named constant (equal value) |
| `CONTAINER_STORAGE_MAX_CONTAINERS` 64, `_MAX_MOUNTS` 64, `_TOTAL_MOUNTS` 256 | | storage inventory | consistent with the new bound; compile-time assert ties them |
| `MAX_INPUT_TEXT` 48 KiB, `MAX_REPORT_BYTES` 64 KiB | | cohort wire and inventory | **kept**: a 64-container cohort with realistic paths serializes to under 32 KiB (unit-tested); pathological text still fails closed |
| Reader unit | 64 MiB, 16 tasks, 256 FDs; worker 20 MiB RSS, 67 FDs (admission samples) | `limeos-storage-reader.service`, `stack_read.rs` | unchanged; measured below |

`EngineWideV1`: 64 consumers and running (this projection scans no processes), 256 selected mounts (64 each),
256 KiB list response (inspect/version/info stay 64 KiB), 4 MiB body allowance, 4 MiB live ledger, 8,192 nodes and
256 KiB decoded strings per response, 256 KiB streamed (never allocated) report ceiling. Excess still refuses the
whole read. The 7x scratch model charges 1.8 MiB for the largest list, inside the 4 MiB ledger.
Wybie fit: 18 containers; "media" at roughly 13-14 services stays within the 16 members/services per stack.

## Results

Source `5a57a9d` (product code from `4532dbc`), same offline build and packages as the previous packet
([build](raw/build-record.json), [inputs](raw/package-inputs.json)). Guest `tests/privileged_vm/p04_stack_scale_guest.py`
reuses the lifecycle guest's setup and helpers. 1024 MiB, 2 vCPU, one VM at a time, host MemAvailable minimum 19.8 GiB,
no guard triggered, `qemu_left: []`, scratch removed, no qemu process afterwards, Docker Desktop stopped.

| Case (2 runs: `scale-1`, `scale-2`) | Result | Observed |
| --- | --- | --- |
| S1 30 containers: 15 running (old reader cap 8), 24 standalone, two registered 3-member projects | pass, pass | inventory `fresh`, `standalone_containers` 24, stack restart planned, approved, queued, `succeeded`, receipt `verified`, exactly one restart event per member, new StartedAt, no locks left |
| S2 exactly 64 containers (12 running) | pass, pass | `fresh`; stack restart plans and runs the same way |
| S3 65 containers | pass, pass | inventory not fresh (`stale`, the earlier sample ages out and no new one is admitted), plan 503, still closed for 15 s, then back to `fresh` after one container is removed |

Stack reader cgroup (`limeos-storage-reader.service`; sampled every 20 ms across each run, about 7,300 samples; `memory.peak`
is the kernel's cgroup peak since the unit started):

| | memory.peak | peak tasks (`pids.peak`) | open FDs, all reader processes / largest one | limit |
| --- | --- | --- | --- | --- |
| scale-1 | 5.41 MiB | 5 | 36 / 18 | 64 MiB, 16 tasks, 256 FDs |
| scale-2 | 5.61 MiB | 5 | 35 / 17 | same |

Before any scale case the peak was already 4.8 MiB (setup, 4 containers), so 64 containers added about 0.6-0.8 MiB.
Core PSS 9.3-9.9 MiB, executor 3.8 MiB, reader 1.9 MiB, swap 0, guest MemAvailable 500 MiB of 973 MiB at the end.

Not shown by this packet: the same guest against the pre-fix binaries (the previous packet already recorded
25 containers => unavailable, `stack-3`, `stack-4`).

## Tests

`cargo test`: domain 577, executor-container 389 plus 19 doctests, core 90, executor 55, observations and api
unchanged; `clippy --workspace --all-targets --locked -D warnings`, `cargo fmt --check`, `check_contracts.py`,
`check_repository.py` pass. New: domain 64/65 (inventory, cohort constructor, wire decode), executor `engine_facts`
64/65, Engine read 64 running with 4 mounts each (revalidated) and 65 / 5 mounts refused, `CombinedV1` still 16/8/32,
`project_members` 64 rows accepted (16 members + one-offs) and 65 refused before any inspect, core test planning,
approving, queueing and running a stack among 40 containers in three projects.

## Untested

Wybie or any real 18-container host (not touched); a real `docker compose up` project; UI; ARM64; the health
projection (`inspect_health`) still uses `CombinedV1` (16/8/32) and has the same limit; worst-case adversarial inspect
bodies (64 x 64 KiB would exceed the 4 MiB body allowance and fail closed); more than 24 s of continuous 64-container
churn; the 65-container refusal was observed as `stale`/503, not as the reader's own journal line.

## Reproduce

```
python3 docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount/harness/build_binaries.py   # Docker Desktop on
python3 docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount/harness/prepare_packages.py # then stop Docker Desktop
python3 docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount/harness/launch_vm.py scale-N tests/privileged_vm/p04_stack_scale_guest.py
sha256sum -c SHA256SUMS
```
