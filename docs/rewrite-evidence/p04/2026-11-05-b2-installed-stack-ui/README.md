# Current installed Stack API/UI qualification

The covered B2 read flow passes on a disposable Debian 12 AMD64 VM: the signed
standard package, genuine Docker Engine, protected catalog, registered reader,
authenticated HTTP API and Chromium UI agree on the tested layouts. This closes
the current installed read/UI scenario, not B2, RW-042, P04 or Pi cutover.
The narrow member table still needs a readability correction and another visual
review; its capture also clips part of the Stack heading.

## Source and package identity

| Input | Identity |
| --- | --- |
| Final fixture | `4c4f7004564f72c43050d26a3acac7ffd3dd873d`, 752 hashed inputs |
| Package and production UI | `4a165707d5630828c864418ecb86a196f135609f`, 749 hashed inputs |
| Rust executables | `e417751ba57a3945fb47e60d513c087f75d5da7b`, original build 12 |
| Installed standard package | `0.4.14+healthui.1`, SHA-256 `40264798f797857f30560f608da8cbb07f19c46be4144afdedaca148c4fa1a73` |
| Shadow payload, not installed here | SHA-256 `17de12f85d42ec36f7761789535e6e647be1cb01ee63dc6ec94bba28d3d193b5` |
| Canonical VM | Unchanged runner, 1,024 MiB, two vCPUs |

[Source joins](source-match.json), [package inputs](package-inputs.json.gz) and
[original build record](original-build-record.json.gz) retain distinct provenance.
Only two existing test files differ between the package and final fixture; three
test files were added. All other package inputs and the installed executable/UI
bytes match. No production Rust, schema, contract, UI, policy or unit changed in
this slice. Earlier build/frontend qualification belongs to its original source;
no fresh broad Rust/frontend run or ARM64 result is claimed.

The final native checks ran before the fixture commit: their recorded Git head is
`45a5181`, while their exact before/after input map matches committed `4c4f700`.
That distinction remains in the original records. The earlier screenshot helper
change also belongs to `45a5181`; it captures approved Stack articles using full
CDP document rectangles rather than Selenium's viewport-clipped element capture.

## Covered installed behavior

Four checkpoint groups pass in [the installed result](installed-result.json):

1. All eight protected registrations appear in the actual API and UI with a
   genuinely empty Engine. A registered empty stack remains visible.
2. Sixteen real containers, eight running, and thirty-two read-only bind
   declarations preserve full IDs, stopped replicas, one-offs, an undeclared
   service, a standalone container, and missing ordinary services. Independent
   Engine inspection supplies the expected membership.
3. Eight registrations with sixteen declared services each reach the API/UI;
   the undeclared service produces seventeen observed rows for one stack.
   Making the fixture catalog writable causes genuine source refusal: previous
   facts remain stale, current counts become unknown and the original sample
   time survives. Restoring safe ownership/mode obtains a newer sample.
   Twenty GETs do not renew a sample or create durable authority/work.
4. An actual Core SIGKILL/autorestart, controlled reader restart and real browser
   logout/relogin recover the same Stack identities. Core PID changes from 2838
   to 9563; reader PID changes from 8211 to 9578.

The default enrollment stores explicit `HealthRead` grants; the administrator
role alone does not authorize these reads. Original Compose/sentinel bytes,
numeric ownership and modes, plus production config/policy/unit hashes, match
before and after. All twelve measured job, typed-plan, claim and automation
tables remain empty. Compose file state honestly remains `not_inspected`.

The fixture uses real Caddy and Chromium 156.0.8078.79 with the matching Debian
driver and Selenium 4.8.3. Its local certificate exception and disabled browser
sandbox apply only inside this disposable VM; production TLS/browser hardening
is outside this result. The HTTP trace records only method, path and status;
passwords, cookies and enrollment material are excluded.

## Resource observations and visual review

The complete reader service cgroup has 11,912 samples and a kernel memory peak
of **6,250,496 bytes (5.96 MiB)**. Maximum observed tasks are five and the sampled
descriptor maximum is eighteen. The unchanged service limits are 64 MiB,
sixteen tasks and 256 descriptors. Kernel memory/pids events record no limit or
OOM event; guest-wide OOM kills remain zero. Actual stable PID/start-time and
executable hashes identify manager, guardian and sealed worker, each with empty
effective/permitted/bounding capabilities and `NoNewPrivs=1`. Observer errors are
empty. These short, small-body cardinality scenarios do not establish worst-byte
footprint, descriptor peaks, long-soak or the full failure/kill/deadline matrix.

