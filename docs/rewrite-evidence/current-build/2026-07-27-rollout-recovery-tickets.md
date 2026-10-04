# LimeOS / Rollout and Recovery Hardening - Remediation Tickets

Date: 2026-07-27
Source: external project review (rollout/recovery pass), verified against the working tree on
`fix/ao009-actuator-lifecycle-socket` before ticketing
Branch: TBD (recommend `feature/rollout-hardening`)
Status: Planned

## Objective
Close the install, activation, and recovery failure paths the unit suite never exercises. Every
defect here lives on the edge the tests do not reach — the installer, the systemd units it writes,
the privileged-helper repairs, and the config files that survive between runs. Fix these before
the structural refactors at the end of this document, because those refactors move exactly the
paths these defects sit on.

## Source Review Verification
The originating review's ten findings were checked individually. Outcome:

- **Nine reproduce.** Ticketed below.
- **One is rejected.** "journald restart and vacuum failures are ignored" is a documented,
  reasoned best-effort (`pihealth_helper.py:2666-2669`) — the cap governs journald from its next
  start and the config is already on disk. No ticket.
- **Two were re-ranked.** The credentials finding drops to P3 (the script's own write is atomic,
  so it cannot produce the corrupt file it warns about). The journald-detection finding is
  reframed: the ordering/empty-value bugs are marginal, but the *four divergent implementations*
  of the same check are not.
- **Three compounding effects the review missed** are folded into the tickets: MNT-001 + MNT-002
  chain into a silent boot-hang (see MNT-002), PLG-001 inverts rather than hides (see PLG-001),
  and the remedy proposed for PLG-002 does not actually fix PLG-002 (see PLG-002).
- Line references in the source review's `setup.sh` citations were ~26 lines stale (pre-diff
  numbering). All references in this document are against the current working tree.

## Working-Tree Caveat
`host_prerequisites.py`, `pihealth_helper.py`, `setup.sh`, `scripts/onboarding-preflight.sh`, and
their tests carry uncommitted journald-cap work. **HOST-001 subsumes part of it** — land or discard
that work before starting HOST-001, do not fix around it.

## Scope Guardrails
1. No API contract changes; this sprint is installer, unit-file, and persistence behaviour only.
2. Every fix lands with a failure-path test. That gap is the whole reason these defects exist.
3. Host-repair logic gets exactly one owner (the privileged helper). No new copies in shell.
4. `tox -e all` and the frontend bundle budget stay green.
5. TEST-001 lands before any REFAC ticket starts. Non-negotiable — see the note on REFAC.

## Remediation Order (recommended)
1. Delete-only quick wins: **PKG-001**, **HYG-001**, **CI-001**.
2. Install correctness: **INST-001**, **INST-002**.
3. Boot-path correctness: **MNT-001**, **MNT-002** (do these together; they compound).
4. Single owner for host repairs: **HOST-001**.
5. Persistence fail-closed: **PLG-002**, **PLG-001**, **DEP-001**, **INST-003**.
6. Installer failure-injection harness: **TEST-001**.
7. Structural work, gated on TEST-001: **REFAC-001..004**.

