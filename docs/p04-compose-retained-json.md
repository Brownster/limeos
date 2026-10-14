# Retained Compose JSON source binding

`limeos_executor_container::compose_management::sources::inspect_bound_compose_json`
collects a selected management binding and admits supported JSON directly from its
held protected file descriptors. It takes only catalog path, stack ID and a
cancellation callback. The public factory requires real effective UID 0 before
callback, target, namespace or canonical Engine access. It is unregistered library
logic; no CLI/RPC/route, job, service or package activates it.

The owner retains the genuine [management catalog/file/Engine binding](p04-compose-management-binding.md)
and privately checked [JSON source value](p04-compose-json-source.md). It preserves
the original binding window, every child observation and spent ledger. It creates
no new lease, retained descriptor or management declaration. Callers cannot supply
body bytes, hashes, child owners, root/UID/clock/endpoint overrides or numeric caps.

A crate-private typed admission operation enters the file owner's existing atomic
guard. Fresh protected root/namespace/ancestor/name/identity/hash validation brackets
the body read and parse. Held RDONLY FDs are read positionally, with original identity
checks before/after and SHA256 matching the original observed bytes. Each buffer
requests only the admitted size plus one sentinel, at most 65537 bytes; each pread
requests at most 4096 bytes. Every request, including EOF, consumes the same original
1 MiB requested-read ledger. Growth, truncation or changed identity/hash refuses.

Temporary bodies drop after strict admission and never appear in the report. The
original file four-second wall/monotonic lifetime and two-second cooperative pass
remain in force. Fresh content checks and final protected namespace/root/name walks
preserve the existing no-follow, UID/mode, regular single-link and ambiguity rules.
Parse/limit/freshness failure, cancellation, reentrancy, concurrency or unwind
permanently invalidates the file owner. Large otherwise supported files can exhaust
the original read budget through repeated verification; no larger budget is granted.

Parsed raw byte counts and hashes must match the retained ordered operator/managed
observations and selected presence. Every effective source service must be registered.
Every selected full Engine member must name a service defined in that source. An
observed registered orphan refuses the whole composition. Future allowed names
without a source definition or observed member grant no action.

Original full container identities, replicas, one-offs, stopped/mountless members,
unrelated projects and non-Compose rows remain in the complete parent report.
Zero selected members is an observation, without safe-absence or deletion authority.
Association verifies selected declared file/project/service identity; it does not
prove that deployed image, mount, port, numeric user or network settings match the
parsed project. Image reference digests and Engine image IDs are separate facts.

`BoundComposeJsonEvidence::snapshot()` returns one borrowed serialize-only report
containing the original binding and source summary. No child-owner getter or snapshot
clone is provided; reports/owners cannot deserialize. Count the whole compact report
against 256 KiB while retaining individual 80 KiB binding/192 KiB source limits.
Combined excess refuses. Retained FDs remain at most 38. Temporary body buffer
requests total at most 131074 bytes, plus bounded parser/model allocations. These
are admission limits, not peak allocator/RSS or combined-worker resource proof.

Collection revalidates before returning. Subsequent async checks bracket freshly
admitted held bodies with genuine parent catalog/file/Engine checks, compare original
source fingerprints/projects/declarations, repeat service association and check the
original window/report. Child times/budgets never renew. Dropped async validation
closes its active Engine connection and invalidates the new owner. Failure, timeout,
cancellation, reentrancy, concurrency or unwind permanently poisons it. A still-valid
child binding cannot restore the failed source owner. Borrowed reports remain past
facts after expiry/failure. Synchronous kernel work needs independent supervision;
these checks do not create a writer transaction.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-retained-json/README.md)
uses private scratch-root/UID/clock fixtures through production traversal/read/hash
code and genuine existing owners from kernel-authenticated owned Unix listeners.
See the [design](plans/2026-10-14-compose-retained-json-design.md). Public root-positive,
foreign namespace, independently identified daemon/physical host and installed
resource qualification remain open. The existing six pinned offline parser checks
cover their earlier supported fixtures, with no daemon effects.

The [lifecycle proposal composition](p04-compose-lifecycle-proposals.md) consumes
this owner to bind exact compiled candidate bytes and source changes to the original
complete membership, with no additional lease or execution authority.

General YAML/.env/includes/interpolation, actual deployed configuration equivalence,
complete physical consumers, principal/fresh executable approval, acquired claims/
locks, writes/recovery, verified shutdown, worker integration and independent review
remain open. Core/Store, generated contracts, dependencies, service guards and the
frozen signed ARM64 candidate stay unchanged. No Pi, SSH, production Docker, raw
block probe or guest was used. Durable principal/approval and claim/lock integration
through recovery, with fresh checks immediately before effects, remain subsequent work.
