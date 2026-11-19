# B1 installed browser controls pass on disposable AMD64

On 5 November 2026, fixture `a652a7b` passed the real browser journey against
signed standard package `0.4.13+coreopen.1`. Actual Caddy served the installed UI;
Chromium used the real Core API, password authentication, grants, protected
reader, Docker daemon and ordinary observation timer. The canonical guest stayed
at 1,024 MiB and two vCPUs. No API routing fixture, session injection, clock
override, direct database write, synthetic settlement or mutation retry was used.

This accepts the covered installed browser control flow. Inaccurate availability
copy and broader UI/health coverage remain open. It does not close B1 or P04.

## What passed

- Actual sign-in, a schedule POST refused with `403` before management enablement,
  unchanged durable history, password enablement and actual sign-in again.
- The policy form saved failure threshold one. The schedule form created one
  one-minute health schedule with `201`, using normal wall time.
- The real timer recorded one stopped-container incident and delivered its in-app
  notification once. The history panel rendered it. Actual Core SIGKILL and
  automatic restart preserved every original occurrence/job/result/alert/notice
  identity, and the same browser session still worked after page reload.
- Independently starting the same real container produced one recovery for the
  same generation. The browser rendered the recovery and clear alert state.
  Acknowledgment and pause each used one successful UI POST. Both persisted after
  a second actual Core crash and page reload, with exact durable history preserved.

[Installed result](EVIDENCE-BUNDLE.json "bundled: installed-result.json") contains the independently read SQLite
snapshots, resource/unit observations, notification identities and sanitized
method/path/status records. Final counts are one paused schedule at revision two,
two occurrences, two succeeded observation jobs/results, one clear alert and two
one-attempt delivered in-app notifications. Due occurrences are exactly 60 seconds
apart. The incident acknowledgment persists. Actual Core PIDs were
`2834 → 8529 → 8905`. Container-only health coverage stays partial; the recovery
describes that container, not the complete host.

The browser was Chromium `156.0.8078.79`, with matching Debian driver, Selenium
`4.8.3+dfsg-1` and Caddy `2.6.2-5`. Test dependencies were installed only in the
throwaway guest. Chromium accepted the guest's local certificate and ran without
its sandbox there. Production certificate trust and browser hardening were not
qualified. Package Core origin, configuration, reader policy and actual unit
files matched their before-images after the journey. This uses the previously
qualified Core profile correction, including its explicit removal of the
SUID/SGID file-bit restriction; no further confinement exception was added.

## Source and failures

[Source joins](source-match.json) and the original frozen manifests distinguish:

- Fixture `a652a7b4b3345676dd3f89971f17f774a0be1a99`, 749 checked inputs.
- Current package source `ce2e9163a7a64ce57f0936034c81cfdf72c439d3`, 747 inputs.
  The fixture adds exactly two test files; all current package inputs match.
- Executable/UI source `e417751ba57a3945fb47e60d513c087f75d5da7b`, original 740
  inputs. The only production delta is the already tested Core unit correction;
  nine test files were added since that original build. All five installed binary
  hashes and all three UI assets match the reused package payloads exactly.

No application rebuild was necessary. The existing signed candidate-13 repository
and public verification material are preserved; private signing keys and guest
SSH keys, passwords, cookies and browser profiles are excluded. Credentials were
freshly generated, used through the actual console enrollment/browser flow, and
never included in result data or raw network logs. Both attempts' 34-module guest
copies and unchanged canonical runner are available in
[reproduction sources](EVIDENCE-BUNDLE.json "bundled: reproduction-sources.tar.gz").

The first installed attempt, fixture `692d92d`, failed before installation because
the guest root cgroup has no `memory.events` file. Its original logs and bounded
failure packet remain in `checks/` and `observations/`. The fixture now uses the
kernel's guest-wide `/proc/vmstat` `oom_kill` counter and refuses its absence.
No application action was retried from that guest. A fresh guest passed with
counter zero before and after. A cached preparation helper also stopped after
freezing attempt two because it mistakenly renamed the old launcher input path;
the frozen manifest/guest files were preserved and distinct launch helpers were
completed without recreating those inputs.

The first native development check failed two tests that expected the wrong
exception type for the existing refusal helper; the failures remain archived.
Subsequent development checks passed on their own earlier inputs. Only final
reference attempt five is credited to the current source: **18 scoped cases**
(seven browser-boundary, nine timer-fixture, two package-profile cases), plus
Ruff, formatting, repository and diff checks. These are not extra installed
scenarios or a new broad Rust/ARM64 qualification claim.

## Resources, cleanup and visual limits

The successful original VM ran **16:04:08–16:07:28 UTC**, exit zero, all 749
inputs unchanged, no sampled stop and minimum host availability
**9,391,218,688 bytes**. The unchanged sampled stop floor was 3 GiB; admission
was 4.5 GiB with the 8 GiB free-disk floor. Native checks and VMs ran serially
using repository-disk scratch. Core retained 128 MiB, 32 tasks and 1,024 FDs;
reader observations and limits are in the result.

Original live wrapper/runner/QEMU identities were captured for both VMs; the
second observation preceded QEMU startup, and the third captured its actual PID
and start ticks. [Cleanup](EVIDENCE-BUNDLE.json "bundled: observations/cleanup-final.json.gz") confirms all
**16 unique recorded native qualification PIDs absent** and both exact owned
temporary VM directories removed. No unrelated process was stopped. No Pi,
production service, external delivery or publication was involved.

The three screenshots retain the actual phase viewports: desktop 1,100×757 and
narrow 390×701. They were visually inspected and contain empty password fields.
The desktop captures show the surrounding controls and stale availability notice;
the notification rows are outside those captured viewports. The narrow capture
shows the paused schedule and scrolling table. These frames do not independently
establish incident/recovery table appearance or complete mobile layout acceptance;
the browser's actual DOM assertions and HTTP/SQLite joins establish the tested
state transitions. Targeted panel captures and mobile overflow review remain open.

Next: correct the history/policy/save notices that say automatic observations are
unavailable, then qualify those new packaged assets with targeted visual evidence.
Other health sources, ordinary external channels, replacement/retirement and
compaction, current ARM64/Pi, upgrade/return, resource/performance/soak, remaining
A/B/P04 and P05/P06 stay open. See the
[design](../../../plans/2026-11-05-b1-installed-browser-design.md) and
[B1 handoff](../../../p04-b1-observation-supervisor-handoff.md).
