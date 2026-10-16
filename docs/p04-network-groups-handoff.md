# Network-group observation handoff

Engineer B, 15 October 2026. Branch `engineer/p04-network-groups`, based on B2
`5e275ca`. Tested code: `c25dbe59a0c85d445061ad03b67629a7b28f9a90`.
This branch contains B2 and this read-only follow-up; catalog, image updates,
packages and prerequisites retain their separate branches/evidence.

October 16 continuation: the [egress comparison handoff](p04-network-egress-handoff.md)
adds pure host/provider address comparison and bounded supplied-response decoding
from the combined Stream B baseline. It covers part of integration requirement 5;
active protected probes and API/UI acceptance remain pending. This packet's
54-case qualification retains its original source and scope.

## Delivered behavior

`domain::deployment::networks::observe` produces bounded, deterministic shared-
network diagnostics from ordinary supplied observations. `executor-container::
deployment::networks::parse_inspect` selectively decodes one supplied inspect body
against an expected full ID. Neither function collects evidence or authorizes an
effect. There is no Docker request, process launch, filesystem read/write, clock
acquisition, probe, execution plan or receipt in these helpers.

- Preserve providers, members, stopped/one-off members, Compose identity and
  health state. Resolve container references by full lowercase ID or exact name;
  short hexadecimal ID prefixes stay unsupported. An ID/name collision remains
  ambiguous. Resolve service references only within the exact project, directory
  and ordered files, with one ordinary replica. Never prefer a running replica
  or search globally by service name.
- Keep missing providers separate from suggested providers. A single well-formed
  legacy dependency can annotate an orphan and degrade a suggested group. It
  never repairs the reference. Multiple dependencies, foreign projects and
  ambiguous replicas do not select a suggested provider. Self-reference, cycles
  and chained providers cannot become an observed-running group.
- Distinguish stopped, unhealthy, starting, missing health and missing network
  mode. A configured health-check object without a result is unknown; disabled
  checks also remain unknown because command/test contents are not interpreted.
  No configured check and no result is `not_configured`, not a health pass.
  Paused, restarting and dead providers are unavailable even if `Running` is true.
- Missing, partial, stale, future and different-Engine cohorts withhold rows and
  counts. Only a complete fresh empty cohort reports zero. Preserve original
  sample times and the inclusive 90-second freshness bound. Missing network-mode
  rows are counted explicitly; an empty groups list alone cannot prove absence.
- Require JSON objects at every consumed object position. Refuse duplicate
  consumed fields, wrong types, malformed selected data, ID mismatch and trailing
  bytes. Skip environment, other labels, health commands/logs and unknown payloads.
  These values never enter Node/summary/Debug/fixed errors. Valid deeply nested
  ignored data is skipped iteratively by the pinned serde_json implementation;
  raw byte admission still bounds it. This is selective metadata admission,
  not validation of the entire Engine response.

Bounds: 32 KiB per inspect body before parsing; 16 cohort nodes; 24 KiB aggregate
admitted text before report clones; 64 KiB streaming serialized report; 128-byte
names/container targets, 64-byte service names, 256-byte modes and 512-byte raw
dependency labels. B2's path/file/name bounds are applied to supplied Compose
identity. No cap silently truncates a cohort.

These are configured-reference diagnostics. Even `resolved`/`observed_running`
does not establish live process namespace equality, VPN use, leak absence,
reachability, source-file equivalence or execution safety. Plain node inputs,
complete/fresh metadata and serialized reports are never protected owners.

## Integration requirements

1. Export the new `networks` modules alongside B2 deployment leaves when accepting
   this branch. Existing serde/serde_json dependencies suffice; root manifests,
   lockfile, shared protocol, Store, route aggregation and shell are unchanged.
2. Extend the existing protected Engine collector within its ownership. Collect
   all running/stopped/one-off/standalone members, bind each successful inspect
   to its full ID and B2 identity from the same cohort, preserve original time
   and revalidate existing owners/ledgers before projection and delivery. The
   parser cannot prove success status, endpoint, root peer, completeness, time,
   source provenance or the association supplied by its caller.
3. Enforce collection-wide raw-byte, elapsed-time, memory, descriptor, concurrency
   and retained-response budgets before invoking this leaf. Per-body limits do
   not grant 16 independent reader/worker budgets. Overflow/failure/truncation
   makes collection partial/unavailable, never an empty successful inventory.
4. Register an authenticated cache-only read through the shared API and mount its
   UI. Resolve current grants; GET must not trigger collection, probing or writes.
   Show source/sample time, unknown counts/modes, orphan hints and actual health
   distinctions. Expiry, logout, principal/grant changes and 401/403 must clear
   retained views; stale observations cannot remain styled as current. No new
   API/client/UI code or acceptance is claimed by this branch.
5. Preserve the frozen explicit public-IP comparison as a separate pending parity
   row. It needs a bounded approved probe, trustworthy target identity and an
   honest unknown result on partial failure; a differing IP is not VPN proof.
   This slice performs no network or container-exec probe.
6. Group recreation remains pending. Reuse registered whole-stack execution and
   the common authority, exact source/resource claims, locks, verified provider-
   first outcome and recovery path. Cross-project references require all actual
   resources; service-name fallback and the frozen provider's labels cannot
   authorize arbitrary members. Never turn these diagnostic hints or a list of
   arguments into execution authority. No Compose-management library is modified.

## Focused evidence and remaining acceptance

[Packet](rewrite-evidence/p04/2026-10-15-network-groups/README.md): **54 Rust tests**
(23 topology, 19 inspect adapter, 12 existing B2 domain), strict Clippy, Rustfmt,
Ruff and harness formatting pass. Frozen source is read/hashed, never imported or
executed. The three inspect fixtures and seven health cases are synthetic ports
of the frozen provider/member/orphan/health behavior, not real Engine captures.

Red/green diagnostics record short-prefix and configured-health fixes, plus
Serde's sequence-as-struct behavior corrected with map-only decoding. An earlier
test wrongly expected valid ignored deep JSON to be refused; source inspection
showed iterative skipping, and the test now checks success without retention and
failure on malformed nesting. Intermediate diagnostics do not qualify final code.

No B2 reader/API/UI rerun, browser, live Engine/root-positive collection, protected
worker budgets, lifecycle/recovery, package/ARM64/Pi/performance/soak or cutover
acceptance ran. The integrator and qualification owner retain those tasks. This
does not close RW-042, the registered Stream B milestone, or P04.
