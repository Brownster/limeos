# Resolving a `needs_intervention` job

A container or stack job that ends `needs_intervention` (or is left `running`/`verifying` by a core restart) keeps its resource locks in core and its `actions`/`stack_actions` row in the executor. Nothing releases them automatically, because the core cannot know whether the Engine call took effect. An administrator can settle the question on fresh evidence. Design and rationale: `docs/plans/2026-11-06-needs-intervention-unlock-design.md`. Storage and backup jobs are out of scope.

Resolution is bookkeeping plus read-only verification. It never starts, stops or restarts anything, and it never edits the job, plan, receipt or member results; `resolved` is an additional, terminal, lock-releasing state with its own record.

## Who may do it

Only an administrator, only on an interactive session. There is no grant, task scope, token or API key that carries it. The HTTP routes read the session cookie and nothing else; any `Authorization` or `X-API-Key` header is refused with 403. The decision also needs the usual `Origin` and `X-CSRF-Token` headers and the administrator's own password (fresh re-authentication; the repo's existing precedent is the observe-health enablement, which also takes `password` in the body). Wrong password is 401 `unauthenticated`: if `GET /api/v1/auth/session` still succeeds, the session is fine and the password was wrong. Failed password attempts are limited to ten per minute per administrator, shared with the observe-health enablement and inside the global login maximum; a correct password and a malformed body do not count (429 `overloaded` when exceeded). One administrator is enough; `accepted_divergence` does not need a second.

## Outcomes

| `declared` | Means | Allowed when | Reason |
| --- | --- | --- | --- |
| `verified_done` | The intended end state is present on every target | every target's `intended_state_reached` | 1-500 characters |
| `not_done` | Nothing took effect; every target still equals the plan | every target's `matches_expected` | 1-500 characters |
| `accepted_divergence` | Present state differs from before and from intended; the operator accepts it | always, while the job is awaiting resolution | 20-500 characters |

`not_done` only releases the lock; re-plan separately. A container that no longer exists proves neither `verified_done` nor `not_done`. Reasons may not have leading or trailing whitespace or control characters.

## HTTP API

Both routes are on `/api/v1/jobs/{id}/resolution`, where `{id}` is the 64-hex job ID of a container or stack job (the same ID the restart and stack job routes return). Errors outside the documented bodies use the usual `ErrorEnvelope` (`code`, `message`, `retry`, `audit_id`).

### GET

Fresh read-only inspection of exactly the job's targets on every call (503 `unavailable` when the executor cannot be read). Session cookie only; 403 for a non-administrator and for a job that is not a container or stack job; 404 for an unknown job.

```json
{
  "job": "e5…",
  "state": "needs_intervention",
  "kind": "stack",
  "operation": "restart",
  "targets": [
    {
      "expected": {"resource": "container:aaaa…", "image": "sha256:…", "started_at": "2026-10-05T07:00:00Z", "running": true},
      "measured": {"container": "aaaa…", "running": true, "started_at": "2026-10-05T07:05:00Z", "image": "sha256:…",
                   "matches_expected": false, "intended_state_reached": true}
    }
  ],
  "evidence_digest": "9f…",
  "allowed": ["verified_done", "accepted_divergence"],
  "members": [{"container": "aaaa…", "state": "verified", "error": null}],
  "pending": null,
  "resolution": null
}
```

- `kind`: `container` or `stack`. `operation`: `start`, `stop` or `restart`.
- `targets` are sorted by container ID. `measured.container` has no `container:` prefix. A container the Engine no longer has appears with empty `image` and `started_at`, `running` false and both flags false.
- `evidence_digest` and `targets` are present only while the job is awaiting resolution (`needs_intervention`, or `running`/`verifying` left by an earlier core process) and no record exists. Otherwise `targets` is empty, `evidence_digest` and `allowed` are null/empty.
- `allowed` lists the declarations the evidence supports now; `accepted_divergence` is always included while awaiting resolution.
- `members` are the executor's recorded per-target outcomes (`ExecutionState`), null before any receipt.
- `pending` is a recorded but uncompleted request (see below). `resolution` is the completed record (`job`, `resolved_by`, `declared`, `reason`, `evidence`, `evidence_digest`, `requested_at`, `resolved_at`).

### POST

Headers: `Origin`, `X-CSRF-Token`, `Content-Type: application/json`. Body, no other fields:

```json
{"declared": "not_done", "reason": "Checked docker ps by hand; jellyfin never restarted.",
 "evidence_digest": "9f…", "password": "the administrator's password"}
```

`evidence_digest` must be the digest the administrator was shown. Every success or conflict body is a `ResolveJobResponse`:

```json
{"status": "resolved", "message": "…", "resolution": { … }, "view": null, "blocking": []}
```

| HTTP | `status` | Meaning |
| --- | --- | --- |
| 200 | `resolved` | Executor and core are both released. `resolution` is the record. |
| 202 | `pending` | Recorded and held: the executor was unreachable or its answer was lost. Core retries on its own; repeating the identical POST is safe and idempotent. The job stays locked until it completes. |
| 409 | `stale_evidence` | The state changed since the evidence was shown, or the executor measured something different (the request is then withdrawn). `view` is fresh evidence; show it and let the administrator decide again. |
| 409 | `unsupported_declaration` | The evidence does not support `declared`. `blocking` lists the container IDs that differ; `view` has the per-target plan-vs-measured detail. Nothing is recorded. |
| 409 | (`ErrorEnvelope`, `conflict`) | Another request is open for the job, the job is not awaiting resolution, or it is already resolved. GET to see `pending` or `resolution`. |
| 400 | (`ErrorEnvelope`) | Bad body, reason rules, job ID. |
| 401 | (`ErrorEnvelope`) | No or expired session, or wrong password. |
| 403 | (`ErrorEnvelope`) | Not an administrator, bad CSRF/Origin, or a header credential was sent. |
| 429 | (`ErrorEnvelope`) | Password attempts rate-limited. |
| 503 | (`ErrorEnvelope`) | Evidence could not be gathered (executor unreachable before anything was recorded). Nothing was recorded. |