## Execution Order and Dependencies
| ID | Title | Severity | Area | Depends | Status |
|---|---|---|---|---|---|
| PKG-001 | Stop publishing the unsupported Docker image | P1 | ci | — | Planned |
| INST-001 | Refuse root as the dashboard service account | P1 | installer | — | Planned |
| MNT-001 | Mount-wait unit waits on device mountpoints, not media dirs | P1 | backend | — | Planned |
| MNT-002 | Startup-service activation reports real helper results | P1 | backend | — | Planned |
| HOST-001 | One owner for host prerequisite detection and repair | P1 | installer+helper | — | Planned |
| INST-002 | Helper symlink is idempotent and re-points on move | P2 | installer | — | Planned |
| PLG-002 | Fail closed on unreadable plugin configuration | P2 | backend | — | Planned |
| PLG-001 | Plugin registration failures are diagnosable | P2 | backend | PLG-002 | Planned |
| CI-001 | Enforce lint and shell/unit validation in CI | P2 | ci | — | Planned |
| DEP-001 | Bound the runtime dependency set | P2 | build | — | Planned |
| HYG-001 | Untrack generated files, drop dead code, consolidate docs | P3 | repo | — | Planned |
| INST-003 | Validate preserved credentials instead of trusting size | P3 | installer | — | Planned |
| TEST-001 | Installer and activation failure-injection harness | P2 | tests | INST-*, MNT-*, HOST-001 | Planned |
| REFAC-001 | Centralise config persistence on `JsonFileRepository` | P3 | backend | PLG-002 | Planned |
| REFAC-002 | Split the privileged helper | P3 | helper | TEST-001 | Planned |
| REFAC-003 | Reduce `app.py` to construction and registration | P3 | backend | TEST-001 | Planned |
| REFAC-004 | Move root modules into a `limeos` package | P3 | repo | TEST-001, REFAC-002/003 | Planned |

---

## P1 — Rollout correctness

### PKG-001 — Stop publishing the unsupported Docker image
Files: `.github/workflows/docker-image.yml`, `docker-compose.yml`, `Dockerfile`, `README.md:63`.

The README states Docker deployment of Pi-Health itself is unsupported, while
`docker-image.yml:34` pushes `:latest` to Docker Hub on every commit to `main`. The image has
never worked: `Dockerfile:6` sets `APP_PORT=8080` and `Dockerfile:24` exposes 8080, but **nothing
in the codebase reads `APP_PORT`** — `app.py:1997` reads `PORT`, defaulting to 8002. The container
listens on 8002 while `docker-compose.yml:5` maps `8080:8080`. The image also cannot provide the
host helper boundary, which is the product's entire privilege model.

Tasks: delete the workflow, `Dockerfile`, and `docker-compose.yml`. Remove or deprecate any
published tags. If container deployment is ever wanted, it returns as a supported, tested product
with its own privilege story — not as an unowned build artefact.
Acceptance: no CI job publishes an image; the repository no longer ships a container definition
the README disowns.

### INST-001 — Refuse root as the dashboard service account
Files: `setup.sh:5`, `setup.sh:348`, `setup.sh:447`, `setup.sh:479`, `start.sh:6`,
`scripts/onboarding-preflight.sh:118-147`.

`RUN_USER="${SUDO_USER:-${USER:-$(id -un)}}"`. A root SSH session hits `start.sh:6` with `EUID`
already 0, execs `setup.sh` directly, so `SUDO_USER` is never set and `RUN_USER` resolves to
`root`. That account is then written into both generated units (`User=${RUN_USER}` at `:447` and
`:479`) and added to the `pihealth` group at `:348`. The dashboard runs as root and the entire
privileged-helper boundary becomes decorative. The preflight does not catch it: `:141-142` checks
that root is *reachable* and prints "Root access is available" — it never checks whether root is
about to *become* the service account.

Tasks:
- Reject `root` as `RUN_USER` on a fresh install with an actionable message, or create a dedicated
  unprivileged `limeos` service account and use it.
- Add a preflight check that names the account the install will use, and blocks when it is root.
- **Migration:** decide and implement what happens to hosts already installed this way. An update
  that silently keeps running as root leaves the fleet unfixed; one that reassigns the service
  user must also re-own `${LIMEOS_CONFIG_DIR}`, `${LIMEOS_STATE_DIR}`, `${LIMEOS_LOG_DIR}`,
  `${STACKS_PATH}`, and the venv. Record the decision in this ticket before coding.
Acceptance: a fresh `root` SSH install either fails with a clear instruction or provisions a
non-root service account; an existing root install is detected and handled by a stated path.

