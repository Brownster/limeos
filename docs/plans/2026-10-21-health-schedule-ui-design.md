# Mounted health schedule management

The registered production API already provides owner-scoped schedule pages,
timing creation and revision-checked pause/resume. Fixed self-enablement verifies
the current password and existing IdentityManage/HealthRead grants before adding
only `automation:observe-health`. A changed grant revision requires sign-in
again. The dashboard mounts none of this flow; the existing three-page
AutomationPanel also requests unregistered alert/notification routes.

Mount a collapsible Health schedules workspace independently of Overview
availability. Use the existing restrained dashboard typography, tables and lime
controls. Reuse AutomationView and its controller with an explicit schedules-only
mode: no requests or mutations for alerts/notifications. Keep the existing full
leaf mode for later B1 registration. Show saved/paused/suspended state truthfully,
including the fact that automatic observations are not running yet.

Expose create and pause/resume as requests whose current grants are checked by
the server, without deriving authority from a displayed role. Include a separate
optional password confirmation for the fixed self-enablement endpoint. Send only
password and CSRF; no subject, scope, operation or caller clock. Clear the input
before awaiting the request and retain no credential in controller state.
Successful enablement with a new revision clears private owners before returning
to sign-in with an explanation. An already-enabled response refreshes committed
schedule state. No failed or ambiguous POST is automatically retried.

One owner is keyed by principal, grant revision and CSRF. Dispose read/write
controllers in layout cleanup on close, sign-out or replacement. Serialize
enablement against schedule writes and block duplicate same-frame submissions;
late results cannot expire a newer session or refresh its pages. Focus/reconnect
refreshes remain GET-only. Permission failure and unknown outcomes stay visible.

Validate fixed request shape, response bounds, 401/403 preservation, duplicate
admission, disposal/reentrancy, stale replies and schedules-only traffic. Check
the mounted flow in a local browser fixture, including enable/re-authenticate,
create, pause/resume, source errors and ownership replacement. This completes
the native UI seam for existing metadata only. B1 still needs the durable
observation producer, alert/outbox registration, restart/deduplication and
installed API/UI qualification; it does not close B1 or P04.
