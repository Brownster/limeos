# B1 installed browser journey

The ordinary AMD64 API/runtime scenario now passes with package
`0.4.13+coreopen.1`: the real Core timer observes a real container through the
protected reader, records one incident and recovery, and preserves history across
two Core crashes. Evidence commit `72c7938` does not qualify the installed UI.
This slice supplies that missing journey without changing application authority.

## Approach

Run Chromium, its Debian-packaged driver and Selenium inside the existing
disposable Debian guest. Serve the package's actual `/usr/lib/limeos/ui` assets
through Caddy using the package's documented HTTPS configuration and unchanged
`https://localhost` Core origin. The browser accepts this disposable guest's local
certificate; production certificate trust and browser sandbox hardening are not
acceptance claims. No routes, API responses, clocks, sessions or credentials are
mocked. Console enrollment creates a fresh account with an ephemeral password;
the browser performs login, password enablement and reauthentication normally.
Credentials remain in guest memory and private guest state, never evidence.

This retains the canonical runner unchanged, including its 1024 MiB guest, two
vCPUs, verified base image, temporary overlay, loopback-only SSH forward and
termination/cleanup path. An alternative host-browser SSH bridge would need new
origin, forwarding and coordination machinery. Repeating the existing closed
browser fixture would leave the installed integration gap open. The guest browser
adds test dependencies only; those are not application package dependencies.

## Scenario and independent checks

Install the exact signed standard package and required test dependencies. Keep
the default root reader policy and all package service units and ceilings. Create
one real, stopped, `unless-stopped` container with a fixed fixture image. Start
the real protected reader and prove readiness as Core's UID. Record exact
executable, UI and unit hashes and independently sourced database schema.

Use visible controls to log in, attempt schedule creation without the management
grant, enable scheduling with the current password, and log in again. The refused
attempt must leave durable history unchanged. Save failure threshold one through
the policy form, then create a one-minute observation schedule through its form.
Read-only SQLite snapshots independently bind the resulting schedule, due tuple,
job, result, alert generation and delivered in-app notification. Never insert,
settle, repair or replay a job directly.

Refresh the actual history panel until the first delivered incident appears.
Kill actual Core once and require automatic restart, unchanged original history
and the same browser session. Independently start the same fixture container;
wait for the next real minute's recovery and require exactly two occurrences,
jobs, results and notifications, with the same incident generation. Acknowledge
the incident and pause the schedule through the UI. Kill Core again, reload the
page and require the persisted pause, acknowledgment and exact history. Capture
desktop and narrow viewport screenshots after password fields have been cleared.
Coverage stays partial; unavailable host/storage/SMART/protection sources do not
become healthy. Existing inaccurate availability copy is recorded as a UI defect,
not treated as complete truthful-status acceptance.

## Failure, resources and evidence

All guest effects refuse outside root on hostname `limeos-p01-test`. Bound waits,
history, screenshots and the result file. Selenium uses the installed driver
explicitly and performs no driver download. Browser closure runs in `finally`;
the canonical runner owns guest cleanup even on failure. Preserve original failed
results and diagnostics, with no automatic mutation retry or authority repair.
Record guest OOM counters and actual browser/dependency versions. If the browser
does not fit, preserve the refusal/failure rather than increasing guest memory.

Use repository-disk scratch, serial phases, 4 GiB native and 4.5 GiB VM admission,
the unchanged sampled 3 GiB host stop floor and 8 GiB free-disk floor. Freeze exact
package, executable/UI and fixture source identities independently. Existing
package bytes may be reused only after proving unchanged application inputs;
new test files must be reported separately. Seal native checks, original VM
process identities, result/screenshots and cleanup evidence. No Pi, production
service, publication or external notification is involved. This journey alone
does not close B1, RW-045, P04, ARM64, upgrade/return, soak or P05/P06.
