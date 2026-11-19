# B1 status, narrow layout and full-panel evidence

Signed `0.4.14+healthui.1` packages now contain corrected history/policy/save
notices and account-text wrapping. Static notices describe partial coverage;
schedule-specific waiting/checking/unavailable states still describe current
producer availability. The actual full-length account ID previously widened a
390-pixel page to 470 pixels. It now wraps without concealing overflow, while
table regions retain their own horizontal scrolling.

Both fresh installed AMD64 browser runs pass the real control flow: login,
permission-refused create, password enablement/relogin, policy save, one-minute
schedule creation, ordinary timer incident/recovery, acknowledgment and pause.
Each run includes two actual Core SIGKILL/automatic restarts. Exact original
history and the same browser session survive; final counts are one paused
revision-two schedule, two occurrences/jobs/results, one clear alert and two
one-attempt delivered notifications. Due tuples are 60 seconds apart. The
guest-wide OOM-kill counter stays zero and health coverage remains partial.

## Sources and checks

- Product correction `473544b`: four UI source files and two existing frontend
  test files changed. Design `f378fd0` records the intended boundary.
- Package/UI source `4a165707d5630828c864418ecb86a196f135609f`, 749 checked
  inputs. All five executable hashes remain those of original source
  `e417751ba57a3945fb47e60d513c087f75d5da7b`; no Rust/protocol/schema input changed.
  Since that original 740-input build there are five runtime changes (the earlier
  Core unit correction plus four UI files), two changed frontend tests and nine
  added test files. These categories are recorded separately.
- The first fixture uses that same `4a16570` source/map. Follow-up fixture
  `23eeaa552fd286245086e1ca09aa33042499fb14` changes exactly two test files;
  all application inputs and package bytes remain identical. Its manifest reports those
  changes explicitly. The same candidate-14 package bytes are used in both VMs.
- **244 frontend cases**, strict TypeScript and the fresh Vite production build
  pass. **19 scoped reference cases** pass on each fixture version: eight browser
  boundary cases, nine timer-fixture cases and two package-profile cases. Ruff,
  formatting and repository/diff checks pass. The second run adds a clipped-image
  regression; it is not a second broad frontend/Rust qualification count.

The minimal layout diagnostic first used a non-representative repeated `f` ID
and did not reproduce overflow. Using the actual earlier installed account ID
isolated the overflowing account span. The corrected stylesheet measures
390-pixel page width in a 390-pixel viewport, with a 640-pixel schedule table
scrolling inside a 348-pixel region. The full installed narrow pages separately
pass the page-width check. These diagnostic fixtures establish CSS behavior;
installed acceptance comes from the genuine guest runs.

Both actual standard/shadow Debian payloads have root archive ownership,
unchanged executable/unit hashes and the exact new UI assets. Only standard is
installed here. Disposable repository signature and APT verification use public
fingerprint `65E8DC7AE10AA5C4837A908EB4BE90BE61BCD42C`, expiring
`1794500424`. Private signing/SSH keys, browser profiles, passwords and cookies
are excluded. Original result packets, public repository material, package/build
records and reproduction sources are retained. No application configuration,
origin, Root policy, resource ceiling or further confinement exception changed.
The already documented Core file-bit-filter exception remains explicit.

## Original and corrected captures

The [first result](EVIDENCE-BUNDLE.json "bundled: installed-result.json") passes status and geometry checks, but
WebDriver's element screenshot clips tall targets to the visible viewport.
Those original five PNGs, result, source map and harness archive are preserved
unchanged; they are not credited as complete panel evidence.

The [follow-up result](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/installed-result.json") uses bounded
Chromium document-rectangle capture, verifies requested target dimensions against
PNG dimensions, and captures both horizontal edges of the narrow notification
table. Its six original PNGs were visually inspected:

- [Policy](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/policy-desktop.png") shows the corrected
  partial-coverage notice and save confirmation, threshold one and current controls.
- [Incident](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/incident-desktop.png") shows the open
  warning, one delivered incident and acknowledgment control.
- [Recovery](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/recovery-desktop.png") shows the clear
  alert and exactly two delivered transitions for the same container.
- [Narrow history](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/acknowledged-mobile.png") and its
  [right edge](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/acknowledged-mobile-right.png") show
  the transition rows, delivery states and persisted acknowledgment through the
  existing scroll region. Cell labels can wrap in the narrow layout.
- [Narrow schedules](EVIDENCE-BUNDLE.json "bundled: full-panel-followup/screenshots/paused-mobile.png") shows the
  persisted paused state and accessible form with an empty password field.

This accepts the shown status/notification/schedule states and page-overflow
regression. The captures record requested rectangles; Chromium scrollbar/reflow
can alter adjacent spacing between frames. They are not a pixel-perfect audit of
all views or every mobile interaction. Full UI parity and other states remain
separate work. [Visual review](visual-review.json) records the scope explicitly.

## Resources and cleanup

Original VM one ran **16:21:08–16:25:19 UTC**, exit zero, unchanged inputs,
minimum host availability **8,941,129,728 bytes**. Follow-up VM two ran
**16:29:06–16:34:37 UTC**, exit zero, unchanged inputs, minimum availability
**9,003,905,024 bytes**. Both retain the canonical 1,024-MiB/two-vCPU guest,
4.5-GiB host admission, unchanged sampled 3-GiB stop floor and 8-GiB free-disk
floor. All phases ran serially with repository-disk scratch. No sampled stop or
guest OOM kill occurred. Service observations retain the original ceilings.

Both original wrapper/runner/QEMU identities were captured live, including the
second VM during its longer bounded wait. [Final cleanup](EVIDENCE-BUNDLE.json "bundled: observations/cleanup-2.json.gz")
confirms all **18 unique recorded native qualification PIDs absent** and both
exact owned VM directories removed. A later read-only SSH observation arrived
after normal cleanup; authoritative guard/result records establish completion.
No unrelated process was stopped. No Pi, production service, external delivery
or publication was involved. The browser's disposable local-certificate acceptance
and disabled sandbox do not qualify production TLS/browser hardening.

B1/RW-045/P04 remain incomplete. Other health sources, external ordinary channels,
replacement/retirement and compaction, deployment/storage effects and recovery,
current ARM64/Pi, upgrade/return, performance/footprint/soak and P05/P06 remain
open. Next work returns to the registered Stack/deployment path and the independent
A1 task/FD-churn barrier. See the [design](../../../plans/2026-11-05-b1-status-panel-design.md)
and [B1 handoff](../../../p04-b1-observation-supervisor-handoff.md).
