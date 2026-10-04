# LimeOS Rust rewrite: defect register

Status: acceptance register; all 50 requirements open. Review continues in P00 (RW-003). Prepared: 2026-10-04.
Companion documents: [delivery roadmap](2026-10-04-rust-rewrite-roadmap.md), [target architecture](2026-10-04-rust-rewrite-architecture.md), and [assistant/model-routing design](2026-10-04-assistant-model-routing.md).

The current build is frozen, so its known defects are not fixed there. This register carries each one into the new build as a design rule and a regression scenario. A row is closed when its scenario passes in the phase named, and P06 cannot complete with an open row.

Each scenario is written against the behavior that caused the defect, so it would fail against the current build and must pass against the new one. Status starts `Open`. To mark a row `Closed`, link the executable test and a passing evidence record containing the command, date, tested build, and result. A written scenario or an experiment test alone does not close a production requirement.

Sources:

- **Review:** [June 2026 repository review](https://github.com/Brownster/pi-health/blob/80593b2/Docs/colleagues_review_of_limeOS.txt) (SEC, CAT, STK, MFS, SRA, DSK, SYS, API, UI, CFG, ARCH, FEATURE).
- **Rollout:** [July 2026 rollout and recovery tickets](../rewrite-evidence/current-build/2026-07-27-rollout-recovery-tickets.md) (PKG, INST, MNT, HOST, PLG, CI, DEP, HYG, TEST). Copied here because the original is uncommitted in the pi-health working tree.
- **Baseline:** [reference Pi 5 baseline](../rewrite-evidence/p00/2026-10-04-reference-pi5-baseline.md) (LIVE).
- **Plan review:** the 2026-10-04 review of this plan (BKP, PLG-003, RT, MIG).

## Privilege, installation, and packaging

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| SEC-001 | Falls back to `admin`/`pihealth` when no credentials are configured, on all interfaces. | No default credential exists. The first administrator enrolls through a one-time local bootstrap. | A fresh install with no credential input cannot be logged into; the bootstrap token works once and expires. | P01 | Open | Pending |
| SEC-002 | The root helper accepts complete file content for unit files and startup scripts. | Executors accept typed parameters only and render every file from templates they own. | The executor protocol schema has no free-form content field; a request carrying unit text is rejected. | P01, P04 | Open | Pending |
| LIVE-001 | Root runs `/home/holly/pi-health/pihealth_helper.py`, owned and writable by the dashboard account. | Root executes only root-owned files installed by the package. | A packaging test walks every root unit's executable and library paths and fails on any component an unprivileged account can write. | P01 | Open | Pending |
| LIVE-002 | The dashboard account is in the `docker` group. | Only the container executor's account can open the Docker socket. | Core, assistant, and runner accounts get `EACCES` on the Docker socket. | P01, P03 | Open | Pending |
| INST-001 | A root SSH install makes root the dashboard service account. | Packages create dedicated system users. No service runs as root except the host executor, and none as a human login account. | Installing from a root shell and from a sudo user produces the same service accounts. | P01 | Open | Pending |
| INST-002 | The helper symlink into the checkout aborts on a dangling link and silently runs an old helper on a stale one. | No symlinks into a checkout; every executable comes from the package at a fixed root-owned path. | After upgrade and downgrade, every unit runs binaries from the installed package version. | P01 | Open | Pending |
| INST-003 | The installer keeps any non-empty credentials file without validating it. | Maintainer scripts validate preserved configuration and stop with a named repair step, never overwriting it. | Reinstalling over malformed configuration exits non-zero and leaves the file byte-for-byte unchanged. | P01 | Open | Pending |
| HOST-001 | Host prerequisite checks and repairs are duplicated across installer, preflight, helper, and app with different rules; the installer writes `cmdline.txt` non-atomically. | One detection rule and one repair per prerequisite, owned by the host executor and called by maintainer scripts. Boot-file writes are atomic. | An empty `SystemMaxUse=` counts as uncapped; a drop-in's last assignment wins; an interrupted `cmdline.txt` write leaves the old or new file, never a partial one. | P04 | Open | Pending |
| CFG-001 | Runtime state lives in the source checkout; the reference host's checkout has edited configuration files. | Configuration in `/etc/limeos`, state in `/var/lib/limeos`, logs in journald or `/var/log/limeos`; the package tree is read-only. | `dpkg --verify` reports no changed package files after a week of normal use. | P01 | Open | Pending |
| LIVE-004 | The production web server is Werkzeug's development server. | HTTP is served by Axum with body, header, connection, and time limits. | Oversized headers and bodies and slow clients are rejected within the configured limits. | P01 | Open | Pending |
| PKG-001 | CI publishes a Docker image of LimeOS that never worked and cannot provide the helper boundary. | The Debian package is the only supported distribution; no LimeOS container image is published. | CI has no image-publish job. | P01 | Open | Pending |
| DEP-001 | Python dependencies are unbounded and there is no lockfile. | `Cargo.lock` is committed and builds use `--locked`; `cargo deny` checks sources, licenses, and advisories. | CI fails on an unlocked build or a new advisory. | P01 | Open | Pending |

## Storage and boot

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| MNT-001 | The mount-wait script tests media directories, which are never mount points, and loops forever with no start timeout. | Mount waits use the storage contract's device mount points and a bounded `TimeoutStartSec`. | Each storage profile generates waits only on real mount points; an unsatisfiable wait fails within the bound instead of blocking boot. | P04 | Open | Pending |
| MNT-002 | Startup-service activation discards helper results and reports success. | Every step's result is checked; a failed activation reports the failing step and disables the unit it wrote. | `daemon-reload` failure, enable failure, and executor timeout each surface as errors. | P04 | Open | Pending |
| DSK-001 | Unmount does not check dependent containers, shares, pools, or SnapRAID paths. | Unmount plans list dependents and stop or block them; designated media mounts need an explicit force operation. | Unmounting a disk a running container uses is refused or stops the container first, then verifies the mount state. | P04 | Open | Pending |
| DSK-002 | fstab defaults have no device timeout and accept free-form options. | Filesystem-specific presets with a bounded `x-systemd.device-timeout`; no free-form options. | Booting with a data disk missing completes within the bound. | P04 | Open | Pending |
| RT-001 | The boot unit starts Compose once with no `ExecStop`, so runtime mount loss leaves stacks writing to the bare directory. | Runtime mount loss stops the stacks that depend on the mount, through a mount-health supervisor or per-stack units with `ExecStop`. | Removing a data disk at runtime stops its stacks and raises an alert. | P04 | Open | Pending |
| SRA-001 | SnapRAID sync proceeds when the diff check fails, with no mounted-source preflight. | Sync verifies every data, content, and parity path is a mounted source with the expected identity and fails closed if the diff cannot run. Overrides are separate audited operations. | Sync with an unmounted disk, a swapped disk, or a failing diff is refused. | P04 | Open | Pending |
| SRA-002 | SnapRAID can be configured on a MergerFS pool path. | Cross-capability validation rejects any SnapRAID path at or below a pool mount point. | Configuring a pool path is rejected. | P04 | Open | Pending |
| MFS-001 | MergerFS mount, unmount, and balance always report success. | Every host operation returns its real exit status and verified state. | Non-zero exit, timeout, and missing binary each report failure. | P04 | Open | Pending |
| BKP-001 | Backup restore extracts an archive over `/` as root. | Staged restore: inspect in staging, allow only managed paths, enforce link, size, and entry limits, keep a recovery snapshot, and verify. | Archives with `..`, absolute paths, symlink escapes, device files, or decompression bombs are rejected before anything is applied. | P04 | Open | Pending |

## Stacks and catalog

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| CAT-001 | Removing one catalog app stops the whole stack. | Removal targets the single service; the Compose edit aborts if the stop fails. | Removing one of two running services leaves the other running. | P04 | Open | Pending |
| CAT-002 | Catalog install holds a request worker for up to five minutes. | Install is a configuration change plus a durable job; HTTP returns `202` at once. | A slow `compose up` never holds an HTTP worker; reconnecting to the stream does not relaunch the job. | P03, P04 | Open | Pending |
| CAT-003 | Catalog merge drops top-level `configs`, `secrets`, and other Compose sections. | The template schema defines allowed top-level sections and collision behavior. | Templates with `configs` and `secrets` deploy with them intact. | P04 | Open | Pending |
| STK-001 | Stack deletion ignores `compose down` failure and deletes the directory. | Deletion requires a successful shutdown; force deletion is a separate approved operation. | A failing `down` leaves the directory in place and reports the failure. | P04 | Open | Pending |
| STK-002 | Stack and `.env` writes are unlocked and non-atomic and race with Docker operations. | A per-stack lock covers edits, backups, replacement, and conflicting Docker operations; writes are atomic. | A concurrent edit and restart of one stack serialize; no partial file is ever visible. | P03, P04 | Open | Pending |
| STK-003 | Catalog rewrites destroy comments and formatting in operator YAML. | LimeOS writes managed override files and never re-serializes operator-authored Compose files. | An operator's comments and anchors survive install and removal. | P04 | Open | Pending |
| STK-004 | `compose up` leaves removed services as orphans. | Orphan handling is decided per stack and shown in the plan diff. | Removing a service from a managed stack removes its container on the next apply. | P04 | Open | Pending |
| STK-005 | Duplicate Compose filenames are silently resolved by priority. | Multiple candidates block edits and actions until resolved. | A stack with both `compose.yaml` and `docker-compose.yml` reports the conflict. | P04 | Open | Pending |
| STK-006 | Status polling spawns one `compose ps` per stack per poll. | One observation subsystem, using Docker events and reconciliation, serves every reader. | Docker calls per minute stay constant as stacks and open dashboards increase. | P02 | Open | Pending |
| API-001 | Stack start, stop, pull, and restart run from GET endpoints. | GET never starts an operation; mutations are POST with CSRF validation; streams read a job resource. | A route-table test fails if any GET handler dispatches an operation. | P01, P03 | Open | Pending |

## Persistence and extensions

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| PLG-002 | A corrupt plugin configuration is silently replaced with defaults, and every read rewrites the file non-atomically. | Reads distinguish missing, corrupt, and valid; corrupt files are quarantined and reported; reads never write; writes are atomic. | An invalid file is preserved and reported; a status read performs no write. | P01 | Open | Pending |
| PLG-001 | Plugin import failures are swallowed, and a failed `plugin_manager` import enables every plugin. | Capability loading fails closed and reports a diagnostic per capability. | A broken capability appears with its reason; a registry failure enables nothing. | P04 | Open | Pending |
| PLG-003 | The plugin manager executes third-party Python inside the application. | Built-in capabilities are compiled in; third-party manifests are data; executable extensions run out of process with explicit access. | A manifest cannot cause code to load into core. | P04 | Open | Pending |
| LIVE-003 | The supervised repair runner writes 189 MB a day to a 135 KB database. | Periodic work commits state transitions only. | A 24-hour idle run of all resident processes writes under the architecture's disk-write target. | P01, P04 | Open | Pending |

## Observations and API

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| SYS-001 | A missing secondary disk makes `/api/stats` return 500. | Each optional metric source fails independently and is reported as missing. | Removing a secondary disk leaves the overview working, with that metric marked unavailable. | P02 | Open | Pending |
| UI-002 | The API client discards server error details. | Errors carry a stable code, safe message, retry class, and audit ID, and the UI shows them. | A validation error's message reaches the screen. | P02 | Open | Pending |
| UI-003 | Mount and share pages present failed plugin requests as complete data. | Partial results say what failed; fan-out is concurrent and bounded. | One failing source shows a partial-data warning. | P02, P04 | Open | Pending |

## User interface

The React code is copied from the current build, so these defects arrive with it. Each is fixed when its screen moves to `/api/v1`.

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| UI-001 | Modal focus resets on every keystroke when a page passes an inline close callback. | The modal keeps the latest close handler in a ref. | Typing several characters in a modal field keeps focus there. | P02 | Open | Pending |
| UI-004 | The mobile navigation drawer does not move, trap, or restore focus, and there is no skip link. | The drawer is a proper modal with an inert background; the shell has a skip link to main content. | A keyboard-only test opens, uses, and closes the drawer. | P02 | Open | Pending |
| UI-005 | Service links hard-code `http://`. | The link scheme comes from service metadata. | A service registered with HTTPS opens over HTTPS. | P04 | Open | Pending |
| UI-006 | The catalog UI cannot target or disambiguate stacks. | Installations are `(app, stack)` pairs; install and remove name the stack. | An app installed in two stacks can be removed from one. | P04 | Open | Pending |

## Process and tests

| ID | Defect in the current build | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| CI-001 | CI skips lint, ShellCheck, and unit-file validation. | CI gates `cargo fmt`, Clippy, `cargo deny`, `cargo audit`, ShellCheck on maintainer scripts, `systemd-analyze verify` on units, and the frontend checks. | A deliberate violation of each check fails CI. | P01 | Open | Pending |
| TEST-001 | Installer and activation failure paths have no tests. | Package install, rerun, upgrade, downgrade, removal, and activation run under failure injection in disposable VMs. | Every install-path row in this register has a test that runs in that harness. | P01, P06 | Open | Pending |
| HYG-001 | Generated files are tracked; there are two docs roots; the companion app has a placeholder package name. | One `docs/` root; build outputs ignored and untracked; the companion app's future is decided in P00. | CI fails if build outputs are tracked. | P01 | Open | Pending |
| ARCH-001 | Very large modules hide ownership: `app.py`, the 6,000-line helper, 800-line pages. | Crate and module boundaries follow the architecture's dependency rules; screens are split by domain as they move. | A dependency-direction check runs in CI. | P01 | Open | Pending |

## Migration

| ID | Fact about existing installations | Rule in the new build | Regression scenario | Phase | Status | Passing evidence |
|---|---|---|---|---|---|---|
| MIG-001 | Users have Werkzeug scrypt or PBKDF2-SHA256 hashes. | Core verifies both and upgrades to Argon2id after a successful login. | Hashes generated by the current build verify; wrong passwords fail; the stored hash is upgraded after login. | P01, P06 | Open | Pending |
| MIG-002 | The current build has no roles. | Imported users become administrators; household roles are new. | Every imported user keeps full access. | P06 | Open | Pending |
| MIG-003 | The service account is a human login account in the `docker` group that owns stacks, configuration, and credentials; some installations run as root. | Migration moves ownership to the new service users and asks before changing the login account's groups. | After cutover on the reference host and on a root-installed test host, no LimeOS service runs as root (other than the host executor) or as a login account. | P06 | Open | Pending |
| MIG-004 | State spans five SQLite databases, about 20 JSON files, and configuration edited inside the checkout. | Migration reads every source, reports unsupported settings, and never edits the originals. | The migration report lists every source file; returning to the current build finds its state unchanged. | P06 | Open | Pending |

## Rejected claims

These were examined and rejected in earlier reviews. Do not reintroduce them as fixes.

- A default `.unionfs/` SnapRAID exclude: mergerfs does not create that directory on branches.
- `chattr +i` on mount-point directories as the fix for runtime disk loss: at most optional defense in depth after RT-001.
- `BindsTo=` on a oneshot startup unit: it does not stop detached containers; see RT-001.
- Failing the journal-cap repair when the journald restart or vacuum fails: the cap takes effect at journald's next start.

## Enhancements, not defects

| ID | Suggestion | Disposition |
|---|---|---|
| FEATURE-001 | NFS and CIFS remote mounts. | Not parity; candidate after cutover. |
| FEATURE-002 | Per-mount journal diagnostics. | Follows from bounded log reads; after cutover. |
| FEATURE-003 | Disk preparation and power management. | Destructive; needs its own design after cutover. |
| FEATURE-004 | SnapRAID threshold notifications. | Covered by deterministic events and notifications in P04. |
