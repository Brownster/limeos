# Compose preview foundation

Package 0.3.2 adds protected Compose previews to the P03 lifecycle build. Authority schema v5 adds `compose_plans`; container receipt schema v3 stays unchanged. Deployment execution, catalog editing, live stack discovery and the deployment UI belong to P04. No Compose job or executor request exists yet.

Core optionally loads `/etc/limeos/compose-catalog.json` (`/etc/limeos-shadow/compose-catalog.json` for shadow) at startup. Every component of that absolute path must be root-owned, with no group or world write permission; symlinks and non-regular files are rejected. A missing catalog disables previews. Invalid, ambiguous or oversized catalogs fail closed and are preserved. No catalog is shipped by default. The file is limited to 64 KiB; the catalog has at most eight stacks, eight templates per stack and sixteen services per project. Restart core after replacing the root-controlled catalog.

[The generated schema](../contracts/generated/compose-catalog.schema.json) and [synthetic fixture](../tests/fixtures/compose-catalog.json) define the supported subset. Image references require SHA256 digests. Users retain numeric UID/GID. Ports, named volumes, absolute bind paths, devices, privileged mode and host networking are typed. Environment secrets, commands, interpolation and other unsupported Compose keys cannot enter this schema. The catalog describes a planning snapshot; it does not establish that paths or files still exist or that Compose will accept the combined project.

The caller submits only `stack` and `template`. Core selects and normalizes the catalog entry, fingerprints the catalog and template, preserves operator file hashes, computes changed services and names the separate managed file `limeos.override.json`. The diff includes full before/after service fields, including mounts, published ports, ownership and privileges. The source operator YAML, its formatting and anchors remain untouched. This milestone does not create the managed file either.

Previews require `deployment_manage` on `stack:<name>`. Every elevated template additionally requires `deployment_elevated` on the exact resource `stack:<name>:template:<id>:<canonical-project-sha256>`. Wildcards do not satisfy elevated authority. An operator may preview ordinary deployments; only an administrator with the exact additional grant may preview elevated deployments. Media-request and container-management scopes grant neither permission. The acceptance host supplies identity grants through offline root fixtures; deployment grant administration is part of the later identity UI.

| Method and path | Result |
|---|---|
| `POST /api/v1/compose/plans` | Persist a five-minute owner-scoped preview and impact diff. |
| `GET /api/v1/compose/plans/{id}` | Read the current preview without writing or extending expiry. |
| `POST /api/v1/compose/plans/{id}/approval` | Record fresh human approval of the exact stored digest. |
| `POST /api/v1/compose/plans/{id}/cancel` | Revoke the preview and clear its approval. |

All POSTs require a current session, expected HTTPS origin and CSRF token. Approvals are random opaque tokens whose hashes are stored in the authority database. Approval rechecks the catalog snapshot, grant revision, current permissions and expiry. Reads and approvals fail after cancellation, grant revision changes or a catalog change. Durable approval survives core restart while its catalog and five-minute window remain valid.

An assistant-shaped local caller uses `propose_compose` with a task token, task identifier and selection. The kernel UID, generation, expiry and every required scope are checked through the same policy and store as HTTP. Task transport has no approval method. Its token cannot substitute for a human session.

A preview approval cannot authorize a container job or deployment. P04 must issue a new executable operation and fresh approval after observing actual files, mount identities and combined Compose semantics. Its per-stack lock must cover managed-file edits and Docker operations; path traversal must use protected descriptor-based resolution. Directory removal remains gated on verified shutdown.