### MNT-001 — Mount-wait unit waits on device mountpoints, not media directories
Files: `media_paths_service.py:28-38`, `helper_templates.py:14-27`, `storage_compatibility.py:9-17`,
`storage_contract.py:257-299`.

`startup_service_params` selects any path starting with `/mnt/`, and under guided storage those
paths come from the contract: `media_host`, `downloads_host`, `backup_host`. The contract
*requires* those to be strictly below the device mountpoint — `_require_descendant(...,
allow_root=False)` at `storage_contract.py:257-258` for `single_disk` and `:282` for
`protected_pool`. So the generated script tests `mountpoint -q /mnt/storage/media`, which is a
directory on the mount, never a mount point.

The generated unit is `Type=oneshot` (`helper_templates.py:37`), and per `systemd.service(5)` the
start timeout is **disabled by default** for oneshot units. The `while true` loop at
`helper_templates.py:15-26` therefore waits forever, holding a start job against
`multi-user.target`, and `docker compose up -d` never runs. `separate_downloads` passes
`allow_root=True` and may be unaffected.

Reachability: `MediaPathsService.update()` raises `MediaPathsManagedError` under a contract, so
guided storage never generates this on its own. The live path is the operator pressing the Startup
service card on the Mounts page — `POST /api/disks/startup-service` → `disk_manager.py:437` →
`apply_startup_service()` → `self.paths()`.

Tasks: derive the wait list from the contract's device mountpoints, not from the projected media
paths. Add a bounded wait (`TimeoutStartSec=` and a loop ceiling) so a wrong list degrades to a
failed unit instead of a hung boot. Keep the legacy non-contract behaviour working.
Acceptance: with each storage profile, the generated script tests only real mount points; a
never-satisfied wait fails the unit within a bounded time instead of blocking the boot target.

### MNT-002 — Startup-service activation reports real helper results
Files: `media_paths_service.py:130-139`, `pihealth_helper.py:1270-1295`,
`tests/test_media_paths_service.py:193-217`.

`apply_startup_service` checks the result of `configure_startup_service` and then discards the
results of both follow-up calls, returning `{"success": True}` unconditionally at `:139`. The
existing test asserts only the call sequence, which locks the behaviour in.

This compounds with MNT-001 and is the sprint's worst combined path. `cmd_systemctl` appends
`--now` for `enable` (`pihealth_helper.py:1291-1292`) and runs with `timeout=60`. With contract
paths, the operator clicks the button, `systemctl enable --now docker-compose-start.service`
blocks inside the hanging mount-wait loop, the helper call times out after 60s, `:137-138` throws
the failure away, and the UI reports `{"status": "updated"}` — having just enabled a unit that
will hang the next boot.

Tasks: check both helper results; return the specific failing step with a recovery message;
disable the unit again if activation fails after it was written. Replace the sequence-only test
with cases for daemon-reload failure, enable failure, and helper timeout.
Acceptance: any failing activation step surfaces as an error in the API response and the UI; no
path returns success for a unit that was not activated.

### HOST-001 — One owner for host prerequisite detection and repair
Files: `setup.sh:90-148`, `scripts/onboarding-preflight.sh:208-221`,
`pihealth_helper.py:2541-2677`, `host_prerequisites.py:31`, `host_prerequisites.py:99-124`,
`host_prerequisites.py:190-212`, `app.py:1889`.

The same two host repairs now exist twice, and the "is the journal capped" question is answered
four times with three different matching rules:

| Location | Test |
|---|---|
| `setup.sh:109` | `grep -rqs "^[[:space:]]*SystemMaxUse"` |
| `onboarding-preflight.sh:213` | same grep |
| `pihealth_helper.py:2607` | `line.lstrip().startswith("SystemMaxUse")` |
| `host_prerequisites.py:31` | `re.compile(r"^\s*SystemMaxUse\s*=\s*(\S+)")` |

