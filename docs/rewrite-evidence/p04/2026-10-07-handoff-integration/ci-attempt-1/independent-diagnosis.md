# Independent diagnosis: installed Debian VM CI failure

Reviewed run https://github.com/Brownster/limeos/actions/runs/37608091754, job 112752942286, source `a7c73459c70b0555aff5a4f366e13c3ea063ae7f`. This was a read-only review of captured evidence and exact-source code. No tests, SSH, Pi commands, repository edits, or Git mutations were performed.

Evidence directory: `/home/marc/Documents/github/lime-os/docs/rewrite-evidence/p04/2026-10-07-handoff-integration/ci-attempt-1/`.

The decompressed `vm.log.gz` is 72,489 bytes with SHA-256 `68c484e29a8abe4c18ec01fef752d4702bec0467e805bdc3f2d914c2da475c8f`, matching the captured summary. The log records checkout of the exact source above. Native AMD64 and ARM64 jobs passed; the Debian VM job passed P01 and P02 before failing in the original P03 restart fixture.

## Finding

The failure is consistent with a deterministic gap in the fixture's recovery readiness condition, and the evidence supplies a concrete timing instance of that gap. It does not demonstrate a runtime recovery failure.

- The guest journal records `limeos-containerd` killed with SIGKILL at 10:53:57 UTC. No subsequent automatic restart is present before collection of the failure diagnostics.
- The fixture prints `PASS kill limeos-containerd during: no unjustified second effect` at 10:53:58.9865212 UTC.
- The next loop iteration's POST `/api/v1/container/restart/plans` fails at 10:53:59.0227456 UTC with HTTP 503, code `unavailable`, and `retry: true`.
- The previous executor crash at 10:53:17 is followed by a scheduled restart and start at 10:53:19, consistent with the installed service's unchanged `Restart=on-failure` and `RestartSec=2s`.

In `tests/privileged_vm/p03_guest.py`, the kill loop (lines 684–697 at this source) sends SIGKILL, then waits for `systemctl is-active` to say active and for the interrupted job's expected state. The active check can see the old unit state while systemd is processing the killed PID. An interrupted job becoming `needs_intervention` does not establish executor readiness: `bins/core/src/operations.rs` deliberately marks that state immediately when the in-flight executor RPC fails. The fixture's later 0.5-second effect-count check also does not wait for the 2-second restart delay.

The next plan must inspect the new resource through the executor. `Core::propose_action` calls `inspect`, and `Core::executor` maps an unavailable Unix socket/failed RPC to `ErrorCode::Unavailable`; the API maps that error to HTTP 503 with retry true. That is the expected response while the executor is down. The failure occurs before plan approval or queuing of the next mutation.

`Type=exec` cannot substitute for application readiness even after an actual restart: executor policy and receipt-store initialization occur before the listener binds. The existing `wait_container_ready()` helper (lines 102–134) probes framed health IPC as the authorized `limeos-core` UID/group. The newer `p03_lifecycle_guest.py` crash loop already waits for the expected interrupted-job state and then calls this helper before proceeding.

## Smallest justified correction and check

Use the existing bounded `wait_container_ready()` after the original restart loop observes the expected interrupted-job state and before it prints PASS or begins the next scenario. The old `is-active` wait can be removed as redundant or retained only as an additional observation; it must not be the readiness gate. Preserve all crash/effect-count/receipt assertions and the existing automatic restart behavior. A blanket HTTP retry, extra sleep, runtime retry policy change, or relaxed crash expectation would obscure this specific defect and is unnecessary.

Add a local deterministic fixture regression in which unit state is already active, the interrupted job is terminal, and authorized IPC remains unavailable until a later readiness probe succeeds. Assert that the next scenario cannot start before that probe succeeds, and that a bounded readiness timeout still fails the fixture. This checks the missing causal gate without a host or real guest.

Rerun the installed Debian VM acceptance at the correction's exact source. Until it passes, the installed VM qualification remains failed. The current artifact ends before automatic recovery of this final crash, so eventual recovery of that crash is not independently proved by this attempt. Earlier successful crash recovery and prior native qualification are supporting context, not replacements for the new exact-source installed VM gate.