All five PNG hashes, dimensions, requested rectangles and page-width checks pass.
Direct review of the desktop captures supports the displayed empty, mixed,
maximum and refused states. [The visual review](visual-review.json) records a
remaining narrow-screen issue: member columns wrap heavily and the mobile CDP
capture clips part of the heading. Passing width geometry is insufficient for
mobile visual acceptance. No production UI correction is included here.

| Capture | Dimensions | Review |
| --- | --- | --- |
| [Empty](screenshots/empty-desktop.png) | 1045 × 288 | Empty registration, zero counts, file-inspection notice |
| [Mixed](screenshots/mixed-desktop.png) | 1045 × 769 | Partial ordinary service and distinct running one-off; full member identities |
| [Maximum](screenshots/maximum-desktop.png) | 1045 × 2331 | Sixteen declared services, missing-service count and member details |
| [Refused](screenshots/refused-desktop.png) | 1045 × 1971 | Unknown current counts and previous-observation labels |
| [Narrow maximum](screenshots/maximum-mobile.png) | 335 × 2903 | Width check passes; readability and heading capture remain open |

## Attempts, failure and cleanup

| Check | UTC interval on 5 November | Outcome |
| --- | --- | --- |
| Native reference 1 | 16:56:17–16:56:18 | 25 cases plus lint/format/repository checks pass |
| Installed attempt 1 | 16:58:16–17:01:40 | Fixture failure: observer queried nonexistent `approvals` table |
| Native reference 2 | 17:05:48–17:05:50 | 26 cases plus lint/format/repository checks pass |
| Installed attempt 2 | 17:06:58–17:13:29 | All four checkpoint groups pass |

The first attempt had passed empty and mixed API/UI assertions before its
read-only authority observer failed. The failure packet and original transcript
remain archived. Approval digests live on typed plan rows, not an `approvals`
table. Commit `4c4f700` uses the actual twelve tables and adds a regression that
applies the real schema/migrations and verifies the observer preserves DB bytes.
There was no application fix or package change between attempts.

The host guard remained unchanged, SHA-256
`380a78ebd9dbb3902a97bebbcdb164df1e6bfc4c069b56a45092af6eb26813d6`.
Both attempts retained identical input maps and stayed above its three-GiB stop
floor; the final VM minimum available host memory was 9,111,379,968 bytes.
VM admission required 4.5 GiB. Work and scratch stayed on repository disk.
The delayed cleanup record at 19:26:56 is separate from the 17:13:29 run end;
the intervening pause is not VM execution time.

[Cleanup](observations/cleanup-2.json.gz) confirms all ten recorded original
native PIDs are absent and both exact owned VM scratch directories are removed.
No unrelated process was stopped. Holly's Pi, the frozen application and the
Python checkout were untouched.

## Remaining gates and reproduction

Protected Compose inspection/review binding, whole-stack/catalog deployment
effects and recovery, grant replacement and complete death/cancellation/deadline
matrices, mobile readability, current native ARM64/Pi qualification, upgrade and
return rehearsal remain open. Historical performance evidence is not replaced
by this run. P05's minimal assistant and P06's migration/return rehearsal remain
required for full cutover.

[Frozen sources](reproduction-sources.tar.gz) contain both 36-module guest
cohorts, the exact runner, launch/admission/guard/observation helpers and reference
source. Original guarded records/logs are in `checks/`; inputs, failure, original
base64-bearing result and cleanup observations are in `observations/`. Public
signed APT metadata and the public key are retained under `repository/`; private
signing keys, credentials, guest disks and Debian payloads are not archived here.
The payload hashes identify the locally retained candidate 14 artifacts.

The final verification checks source joins, archived observations and image
geometry before sealing all packet files in `SHA256SUMS`. Verify from this
directory with `sha256sum --check SHA256SUMS`. Running the privileged guest
fixture again requires a newly admitted disposable VM; none of the archived
one-shot launch/cleanup helpers should be rerun as a verification step.
