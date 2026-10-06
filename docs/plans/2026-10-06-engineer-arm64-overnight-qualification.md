# Engineer 1: overnight native ARM64 qualification

Close the installed native ARM64 and footprint gap using the prepared harness. The integrator continues RW-040 locally; engineer 2 owns archive replay and private staging. You alone use the Pi during this window.

Start `engineer/arm64-overnight-qualification` in a separate worktree from `a595c2f1bcca8a8a0bc21ea0162eea841d0d5d15`. Freeze the tested runtime at `8acac403e32d3692b028a036a11f4fead2050580`; its 231 Rust tests and nine harness tests pass on native AMD64/ARM64 CI, and its AMD64 packages pass 180 guest acceptance groups. Do not chase the integrator's later runtime changes.

Estimate: 24 engineering hours, with host/build/wait time recorded separately. Review remaining scope at 36 hours. The overnight deadline takes precedence over finishing the checklist. This remains within P04's 320-hour estimate and 480-hour cutover-scope review threshold.

## Authorized host and hard stop

The operator explicitly authorized **`holly@wybie` overnight on 2026-10-06, stopped by 07:00 on 2026-10-07, Europe/London**. The hard cutoff is **`2026-10-07T06:00:00Z`**. This supersedes the earlier no-testing instruction only for this named host and window; it is not permanent permission to test on wybie.

Keep Python code, configuration, databases, services, Docker workloads, mounts and account/group membership unchanged. Install LimeOS and run disk writes, service interruptions and package removal only inside disposable Debian ARM64 KVM guests. No host-directory passthrough or physical block-device passthrough. Inspect host prerequisites read-only; report missing tools instead of changing production host packages or services.

Before the first host command, extend the workstation runner with an explicit, recorded authorization window. Its default must still reject wybie. Require the exact authorized target and UTC cutoff; reject absent, expired, malformed or mismatched authorization before SSH. Do not bypass the guard through another alias/IP or remove it. Commit and test this harness-only change separately; record the orchestrator revision independently of frozen runtime and guest-fixture identities.

Enforce shutdown on the host so workstation/SSH loss cannot leave a guest running overnight. Set automatic guest termination for **05:55 UTC**, with bounded forced termination before **06:00 UTC**. Bind the supervisor to this run's exact QEMU process/work directory; it must never kill an unrelated process. Wrapping QEMU's daemonizing parent in `timeout` does not supervise the detached guest. Prove the deadline behavior with a short disposable run and retain the result.

Begin collecting results and orderly teardown by 05:45 UTC. Complete remote work before the hard cutoff. Stop early and retain incomplete results if the remaining window is insufficient. No automatic retry or new guest after expiry. Local regressions must prove default denial, valid named-window admission, expiry/mismatch denial, bounded shutdown and preservation of unrelated processes/state.

## Frozen inputs

Read the [harness instructions](../../tests/qualification/arm64/README.md), [original assignment](2026-10-06-engineer-current-arm64-qualification.md), [integrated evidence](../rewrite-evidence/p04/2026-10-06-handoff-integration/README.md), [approved-target boundary](../adr/0010-p04-approved-storage-targets.md) and [architecture budgets](2026-10-04-rust-rewrite-architecture.md).

- Prepared source: `.cache/arm64-qual/2026-10-06-integration/source.tar.gz` in the integrating checkout, 1,688,191 bytes, SHA-256 `669675abab7b60c0a645ef70531dcd4890a144a681ee2b35c802af870069c594`.
- Runtime and bundled guest fixtures: `8acac403e32d3692b028a036a11f4fead2050580`; authority schema 8; qualification package version `0.4.7+arm64.1`. The source/fixture/frontend hashes are in the corresponding source manifest.
- Genuine schema 6 candidate: [0.4.2 provenance](../rewrite-evidence/arm64/2026-10-06-current-60e6309-preparation/previous-artifact.json), original source `5848bce`, package SHA-256 `0a8651aa73afc7a2e67fe665f4c43d3a9f832e09c3104c316f105bea651c6e51`.
- Genuine schema 7 candidate: [0.4.3 provenance](../rewrite-evidence/p04/2026-10-06-handoff-integration/previous-schema7-artifact.json), original source `f39396b`, package SHA-256 `31e8eb3bc92afb713ae7669a16661e9073a6834453265dea4548c4813f4b001b`.

Use the existing bundle if available and verify its digest. If regenerating it, use the same pinned commits and record the new bundle digest; do not claim byte identity with the old tarball. Harness/guest-fixture changes need their own committed identity and new manifest, without changing the runtime source.

CI's native 0.4.4/0.4.4+ci.1 labels both contain current-source binaries and are not genuine historical upgrades. Do not relabel them as 0.4.7 or fabricate a successful native guest-build transcript.

## Work order and acceptance

1. Record host architecture, KVM, image SHA-512, free RAM/swap, active workloads, existing QEMU processes and available disk space. Verify the supervisor and deadline first. Run one guest at a time, at nice 19/idle I/O priority. Build ceilings: three CPUs, 2048 MiB; test ceilings: two CPUs, 1536 MiB. Use less if host pressure requires it; ceilings do not prove spare capacity. The previous 2.5 GiB guest caused host swap pressure.
2. Build the frozen source in a native Debian 12 guest using Rust 1.88, locked dependencies, fmt, strict Clippy, workspace tests, contract/boundary checks and standard/shadow packages. Preserve signed apt installation and source/binary/package hashes. Verify every installed binary against that build before measuring. Report actual test counts.
3. Run fresh standard/shadow install and the footprint workload through `install_guest.py`. Prove root-owned executable paths, capabilities, dormant optional units, both password formats, durable sessions and shadow write refusal. Capture per-service/total PSS/RSS/swap, ten-minute idle CPU, readiness, short-lived CLI/tasks, authenticated latency, installed bytes and writes over a stated interval. Separate password workers and optional storage services. Run the inspector's synthetic cases natively if time permits and label standalone RSS separately from service PSS.
4. Run storage/container acceptance in separate guarded guests using the bundled fixtures. Empty storage disks must pass serial/size/boot/mount guards before any formatting. Cover interrupted jobs, retained claims, receipt reconciliation and real standard/shadow replacement/removal. Test failures are results to retain, not assertions to relax.
5. Run genuine schema 7 → 8 and schema 6 → 8 upgrades in separate fresh guests, in that priority order. Bind each old package/core hash and original source. Verify sessions, pending approvals, receipts, applicable claims and replay. If the deadline prevents either run, mark it untested.
6. Capture optional storage reader/target service and password-worker accounting separately; retain raw measurements and budget comparisons. Record native KVM versus bare metal honestly. A guest Rust measurement and bare-metal Python data are not a controlled comparison; leave that comparison open unless measured under a matched workload without modifying Python or disturbing host services.
7. Copy results/logs/journals and host-pressure samples, stop every owned guest and supervisor before the cutoff, and record teardown. Preserve unsuccessful and incomplete attempts as well as successful ones.

Own `tests/qualification/arm64/` and a new evidence directory such as `docs/rewrite-evidence/arm64/2026-10-07-current-8acac40-overnight/`. Small necessary shared-fixture changes belong in separate commits with their original assertions intact. Keep main, core/runtime APIs, migrations, executor policies, storage implementation, frontend and dependency pins unchanged. Return runtime defects as isolated reproductions for the integrator.

Return a reviewable branch, exact input/orchestrator identities, native build/package proof, successful/failed test transcripts, measurements, deadline/teardown evidence and an explicit untested list. Completing source tests alone does not close installed native or performance gates.
