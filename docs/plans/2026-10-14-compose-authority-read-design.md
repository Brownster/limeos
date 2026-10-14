# Authenticated Compose review reads

Base: local main `e65a686`. Resolve the ordinary preview/scope inputs needed by
retained review subjects through actual current Store authority. Add read-only
session and task resolution APIs using existing schema 9, without migrations,
approval creation/consumption, jobs, claims, locks, recovery or executor activation.
Preserve crate dependency directions; persistence does not depend on executors.

Public inputs are session digest or task digest/transport peer UID/task ID, the
trusted current P03 catalog, ordinary expected preview and exact required scopes.
Production uses fixed current wall time, without a caller-selected clock. Private
test-time selectors exercise the same logic. Authenticate the session or task in
the actual database before processing caller project content. For tasks, require
the existing kernel-UID/task/generation/revision/expiry binding and every exact
scope. Caller principal, role, revision or scope declarations cannot become grants.

Use one deferred SQLite read transaction for a consistent authority observation.
Load the actual owned stored preview through existing compose_plan checks: durable
body/digest, cancellation, catalog, template, before/desired/impact, source hashes,
revision, role and grants. Compare the entire ordinary expected preview with that
bounded stored value before serializing or copying caller data. Derive exact scope
requirements from the trusted normalized catalog, and require equality with the
supplied one-or-two scopes; no omissions, additions, duplicates, reordered or wildcard
elevated scope. Repeat task checks for every derived scope with the same principal.

Recheck identity and stored-preview validity at the fixed current end time, reject
wall rollback, and record the actual database generation. A read transaction is an
observation, without a permission lease or writer transaction. Concurrent changes
after its snapshot require another fresh read; an earlier successful result cannot
authorize an effect. Session CSRF/origin checks for writes and fresh human executable
approval remain the responsibility of the later composition root. Task tokens still
cannot approve.

Return a private owned AuthenticatedComposePreview containing only actual current
principal, actual stored preview, exact requirements, generation, check time and
identity expiry. Expose a borrowed serialize-only observation bounded to 64 KiB.
No token/session digest/CSRF/password is retained or reported. Owner/report cannot
deserialize or supply a constructor. There is no approval, receipt or execution
method. The observation's borrowed preview/scopes fit the retained review subject's
existing check_current inputs, without adding an executor dependency or authority
shortcut. Re-resolve before and after genuine host validation in the future broker.

Integrate session resolution into the existing Core preview-read path and preserve
its original response. Cross-check stored revision/expiry columns with the canonical
plan, retaining a red/green API regression for inconsistent expiry metadata. No new
route or approval behavior is introduced.

Reads leave total_changes, audit events, generation, preview expiry/approval,
jobs/resources/locks and schema untouched. Current permissions are checked even for
an administrator; media/container grants do not imply deployment, and wildcard
elevated grants remain insufficient. A session cannot read another owner's preview;
an issued task with only stack scope cannot satisfy an elevated template.

Local actual SQLite/session/task tests cover ordinary/elevated parity, current
principal identity, exact preview/scope equality, malformed/oversized inputs,
unknown/revoked/expired session or task, peer UID/task/generation mismatch, missing
elevated scope, grant/role/revision changes, catalog/cancellation/body corruption,
rollback and late expiry, read-only behavior/restart and import refusal. Retain
complete diagnostics and exact-source Rust/Clippy/fmt/contracts/repository/local
ARM64/pinned parser evidence, source/artifact hashes and unchanged-schema proof.

Keep the reserved backup and external qualification worktrees untouched. Build/cache/
general scratch stays on disk; existing storage fixtures compile there before small
owned auto-cleaned tmpfs runtime. No Pi access before the operator's new 22:00
Europe/London window; this database slice needs local tests. Host use, if needed,
must preserve the frozen app, media/mounts and host health, with the earlier 07:00
return boundary. No cutover is authorized. Root broker/transport integration,
durable fresh approval, acquired locks/recovery, runtime/physical completeness,
shutdown/effect-time checks, public root/installed/resource/performance qualification,
independent review, P04 and cutover gates remain open. The signed ARM64 candidate
stays frozen.
