# Real-browser qualification: operator resolution UI (Needs attention banner and dialog)

Disposable Debian 12 AMD64 KVM guest (one at a time, `tests/privileged_vm/run.py`; no remote host, no push). Packages built
inside the guest from the bundled source of `b14e774` (`0.4.17+resbrowser.1`), Caddy-served frontend, packaged core and
executor, genuine dockerd, headless Chromium 156.0.8078.79 driven by Selenium/chromedriver (as in the Stack and
Automation browser guests). Guest script: `tests/privileged_vm/p04_resolution_browser_guest.py`; it reuses the induction
helpers of `p04_resolution_guest.py` (container restart killed mid-stop, failing OCI starts, stack restart/start).
Seven jobs are induced as `needs_intervention`: five queued by the administrator, two (a container and a stack) by an
operator-role user, because job lists are per principal (see Findings).

The UI under test is `frontend/dist` built from `f5d8d5a` and copied over `/usr/lib/limeos/ui` in the guest
(`run.py --guest-file`; the package copies `frontend/dist` verbatim, so this equals repackaging; both hash sets are in
[result.json](EVIDENCE-BUNDLE.json "bundled: raw/result.json") under `ui_sha256` and `packaged_ui_sha256`). Final run: [result.json](EVIDENCE-BUNDLE.json "bundled: raw/result.json"),
12 of 12 cases pass; screenshots in [screenshots/](EVIDENCE-BUNDLE.json "bundled: screenshots/") (viewport PNGs, password field asserted empty first).

## Cases

| Case | Result | Observed |
| --- | --- | --- |
| 1 admin banner, containers and stacks | pass | "Needs attention · 3 operations" on the container list and "· 2" on the stack list, each entry with an aria-labelled "Review and resolve" button; admin does not see the operator's jobs (Finding 1) |
| 2 operator read-only | pass | operator sees the banner for the jobs they queued with "Ask an administrator to review them.", no button, no dialog element, on both lists |
| 3 dialog focus | pass | modal (`:modal`), focus moves inside (Close button), 40 Tab and 40 Shift+Tab presses never leave it, every control has an accessible name and no dangling `aria-describedby`; focus returns to the opener on Close and on Escape |
| 4 plan vs measured | pass | per target "Plan expected" and "Measured now" with image, started_at and running, "Same as / Differs from the plan" and "intended end state" sentences equal the API view, which the guest checks against `docker inspect` (container: 1 target, stack: 3) |
| 5 disallowed outcomes | pass | H: not_done disabled; D3: verified_done and not_done disabled; N1: verified_done disabled; each with "Not available: <ids> ..." linked by `aria-describedby`; allowed ones show what they mean |
| 6 divergence | pass | label discloses "at least 20, at most 500"; a 16-character reason gives "(16 written)" and stays on the choose step with `aria-invalid`; accepted reason reaches a confirmation with the amber `.summary.divergence` panel, warning text and a `danger` button, computed style differs from the ordinary confirmation |
| 7 wrong password | pass | POST 401, message "The password was not accepted. Nothing was recorded.", password field emptied, still on the confirm step, session still 200, no state change, no Engine event |
| 8 stale evidence | pass | solo container started in the background while the confirmation was open; submit gives 409, view refreshed (running "yes", allowed outcomes updated), notice "The state changed since you reviewed it", confirmation and password field gone, no radio selected, focus on the decision heading, no state change; deciding again resolves it |
| 9 successful resolve | pass | "Resolved" record (outcome, reason, "The containers are unlocked"), job row shows resolved behind the dialog, banner count 2 to 1, no password field left, core job `resolved`, locks gone, no Engine lifecycle event; focus after Close lands on the banner heading |
| 10 pending (202) | pass | dockerd held and the executor killed and kept down the instant core recorded the request: POST 202, "Recorded, waiting for the executor ... Submitting the same decision again is safe.", button "Submit the same decision again", request row present, password cleared; after the executor returns the identical submit gives 200 and the job is `resolved` with one record |
| 11 keyboard only | pass | opener reached by Tab and opened with Enter; Space selects the radio; Tab to the reason, typed; Tab and Enter on "Review decision"; focus on the confirm heading; Tab to the password, typed; Tab and Space on "Resolve job"; focus on "Resolved"; Tab reaches Close; Space closes; focus lands on the jobs heading. Escape verified in case 3 |
| 12 final and hygiene | pass | stack divergence resolved from the stack list; at every submit and at the end: password absent from DOM, field values, storage and cookie script view, console and every request URL; no `Authorization`/`X-API-Key` on resolution requests; the only SEVERE console lines are Chromium's logging of expected non-2xx fetches (pre-login session probe 401, wrong-password 401, stale 409, and the unrelated storage-mounts panel, which the fixture does not configure, 503/403 for the operator); no React warnings |

## Bugs found and fixed (all in the UI)

1. `4d16cce` (fixed, regression tests in `frontend/tests/resolution.test.mjs` and `focus-ring.test.mjs`)
   - After a resolution the focused "Resolved" heading follows the dialog's only control, so Tab left the modal for the browser chrome (`containFocus` only wrapped at the first and last control).
   - Closing the dialog of a resolved job dropped focus on `<body>`, because the opener (the banner button) had been removed. Focus now goes to the banner heading, or the jobs heading when the last entry is gone.
   - The container and stack banners used the same `id="attention-heading"` (both are on the stack page): now unique.
   First run (original packaged UI): cases 9 and 11 failed on these; [first-run-packaged-ui-result.json.gz](EVIDENCE-BUNDLE.json "bundled: raw/first-run-packaged-ui-result.json.gz").
2. `f5d8d5a` (cosmetic): the confirmation summary's labels touched its border; padded.

## Findings not fixed

1. **Administrators cannot reach an operator's stuck job from the UI.** `container_jobs` and `stack_jobs` select `WHERE principal=?`, so the banner lists only the viewer's own jobs. A `needs_intervention` job queued by an operator shows the read-only text to that operator and is invisible to every administrator; only `limeosctl job resolve <id>` reaches it. Backend/design decision, not changed here.
2. Chromium logs "Password forms should have (optionally hidden) username fields" for the dialog's password form (DevTools issue, not an error). A hidden username field would also help password managers; left as is.
3. After a 202 the pending notice sits at the top of a long dialog while the form is below; it is a polite `role=status` region so it is announced, but sighted users must scroll.
4. Colour contrast and screen-reader output were not machine- or reader-audited; only names, roles, focus order, containment and focus return were.
5. Unrelated: an operator without storage grants gets 403 from `/api/v1/storage/mounts/*` on every dashboard load (console noise).

## Harness changes

`run.py --guest-file` (repeatable) copies extra files to `/root/`. The guest builds one operator user with grants for real UI login and uses `systemctl stop` after killing the executor to keep it down for the 202 case.

## Not covered

429 on the password limiter in the UI, session expiry during the dialog, storage/backup jobs, narrow (mobile) viewport, ARM64/Pi.

## Reproduce

Build once (about 35 minutes; see the 2026-11-06 README), then:

```
(cd frontend && npm ci && npm run build) && tar -czf ui-dist.tar.gz -C frontend/dist .
python3 tests/privileged_vm/run.py --image .../debian-12-genericcloud-amd64.qcow2 --checksums .../SHA512SUMS \
  --repository BUILD/limeos-repo --package-version 0.4.17+resbrowser.1 \
  --guest-script tests/privileged_vm/p04_resolution_browser_guest.py --guest-file ui-dist.tar.gz --output result.json
```

Cleanup: after each run no `qemu-system` process exists and the `/tmp/limeos-p01-vm-*` directory is gone.