Only the last rejects `SystemMaxUse=` (empty, meaning "distro default", i.e. not a cap). The first
three also match `SystemMaxUseAnything=`. The memory-cgroup repair is likewise duplicated between
`setup.sh:96-122` and `pihealth_helper.py:2541`, and the shell copy writes `cmdline.txt`
non-atomically — `cp -a` backup at `:116` then `printf ... > "${cmdline_file}"` at `:118` — while
the helper already does mkstemp/fsync/copymode/`os.replace` at `pihealth_helper.py:2575-2596`. An
interrupted write or a full boot partition there leaves the Pi unbootable.

The correct fix is not to harden the shell copies. The helper owns both repairs and `app.py:1883`
converges them at startup, so the installer copies are redundant.

Tasks:
- Delete `fct_enable_memory_cgroup` and `fct_cap_journal` from `setup.sh`; have the installer
  invoke the helper's `host_prerequisites_apply` (or rely on the startup convergence) and surface
  the reboot notice from its result.
- Reduce the preflight to *reporting* only, sharing one detection rule with `host_prerequisites`.
- Make the helper's `_journal_cap_is_set` use the same value-requiring rule as
  `host_prerequisites.py:31`.
- Honour override order in `is_journal_capped`: `read_journal_config`'s docstring at `:103` says
  "drop-ins last so they win", but `:124` uses `any()`. Evaluate the last assignment wins.
- Fail closed in `HostPrerequisiteService.apply()` when the helper returns success but omits an
  expected result id (`:193-212` currently returns `changed=False, errors=[]`).
Acceptance: exactly one implementation of each detection and each repair; no privileged boot-file
write outside the helper; a drop-in that resets `SystemMaxUse=` is reported as uncapped.

---

## P2 — Recovery and persistence

### INST-002 — Helper symlink is idempotent and re-points on move
Files: `setup.sh:341-343`, `setup.sh:403`.

```bash
if [[ ! -e "$HELPER_LINK" ]]; then
  ln -s "${REPO_DIR}/pihealth_helper.py" "$HELPER_LINK"
fi
```

`-e` follows the link, so a **dangling** link makes the test true, `ln -s` then fails with
`File exists`, and `set -Eeuo pipefail` aborts the install with a message that does not name the
cause. A **valid** link pointing at an old checkout is silently preserved: the helper unit's
`ExecStart=/usr/bin/python3 ${HELPER_LINK}` (`:403`) keeps running the old helper while the app
unit runs the new one from the new `REPO_DIR`, and the unit's `ReadWritePaths` reference the new
path. That version skew is silent and survives updates.

Tasks: resolve the existing link, verify its target is a managed checkout, and replace it
atomically (`ln -sfn`, or symlink-to-temp + `mv -T`). Refuse to clobber a non-symlink at that path.
Acceptance: rerunning the installer over a dangling link, a link to a previous checkout, and a
regular file each produce a correct link or a clear refusal — never a confusing abort or silent
skew.

### PLG-002 — Fail closed on unreadable plugin configuration
Files: `plugin_manager.py:60-73`, `plugin_manager.py:88`, `ports.py:200-232`,
`storage_capability_adapters.py:76`, `agent_actions/repair_job.py:26`.

`_load_config` (`:60-66`) swallows every read and JSON error into `{"plugins": []}`;
`load_plugins_config` (`:88`) then writes merged defaults straight back over the corrupt file. The
operator's plugin state is destroyed by the act of reading it.

Two aggravating factors the source review did not name:

1. **Every read is a write.** `is_enabled` → `get_plugin_entry` → `load_plugins_config` →
   `_save_config` on every call. `_save_config` (`:70-73`) is a plain truncating
   `open(..., "w")` with no temp-and-replace, so plugin status checks rewrite the file
   non-atomically — which is also the most likely *source* of the corruption this ticket is about.
   On a project whose journald cap exists to reduce SD-card writes, this is the same problem.
