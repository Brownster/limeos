# Native health schedule management, 21 October 2026

Source `3e494a0` mounts the registered saved-schedule flow in the authenticated
dashboard. [Source-bound local evidence](rewrite-evidence/p04/2026-10-21-health-schedule-ui/README.md)
includes the actual bundle and closed browser fixture. This is the native UI seam
for existing metadata; it does not finish B1's observation/alert/notifier runtime.

“Manage health schedules” opens independently of Overview availability. The
schedules-only controller reads no proposed alerts/notification routes. Users
can request timing creation, a UTC maintenance window, pause and resume. Current
server grants remain authoritative; displayed roles grant nothing. An optional
current-password POST requests fixed health-scheduling self-enablement. Changed
permission clears owners before sign-in again with an explanation. Inputs clear
before awaiting, and passwords never enter controller state or browser evidence.

Owner keys bind principal, grant revision and CSRF. Layout cleanup aborts reads
and writes on close/replacement; duplicate and reentrant confirmation is bounded.
No failed/unknown POST is retried automatically. Unknown outcomes remain visible.
Unavailable data disables changes, forbidden data clears private rows, and saved
status says automatic observations are not running. Compact references retain
full action labels; mobile scrolling stays within the table.

TypeScript, production build, all twelve frontend test files and 51 focused
cases pass. The core closed-browser scenario verifies permission refusal,
enablement and reauthentication, cross-midnight creation, revision-bound pause
and resume, unavailable/forbidden sources and desktop/mobile render. The expanded
already-enabled/close-pending-request browser follow-up was stopped by the host
memory guard before producing results; its unit coverage is not retagged as
browser proof. No real API/Store, installed package, native ARM64 or Pi acceptance
is claimed. The existing prepared packages contain the earlier UI, not this panel.

Continue with the remaining browser ownership cases when headroom permits, then
genuine API/UI/package qualification. B1 still requires the registered protected
observation producer, durable completion/alert/outbox transactions, notification
delivery and installed concurrent/restart/failure/idle-write evidence. Schedule
creation still starts no timer or effect. A1's current fixture work, wider A/B
parity and P05/P06 cutover gates remain separate. Nothing was pushed or installed
on a production host.
