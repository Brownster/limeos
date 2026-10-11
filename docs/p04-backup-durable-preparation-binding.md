# Durable private backup preparation binding

The Store can now persist an approved private-preparation proposal and return the same queued job after a lost response or restart. It records trusted binding facts and logical resource requirements. The archive library continues to own actual admission, custody, verified staging and private preparation.

## Closed records

`limeos_domain::backup_jobs::CustodyBinding` preserves the existing six-field encoding. The archive crate re-exports it at `limeos_backup_archive::custody::CustodyBinding`. Moving this pure value changes neither custody identity encoding nor archive recovery policy.

`BackupPreparationBinding` contains the custody binding, the genuine preparation plan identity, ordered selected resource IDs and a trusted workspace/policy identity. Identifiers are bounded before serialization; selections contain 1–64 unique IDs, each at most 128 bytes. `BackupPreparationJobPlan` adds the principal, saved grant revision, operation and plan IDs, version and checked 300-second validity interval. Saved bodies are capped at 16 KiB. The Store limits unconsumed plans to 1,024.

The caller supplies no path, opaque owner, destination override, physical claim or lock. A trusted coordinator must derive the full binding from genuine current library results and complete current policy, limits, selection metadata and protected workspace facts. The production source of those trusted facts remains unfinished.

## Store operations

| Method | Result |
| --- | --- |
| `plan_backup_preparation` | Save a bounded proposal with its complete digest |
| `backup_preparation_plan` | Read the caller's proposal under current authorization |
| `approve_backup_preparation` | Check current binding, revision and expiry; rotate a random approval |
| `queue_backup_preparation` | Consume the approval and atomically save the job, resources and audit |
| `replay_backup_preparation` | Return the exact previously committed job using database reads |
| `backup_preparation_job` | Read the caller's saved job under current authorization |
| `cancel_backup_preparation` | Cancel an unconsumed proposal |
| `cancel_backup_preparation_job` | Cancel the caller's queued job |

Every operation requires current administrator/StorageManage authority. Exact consumed replay compares the whole saved proposal, principal, idempotency key and consumed approval hash. It remains available after expiry and restart to a currently authorized caller, without reading archives, refreshing authority or changing database state. Current revocation remains effective. Saved revisions stay valid history; new approval or queue requires the current revision.

Queueing derives exactly `backup:preparation`, `backup:custody:<record_id>` and `storage:configuration`, sorted. These are logical requirements. Queueing acquires no locks and proves no physical alias or reservation. Approval consumption, job insertion, all requirements and audit commit in one transaction; any failure rolls all of them back.

## Startup and recovery boundary

Schema 9 adds the bounded plans table and compiled immutable/state guards. Startup checks guards, metadata sizes, canonical records, approval/job links, saved revisions and exact resources before recovery writes. Migration, startup generation and recovery audit share one transaction. A failed audit or integrity check leaves an old schema-8 database unchanged. Historical compiled-schema fixtures preserve sessions, grants, storage/container plans, approval and receipt payload bytes, resource rows and held uncertain locks. Preserved receipt bytes prove migration compatibility, not executor authentication.

The new intent permits only Queued and Canceled. Generic Store transitions allow Queued → Canceled, and SQL/startup guards reject dispatch or completion. Existing executor candidate queries exclude it. There is no public route, executor RPC, claim, receipt or durable preparation-success transition.

A coordinator must recover the exact retained archive, re-admit its actual bytes under the full current policy, stage a fresh verified catalog and derive a matching current preparation plan before private work. Database metadata cannot reconstruct a library owner or authorize an archive. Custody published before a failed database commit is an inert orphan; no directory scan adopts it. Private preparation remains ephemeral after a crash, including when a queued binding survives.

Production inventory/limits, trusted selection UID/GID/modes, complete protected workspace/ancestor/ACL/mount identity, same-UID exclusion, quotas and physical dependency claims remain integration work. Tests use genuine archive bytes with explicitly synthetic private policies. Installed coordination/recovery, live restore, BKP-001, P04 effects and cutover remain open.
