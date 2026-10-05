# ADR 0009: Reserve complete resource sets in the core writer

Status: accepted for the RW-040 foundation under the sequential rewrite plan.

The original core lock covers one primary resource per job. A storage operation will affect configuration, filesystem identities, mountpoints and dependent services. Acquiring those locks in separate writes would permit partial ownership; losing them on restart would allow competing effects while the first outcome remains unknown.

Schema 7 adds `job_resources` and `resource_locks` to the existing sole SQLite WAL/FULL writer. Queue insertion records the primary resource. Future operation code must derive and add every dependency in the same queue transaction from the approved, fresh evidence. HTTP, task and executor requests accept no caller-supplied claim list. Current container actions still derive only their full container ID; this change does not claim live mount dependency discovery.

Dispatch acquires the entire stored set in the job state transaction. An overlap aborts the whole update, leaving the contender queued with no locks or dispatch event. Unrelated resource sets remain independent. Required sets have at most 128 entries, each at most 768 UTF-8 bytes; active sets and job identity are immutable. The original primary-resource index remains as an additional guard.

Running, verifying, unknown and intervention states retain every claim. Expiry, grant revocation, cancellation requests and core restart do not release them. Only the existing operation result path can commit bound terminal proof, independent verification where required, the terminal state, audit event and release. There are no leases, fencing tokens, timeout unlocks or automatic lock repairs.

Migration copies every legacy primary resource and active lock without rewriting intent, approval or receipt bytes. Startup verifies the compiled dispatch guards, complete active ownership, primary requirements and foreign keys before changing generation or writing recovery events. Missing claims, orphan locks or altered guards stop startup with `state_not_durable`; a healthy SQLite page check alone is insufficient. A temporary in-memory schema supplies the compiled guard definitions during startup and is then discarded.

Qualification must cover atomic overlap, failed audit commits, bound proof and release, revocation/expiry, real process death, damaged claims and migration failure. Installed testing must retain the genuinely frozen 0.4.2 payload for upgrade, rather than assigning an old version label to current binaries. Repeat container interruption tests and all protected-target regressions against the new package.

The root operator target journal remains separate. Newly versioned human-approved storage jobs, fresh container/share/pool dependency discovery, shared executor dispatch and mount/fstab effects are still required before this foundation can coordinate storage writes. Version-1 preview approvals remain non-executable. RW-040 and its P04 defect rows remain open.
