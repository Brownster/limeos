# Two guarded failures identify separate next checks

Both disposable runs used source `f6f583e69518a732bcb8a34b2d09c7f4e3b7e23d`,
schema 12 and packages `0.4.11+fdcontext.1`. The offline build, UI and signed
repository checks passed. All 687 frozen inputs, installed binary hashes and
the same 31 guest modules join the source. Neither run supplies A1 acceptance.

The 384-MiB guest created the genuine positive plan, human approval and queued
job. It then stopped at `await_terminal_receipt`: Core retained
`needs_intervention`, and Root logged `recheck.configuration.before TimedOut`
followed by `revalidate.target_and_host_consumers.after Unavailable`. No
FD-context event fired. The packet does not contain the actual Root receipt or
an accepted mount verification, so it cannot establish the helper's exact
effect state. That failed flow was not retried.

The next guest used the exact committed canonical 1024-MiB runner. Product,
packages, guest modules, service ceilings and original deadlines were unchanged.
It stopped earlier, during `pre_effect_api_refusals`, when the genuine plan
request returned HTTP 409. Root's exact task comparison recorded one addition:
PID 7043, TID 7420, start 23684, namespace `(4, 4026531841)`. The peer identity
and independent before/after task witness place it in the real test Docker
daemon, `/system.slice/limeos-a1-real-docker.service`. This run created no
positive mount job and did not reach the earlier timeout. **There is no causal
conclusion about RAM or the timeout.** The unknown owners of earlier FD failures
also remain unknown.

The first guard completed 14:49:52–14:54:05 UTC, minimum host availability
5,723,648,000 bytes. The second completed 15:20:50–15:24:50 UTC, minimum
5,431,795,712 bytes. Both ended with exit 1, unchanged inputs, no sampled
3-GiB memory stop and removed VM scratch. The detached guards were observed
absent after their original terminal records. The first packet's hashes were
preserved before the second launch and checked again afterward.

The fixture starts a new packaged Docker daemon shortly before planning. Its
listener, identity, `info` and `ps` checks do not establish startup settling.
Investigate a bounded read-only setup warmup and natural quiet observation
before the first genuine plan; this remains a proposed fixture correction.
Every production task/FD comparison and original lifetime stays required.

`source-match.json` records the result. Compressed raw build, guard, package,
fixture and failure records retain both attempts separately. The reproduction
archive includes exact guest, runner, harness and relevant Rust sources.
`SHA256SUMS` covers every other file. Private keys, packages, binaries and VM
disks are excluded. API/UI mount success, restart/replay, upgrades and native
ARM64 qualification remain open. Holly's Pi was untouched; nothing was pushed.
