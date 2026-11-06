# VM qualification: operator resolution of needs_intervention jobs

Disposable Debian 12 AMD64 KVM guests (one at a time, `tests/privileged_vm/run.py`, nothing else touched; no push, no
remote host). Source `7c6ef62` (local main, resolution feature `184b8b2..7c6ef62`), packages `0.4.16+resolve.1` built
inside the guest from the bundled source ([hashes](raw/build-sha256.txt)); `frontend/dist` copied from the shared checkout
(UI not under test). Guest script: `tests/privileged_vm/p04_resolution_guest.py` (reuses the fixture and helpers of
`p04_stack_lifecycle_guest.py`; selected with `--guest-script`, which is how every guest script is wired in `run.py`).
Real HTTP API with an enrolled administrator session and CSRF, packaged dockerd, ordinary busybox containers, the real
executor; `docker events` is the independent proof. Final run: [result.json](raw/result.json), all 12 cases pass.

## How a stuck job is induced

- Restart (container or stack): the members ignore SIGTERM so the Engine takes its full 10 s stop grace; the executor is
  SIGKILLed on the `kill` event of the member in flight. The Engine finishes that restart on its own.
- Start: the container's command is a bind-mounted script that is not executable, so the OCI runtime refuses the start;
  the Engine leaves the container `created`, the executor records `outcome_unknown`. `chmod +x` later makes a new start work.

## Cases

| Case | Result | Observed |
| --- | --- | --- |
| 0 induce | pass | five stuck jobs: container restart (H), stack restart mixed (D3), stack restart fully done (D4), container start (N1), stack start (N2); all `needs_intervention`, core `resource_locks` and executor `actions`/`stack_actions` rows held |
| 1 locks held | pass | for H, D3, N1, N2 a new plan on the same target is accepted (201) and queued (202) but stays `queued` for 15 s with no Engine event, then cancels (204); original locks and state unchanged |
| 2 evidence GET | pass | per target, `expected` equals the Engine state at plan time and `measured` equals `docker inspect` now (running, StartedAt, image); `matches_expected`/`intended_state_reached`/`allowed` equal an independent computation; digest is 64 hex and stable across reads. H: verified_done+divergence; D3: divergence only; N1, N2: not_done+divergence |
| 2b D4 | pass | **Finding:** a stack job whose end state is fully reached (all members restarted) does not stay stuck: the existing receipt recovery moves it to `succeeded` within about a minute, and resolution then reports 409 `conflict`. So `verified_done` on a stack cannot be induced by this route; it is covered for a container (H, H2) |
| 3 refusals | pass | verified_done on not-reached: 409 `unsupported_declaration` with blocking IDs (N1, D3 member 3); not_done on changed: 409 with blocking (H, D3 members 1-2); bogus and real stale digest 409 `stale_evidence` with fresh view; wrong password 401 and the session still works; `Authorization` and `X-API-Key` 403 (GET and POST); operator-role session 403; anonymous 401; bad CSRF 403; short divergence reason 400. No job, lock, record or Engine event changed |
| 4a | pass | H `verified_done` by API: 200 `resolved`, job `resolved`, `resource_locks` empty, `actions` row `resolved`, audit `job_resolution_requested`/`job_resolved`, no unique-index row held, **new restart plan succeeded end to end** |
| 4b | pass | D3 `accepted_divergence`: same checks, `stack_actions` row `resolved`, new stack restart succeeded |
| 4c | pass | N1 `not_done`: resolved; after fixing the cause a new start succeeded |
| 4d | pass | N2 (stack) `not_done`: `stack_actions` `resolved`; new stack start succeeded |
| 5 docker events | pass | every resolution, refusal, CLI call and crash retry asserted an empty window of `start/stop/restart/kill/die/create/destroy/pause/unpause/oom` events for all containers |
| 6a CLI | pass | non-root refused; unknown admin and non-administrator refused; unsupported declaration and stale digest exit 1 and change nothing; `job resolve --outcome verified-done` (H2) exit 0, `show-resolution` digest equals the API's, reason suffixed ` [limeosctl, root peer]`; new restart succeeded |
| 6b CLI | pass | `job resolve --outcome not-done` with the reason on stdin (N3): resolved, new start succeeded |
| 7a crash, request recorded | pass | dockerd stopped as soon as the `job_resolutions` row appeared, executor and core SIGKILLed: request recorded, core job `needs_intervention` with locks, executor row still `outcome_unknown`; after restart the startup retry completed it (job `resolved`, locks gone, executor `resolved`) |
| 7b crash, executor freed | pass | core SIGSTOPped after it sent `ResolveAction` while dockerd was held, then SIGKILLed: executor row `resolved`, core `needs_intervention` with locks and an unconfirmed request; after restart the retry completed it |

## Bugs

No product bug found; no product code changed. Harness fixes: `build_bundle.py` omitted `docs/p04-storage-mounts-profile.md`
that `packaging/build.py` copies (the in-guest build failed after the compile; added).

My first full run had five failing cases, all in my script, not the product: it assumed a stack job whose end state is
reached stays stuck (see 2b), a hand-stopped member is not restarted by a stack restart (a stopped member got no event),
and the crash helper signalled pid 0 (own process group) because the dockerd unit name was wrong. See
[first-run-result.json.gz](raw/first-run-result.json.gz).

## Not covered

Storage/backup jobs (out of scope); `verified_done` for a stack (self-recovers, 2b); 429 on password rate limit; UI;
ARM64/Pi.

## Reproduce

In-guest build and run (about 35 minutes the first time; the guest compiles the Rust workspace):

```
python3 tests/privileged_vm/build_bundle.py --output BUNDLE.tar.gz     # needs frontend/dist
python3 tests/privileged_vm/run.py --image .../debian-12-genericcloud-amd64.qcow2 --checksums .../SHA512SUMS \
  --repository EMPTYDIR --build-bundle BUNDLE.tar.gz --build-output BUILD --package-version 0.4.16+resolve.1 \
  --guest-script tests/privileged_vm/p04_resolution_guest.py --output result.json
# later runs reuse the packages: --repository BUILD/limeos-repo (no --build-bundle)
```

Cleanup: after the final run no `qemu-system` process exists and the `/tmp/limeos-p01-vm-*` directory is gone.
