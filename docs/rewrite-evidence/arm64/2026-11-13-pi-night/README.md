# ARM64 Pi night, 13 Nov 2026 (wybie)

Native KVM qualification of `limeos 0.4.17+arm64.1` (source
`3dc51cabc2b3b3de0704369a3745e72a145e820f`, candidate
`.cache/arm64-qual/2026-11-07-cross/candidate`) on the reference Raspberry Pi 5,
inside the operator's 01:00-09:00Z window ([authorization](authorization.json)).
The container suite passed on 11 Nov with the same candidate; this night ran
`install`, `stack` and `scale`.

## Result

All tests in all three suites passed. The orchestrator exited 4 only because the
install guest's local state could not be archived after a clean stop (see below).

| Suite | Window (UTC) | Result |
|---|---|---|
| install | 01:02-01:24 | 20/20 PASS ([output](run/night-install/suite-output.txt)) |
| stack | 01:24-01:37 | 11/11 PASS ([output](EVIDENCE-BUNDLE.json "bundled: run/night-stack/suite-output.txt")) |
| scale | 01:37-01:42 | 3/3 PASS ([output](run/night-scale/suite-output.txt)) |

Install covers package identity, sandboxing, Docker-socket isolation, enrollment,
password-hash import, restart and cold-start readiness, idle footprint, idle and
streaming CPU and writes, and read latency. Stack covers whole-stack actions,
replay, drift refusal, crash recovery, exclusion, privilege refusal and partial
failure. Scale covers 30 and 64 containers and the fail-closed refusal at 65.

## Host

- Swap was reset once just before the run, as authorized (409 MiB to 0). It
  rose to about 270 MiB while the install guest booted, 305 MiB during stack,
  and 435 MiB after the run (its usual 410-445 MiB). The 503 MiB watchdog never
  fired; no guard failed ([guards](EVIDENCE-BUNDLE.json "bundled: run/guards.jsonl"), [watchdog](EVIDENCE-BUNDLE.json "bundled: run/watchdog.jsonl")).
- The 14 running containers kept their IDs and StartedAt
  ([before](containers-before.txt), [after](containers-after.txt)); the four
  already-stopped containers were unchanged.
- After the run: no qemu process, no watchdog, `~/limeos-arm64-qual/night`
  removed. The QEMU packages were installed before the run and are still
  installed; the harness added none.

## Install-guest archive failure

`native_vm.py stop` stopped the guest and removed its host state, then failed to
rename the local directory to `.cache/arm64-qual/stopped/night-install` because
an archive from 8 Nov already had that name (`OSError: [Errno 39] Directory not
empty`). The orchestrator recorded the problem and exit 4. Nothing was left on
the host. `stop` now picks a timestamped archive name when one exists.
