# ObserveHealth self-enablement

The fixed authenticated endpoint lets an eligible current user add their own
literal `AutomationManage(automation:observe-health)` grant after verifying
their password again. It does not register schedules, run observations or grant
authority over other resources.

```http
POST /api/v1/auth/capabilities/observe-health/enable
Origin: https://configured-limeos-origin
Cookie: __Host-limeos=<current-session>
X-CSRF-Token: <current-session-csrf>
Content-Type: application/json

{"password":"<current-password>"}
```

Only `password` is accepted, with a nonempty 1,024-byte bound. The route rejects
GET and unknown fields, including a claimed user, role, grant, revision, clock or
verification result. It uses the existing login attempt limiter and password
worker. Passwords are not included in responses or diagnostic messages.

The stored current identity must satisfy both
`IdentityManage(automation:observe-health)` and `HealthRead(host:system)` under
the existing policy. A role label alone is insufficient. Current enrollment and
validated legacy import already give Administrators explicit wildcard grants
for those two operations, so those accounts can use the endpoint without a
default-grant change. An account with custom/reduced grants must independently
meet the same current checks. Operator role alone cannot self-enable through
this Administrator-ceiling identity-management action.

A true change returns `{"enabled":true,"reauthenticate":true}`, clears the
browser session cookie and requires a fresh login. One transaction adds only
the exact automation grant, increments the stored grant revision and writes
one existing `GrantsRevised` audit. Other grants, role and active job/effect
claims remain unchanged. A supported legacy password hash can be upgraded in
that same transaction after genuine successful verification. The revision
invalidates old sessions and task authority under the existing rules.

If the exact grant is already present, the response is
`{"enabled":true,"reauthenticate":false}` and the cookie is retained. This
path is a read-only no-op: no hash upgrade, audit, revision or grant write. It
works with a query-only database and full audit storage, while still requiring
the live session, CSRF, password and both current prerequisite grants.

The handler starts one ten-second monotonic deadline before reading the body.
Core retains it across the actual bounded password verifier and both database
calls. Final mutation checks run inside the acquired IMMEDIATE transaction,
using a fresh trusted Core wall clock for session expiry. The no-op pins its
read snapshot before its clock sample. Current session/CSRF, password hash,
role, grant revision and grants must still match the authenticated stored
expectation. Detected expiry at the precommit check, authority changes,
grant-count/audit capacity failure or a database error rolls the transaction
back. A disconnected
HTTP client or lost response is not proof of rollback; a fresh login and retry
resolves whether the grant was committed.

The change uses the existing schema and closed operations, with no enrollment,
import or default-grant change. It does not expose the internal `revise_grants`
primitive or a general grant editor. The existing B1 foundation remains
undispatchable; a capability grant cannot enable a callback or effect. Genuine
producer provenance, original observation lifetime, cleanup, per-resource read
authority, schedule/status registration, timer/UI/delivery integration and
installed qualification remain separate open work.

The estimate is eight human hours, with scope review at twelve hours. P04 stays
at 320 hours with review at 480 hours. Measured human effort is unknown.
