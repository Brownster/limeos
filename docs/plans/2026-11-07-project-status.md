# Rewrite status, 7 November 2026

Scope agreed with the operator: **basic cutover first** (host metrics, container
status/logs/approved start-stop-restart, stack status and whole-stack
start/stop/restart). Storage, backup and the assistant are deferred.
Everything below is on local `main`; nothing is pushed.

## Done and evidenced

| Area | State | Evidence |
| --- | --- | --- |
| Whole-stack start/stop/restart | built, qualified in KVM VM | `docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount/` |
| Engine-wide 64-container bound | built, qualified | `.../2026-11-05-engine-wide-container-bound/` |
| `needs_intervention` resolution (core, API, CLI, UI) | built, VM and real-browser qualified | `.../2026-11-06-resolution-vm/`, `.../2026-11-07-resolution-browser-vm/` |
| Admin view of every user's stuck jobs | built, unit tested | `docs/p06-job-resolution.md` |
| Migration tools: scan, import-users, propose-ceiling, propose-catalog | built, VM rehearsed | `docs/p06-migrate-tools.md` |
| Cutover scripts (preflight, activate with rollback, return) | built, VM rehearsed incl. failed activation and return | `docs/rewrite-evidence/p06/2026-11-06-cutover-rehearsal-vm/`, `.../2026-11-06-cutover-catalog-vm/` |
| Flaky fd-count tests | fixed | commit e16c962 |
| Workspace gates (fmt, clippy, tests, frontend, cutover stubs) | green on main | 1933 Rust, 285 frontend, 45 cutover |

## Not done

- **Native ARM64 qualification on the Pi.** Two night windows were lost to
  runner startup bugs (container guard, ssh watchdog launch), both fixed with
  regression tests. A local end-to-end rehearsal of the runner and a live,
  attended run are planned for the night of 8 November.
- **Storage mount flow.** Refuses 0 of 5 mounts in the VM: the census is now
  stable, but one admission takes 3.0 to 4.6 s against a 4 s window. Branch
  `worktree-agent-afd41b24850233330` (not merged). Needs the operator's decision
  on the plan-digest semantics and on widening the window.
- **Cutover itself.** Needs the operator's choices: TLS mode, public port,
  Caddy install, the 14 managed containers, and an explicit go.
- **Health read** stays capped at 16 containers (alert automation, out of scope).
- Jobs left `running` by a restart are flipped to `needs_intervention` at store
  open, so they appear in the banner; no separate state.
- Backup/restore, schedules, assistant (P05): deferred by agreement.
- Colour contrast and screen-reader audit of the resolution dialog.

## Known limits and caveats

- Cutover rehearsals ran on x86_64 with 4 KiB pages, Docker 20.10 and 7
  containers, not wybie's 16 KiB pages and 18 containers.
- The ARM64 candidate is cross-built: no native test totals.
- Packages in the rehearsals are signed with throwaway keys.
