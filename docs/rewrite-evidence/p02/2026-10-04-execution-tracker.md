# P02 execution tracker

The operator authorized P02 after P01. The accepted roadmap and architecture govern implementation. The frozen reference is `/home/marc/Documents/github/pi-health`; implementation is in `/home/marc/Documents/github/lime-os`.

Status: implementation and local acceptance completed; reference-host qualification remains open. The operator authorized autonomous continuation on 2026-10-04 while keeping wybie undisturbed.

The inherited tracker described the work without a numeric phase estimate. This is a planning gap, not a retrospectively estimated success. The recovery handoff records a four-hour completion allowance, with a scope review at six hours; it was recorded during recovery rather than at the original phase start. The original implementation effort was not recorded. Validation resumed on the evening of October 4 and finished on October 5; elapsed time includes waiting for sandbox approvals and must not be reported as implementer effort. Further phases record numeric estimates before implementation.

## Delivered

- RW-020: one host collector and one Docker collector per executor, adaptive demand, independent source freshness, bounded probes and output, GET-only Engine negotiation/events/reconciliation, selected metadata, separate bounded telemetry with 31-day retention.
- RW-021: current-grant filtering on overview/resources/history and every SSE snapshot; pagination, interrupted-reader limits, full reconnect snapshots, generated contracts, responsive read-only dashboard with loading, missing/stale/partial states and safe error details.
- RW-022: frozen-source semantic fixtures for disk identity, container state and history aggregation. Earlier history remains explicitly available in the current build's System history view. Live comparison on wybie is pending.
- RW-023: independent signed Debian shadow package, accounts, database, socket paths, origin and loopback port. Its disposable VM installation/removal leaves the existing services, login, reference configuration and mounts unchanged. Installation on wybie is pending.

## Validation

The recovery run passed all 39 Rust tests, formatting, Clippy with warnings denied, generated-contract drift, repository boundary checks, ShellCheck, frontend tests, strict TypeScript and the production asset build. The dependency policy checks passed; the current advisory audit reports no vulnerabilities or warnings in 213 dependencies. All thirteen [deliberate gate rejection probes](gate-probes.json) passed. The release-gate harness now derives the workspace version instead of assuming P01's `0.1.0`.

Local implementation commits are `20ede5d` (observations), `477ca02` (dashboard) and `c733e21` (shadow qualification). These commits are not published.

The repeated [Debian acceptance run](vm-result.json) passed six groups. Combined core/container/host executor PSS was 6,397 KiB (6.25 MiB). Cached HTTP p95 was 0.60 ms in this run. Ninety dashboard reads caused one container inventory request, with one event connection. These are measurements in a disposable x86-64 VM with a synthetic Engine, not Pi qualification or a production latency claim. The inherited package was tested again; this recovery run did not rebuild it. [Artifact hashes](artifact-checksums.json) identify the exact package tested.

The initial native test rerun failed while linking because the workstation filesystem had 11 MiB free. Only generated development and gate-probe build output was cleaned. Retesting used two build jobs, no incremental compilation and no debug symbols. Docker Desktop's API was also unresponsive; no Docker daemon or unrelated services were restarted.

## Remaining gates

The roadmap's full P02 gate is still open: reference-hardware targets, live semantic comparison and the reference-host shadow install/remove cycle require an agreed quiet window. No active testing, installation, service restart or state change was performed on wybie in this recovery run. ARM64 native CI and a fresh Debian package rebuild were not rerun locally. The frontend is the initial native dashboard; existing screens continue to migrate in later slices.

No defect-register row is closed merely because a related component exists. Dedicated screen, modal/navigation and scaling regressions still need their own evidence. Local P03 development is authorized by the overnight continuation request; it does not constitute reference-host P02 signoff or authorize write tests on wybie.

During the P03 review, a source regression was added for 64 slow container stats requests. Stats now stop after eight seconds while successful inventory is retained. [The regression](../../../crates/observations/src/collectors/tests.rs) first reproduced a 32-second delay. The inherited package hashes and VM measurements above precede this fix; a fresh package rebuild remains pending.