A browser or proxy deadline does not interrupt a decision that was already recorded; the executor step and completion continue, and GET shows the outcome.

## Finding other principals' stuck jobs

`GET /api/v1/container/jobs` and `/api/v1/stack/jobs` list only the caller's own jobs, so an operator's stuck job would be invisible to an administrator, while its locks block everyone. `GET /api/v1/jobs/needs-attention` is the administrator's read-only view of all of them. Same credential rule as the evidence route: session cookie only, any `Authorization` or `X-API-Key` header is 403, 401 without a session, 403 for a non-administrator (who keep seeing only their own jobs). A GET, so no CSRF.

```json
[
  {
    "job": "e5…",
    "owner": "bob",
    "kind": "container",
    "operation": "restart",
    "state": "needs_intervention",
    "resource": "container:aaaa…",
    "targets": ["container:aaaa…"],
    "created_at": 1762400000
  }
]
```

- Lists `needs_intervention` container and stack jobs of every principal, plus `running`/`verifying` jobs from an earlier core generation (the same test that lets `POST` accept them). Live jobs of the current core are never listed.
- Oldest first, at most 64. `owner` is a username. `resource` is `container:<id>` or `stack:<name>`; `targets` are the `container:<id>` resources the action touches. `created_at` is the job's first recorded event, in Unix seconds (0 if none survives).
- It carries no evidence. Use `GET`/`POST /api/v1/jobs/{id}/resolution` on the listed `job`; both already work for any principal's job when called by an administrator.
- The UI merges this list into the container and stack "Needs attention" banners for administrators, once per job, with `owned by <username>` on other principals' jobs. Operators and viewers see the banner for their own jobs only, as read-only text.

## Order and crash safety

1. Core inspects the targets read-only, checks the digest and that the evidence supports `declared`, then durably records the request (`job_resolutions`). The job stays locked.
2. Core sends `ResolveAction` to the executor. The executor re-inspects, requires the same digest, and moves its own row to `resolved`, freeing its unique index. Idempotent by job.
3. Core stores the executor's confirmation, sets the job `resolved`, releases the locks and writes the audit event in one transaction.

Only a definite executor refusal (`conflict`, `not_found`, `forbidden`) withdraws the request. Anything else, including a lost connection, leaves it recorded, because the executor may already have freed its side. A retry loop re-sends the same request and completes it: once at core startup and then on the dispatcher tick (about every two seconds, at most once per 30 seconds per job, ignoring requests younger than five seconds so it does not race the request's own handler). If the recorded administrator has since lost the role the retry cannot withdraw a refused request; restore the role or check the logs (`job resolution stays pending`).

Audit events (actor is the administrator): `job_resolution_requested {declared}`, `job_resolution_withdrawn`, `job_resolved {declared}`.

## CLI

For when the web UI cannot be reached. Root only, over the core's root-peer socket, like `enroll` and `migrate import-users`. Core does not accept these requests from any other peer. Being root on the host is the re-authentication, so no password is involved.

```
limeosctl [--socket PATH] job show-resolution JOB_ID --admin USERNAME
limeosctl [--socket PATH] job resolve JOB_ID --admin USERNAME \
    --outcome verified-done|not-done|accepted-divergence \
    [--reason TEXT] [--evidence-digest HEX]
```

A root peer has no principal of its own, so `--admin` names an existing, currently administrator user, and the decision is recorded under that user (a non-administrator or unknown name is refused). To record the channel too, core appends ` [limeosctl, root peer]` to the stored reason. The reason must satisfy the same rules on its own, before the suffix, and at most 470 characters are safe.

`job resolve` fetches fresh evidence, prints it (per target: planned, measured, flags, executor outcomes, and the digest), then submits the decision bound to that digest. To decide on what an earlier `show-resolution` displayed, pass its `--evidence-digest`; if the state has changed, nothing is sent. The reason is `--reason TEXT` or, when absent, stdin (up to 2 KiB, trimmed). Exit status is 0 only when the job was resolved; `pending`, stale or unsupported outcomes print the explanation and exit 1.

## Operator procedure

1. Open the stuck job (`needs_intervention`). Read the executor outcomes and the plan-vs-measured evidence per container. Look at the host yourself (`docker ps`, `docker inspect`) when in doubt.
2. Choose the outcome the evidence supports: `verified_done` when every target is in the intended end state; `not_done` when every target is exactly as planned; otherwise `accepted_divergence`, writing what you found and why the present state is acceptable.
3. Submit with your password (UI) or `limeosctl job resolve` (host). On `pending`, wait; check again with GET or `show-resolution`. On `stale_evidence`, re-read the new evidence.
4. A resolved resource can be planned again immediately; the new plan snapshots the present state.

Needs the executor to run with write capability for the job kind (the same `allow_*`/`allow_stack` ceiling that let the job run); otherwise `ResolveAction` is refused and the request is withdrawn.