2. **The proposed remedy does not fix the finding.** `JsonFileRepository.read_json`
   (`ports.py:203-208`) catches `ValueError` and returns the default — the same corrupt-to-default
   collapse. Adopting the repository fixes atomicity only.

Three readers also disagree about a bad file: `plugin_manager` silently defaults, while
`storage_capability_adapters.py:76` and `agent_actions/repair_job.py:26` `json.loads` it directly
and raise.

Tasks: give `JsonFileRepository` explicit missing / corrupt / valid outcomes so callers can choose;
make `plugin_manager` use it, quarantine a corrupt file (`plugins.json.corrupt-<stamp>`) rather
than overwrite it, and surface the state to the plugin UI; stop writing on read — persist only on
mutation; route the two direct readers through the same repository.
Acceptance: invalid JSON is preserved and reported, not replaced; reading plugin status performs
no writes; all readers agree on what a corrupt file means.

### PLG-001 — Plugin registration failures are diagnosable
Files: `storage_plugins/registry.py:130-178`.

Every built-in and third-party import and registration is wrapped in `except Exception: pass`, so
a broken plugin simply vanishes with no log line. Worse than vanishing, one case **inverts the
policy**: `:130-135` wraps `import plugin_manager` in the same swallow and falls back to

```python
def enabled(_id):
    return True
```

so a `plugin_manager` import failure silently enables *every* plugin regardless of configuration.

Tasks: log a bounded diagnostic per plugin (id, phase, exception type, message) and expose it
through plugin status. Make the `plugin_manager` import failure fail closed — no plugins enabled —
and report it.
Acceptance: a deliberately broken plugin appears in status with its failure reason; a
`plugin_manager` import failure disables plugins rather than enabling all of them.

### CI-001 — Enforce lint and shell/unit validation in CI
Files: `.github/workflows/tests.yml:58-59`, `tox.ini:22-26`.

`tox.ini` already defines a `lint` env (`ruff check --select E9,F .`) that CI never runs —
`tests.yml:59` runs `tox -e unit` and the e2e job only. There is no ShellCheck over
`setup.sh`, `start.sh`, or `scripts/*.sh`, and no `systemd-analyze verify` over the generated
units, in a sprint whose defects are concentrated in exactly those files.

Tasks: add `tox -e lint` to the workflow; add a ShellCheck job over all tracked shell scripts; add
`systemd-analyze verify` against rendered unit templates. Fix or explicitly baseline existing
findings so the gate starts green.
Acceptance: lint, ShellCheck, and unit-file validation all gate `main` and pull requests.

### DEP-001 — Bound the runtime dependency set
Files: `requirements.txt`.

Four of eight requirements are unbounded: `flask`, `psutil`, `docker`, `pyyaml`. (`jsonschema`,
`ruamel.yaml`, and `websocket-client` carry ranges; `apscheduler` has a floor only.) There is no
lockfile, so a Flask or docker-py major release breaks fresh installs on hosts nobody has touched.

Tasks: add upper bounds to the four unbounded entries and an upper bound to `apscheduler`;
decide whether a lockfile or a constraints file is warranted for installs.
Acceptance: a fresh install resolves to a tested dependency set regardless of upstream releases.

---

## P3 — Hygiene and low-severity

### HYG-001 — Untrack generated files, drop dead code, consolidate docs
- 81 tracked `.pyc` files. `.gitignore:1-2` already lists `__pycache__/` and `*.pyc`; ignore rules
  do not untrack. `git rm --cached` them.
- `frontend/src/pages/coming-soon-page.tsx` and `frontend/src/pages/containers-placeholder.tsx`
  are tracked with zero references anywhere in `frontend/src`. Delete them.
- `Docs/` (sprint docs, logo PNGs, `legacy/`) and `docs/` (only `plans/`) are parallel roots on a
  case-sensitive filesystem. Pick one; a case-insensitive checkout will collide.
- `companion/pi-health-companion/package.json:2` is still `"name": "react-example"`. Rename, or
  decide the companion app is out of scope and split it out. (`frontend/package.json` is correctly
  named — this is the companion only.)

Acceptance: no generated artefacts tracked, no unreferenced pages, one documentation root, no
placeholder package identity.

### INST-003 — Validate preserved credentials instead of trusting size
Files: `scripts/onboarding-credentials.sh:133-137`.

`if [[ -s "${CREDENTIALS_FILE}" ]]` preserves any non-empty file without checking for
`PIHEALTH_USER`, `PIHEALTH_PASSWORD_HASH`, or hash validity. A damaged file survives every rerun
while the application refuses to start, and the README documents the preserve behaviour so the
operator has no reason to suspect it.

Re-ranked to P3: the script's own write is atomic (`mktemp` + `chmod` + `mv -f` at `:115-124`), so
it cannot produce the truncated file it fails to detect. This needs external damage or a manual
edit. Valid hardening, low likelihood.

Tasks: validate required keys and hash format; on failure, stop with a repair instruction and
**do not overwrite**. Never print the hash.
Acceptance: a rerun over a malformed credentials file exits non-zero with a named repair step and
leaves the file untouched.

---

## Test harness — gate for the structural work

### TEST-001 — Installer and activation failure-injection harness
Files: new under `tests/`, plus `scripts/*.sh`.

Every defect in this document sits on a path the current suite does not execute. Coverage needed:
interrupted boot-file write; dangling and stale helper symlinks; root SSH install; malformed
credentials; absent Docker; contract media paths nested under a mount (MNT-001); failed
`daemon-reload` / `enable` / helper timeout (MNT-002); corrupt `plugins.json` (PLG-002); broken
plugin import (PLG-001).

Prefer a container or chroot fixture that can run `setup.sh` against a disposable root, plus unit
tests for the Python paths. Shell functions should be sourceable for direct testing.
Acceptance: each ticket above has a test that fails against the current implementation.

---

## Structural recommendations — sequencing note

The source review proposed the structural work first and the test harness last. **That ordering is
inverted and this sprint reverses it.** Splitting a 6,033-line helper, reshaping a 1,999-line
`app.py`, and moving 83 root modules into a package are precisely the changes that alter
`REPO_DIR`-relative paths, the helper unit's `ExecStart` (`setup.sh:403`), and the symlink target —
i.e. they will trip INST-002 silently, leaving a valid link pointing at the old layout while the
app runs the new one. TEST-001 must exist first.

### REFAC-001 — Centralise config persistence on `JsonFileRepository`
Depends on PLG-002 (which adds the missing/corrupt/valid states the other callers need). Audit
every JSON read/write for direct `open`/`json.load` use and route it through the repository.

### REFAC-002 — Split the privileged helper
`pihealth_helper.py` is 6,033 lines. Separate the dispatcher/protocol layer from fixed command
modules for storage, host, integrations, agents, and updates. The command whitelist
(`pihealth_helper.py:5702`) is the security boundary and must remain a single reviewable list
after the split.

### REFAC-003 — Reduce `app.py`
1,999 lines. Leave application construction and route registration; move remaining behaviour into
domain services alongside the existing ones.

### REFAC-004 — Move root modules into a `limeos` package
83 Python modules at the repository root. Group by domain. This ticket changes the systemd
`ExecStart` paths and the helper symlink target, so it lands last and only with INST-002 and
TEST-001 in place.

---

## Not actioned

**journald restart/vacuum failures are ignored** (`pihealth_helper.py:2668-2669`). Rejected as a
defect. The comment at `:2666-2667` states the reasoning: the cap governs journald from its next
start, the config is already on disk, and the vacuum is opportunistic reclamation. Failing the
repair because a reclamation step failed would be worse behaviour, not better.
