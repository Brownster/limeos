# Retained combined read collection

`limeos_executor::read_collection::collect` composes the genuine read-only
configuration, Engine, process, physical-storage and destination owners with the
configured pool/SnapRAID/share graph. This is an unregistered library entry:
executor commands, RPCs, services and the standalone probe remain unchanged.

```rust,ignore
pub async fn collect(
    paths: ConfigurationPaths<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<CollectedReadReport>;
```

The future launcher must create an independently supervised fresh worker and
set that worker's own soft descriptor limit to exactly 226 before this call.
The collector reads the limit and its own counters through
[resource admission](p04-combined-read-budget.md); it never changes the caller's
limits. No current launcher supplies that guarantee.

The four configuration paths are trusted package/runtime selections, using the
existing protected-file checks. All four are required, including explicitly
empty configurations. The Engine endpoint is fixed to `/run/docker.sock`, with
root socket/peer authentication. No public UID, proc/sys root, clock, provider,
numeric profile or wire selector is added.

Collection enters Configuration → Engine → Processes → Physical → Destinations
→ Graph budget phases before their operations. Each uses its fixed CombinedV1
profile or the existing closed destination limits. Running process bindings come
directly from the genuine Engine owner. Whole-owner failure aborts the request;
no intermediate body or partial successful inventory is returned.

All owners stay live through final reporting. Destination evidence borrows both
physical and process parents. The configured graph borrows checked declarations;
it does not discover actual Samba/pool state or complete physical consumers.
Final validation performs:

1. Configuration name/handle/content comparison.
2. Retained Engine fresh GET comparison.
3. Destination validation, including both original physical/process parents
   before and after path comparison.
4. Retained Engine fresh GET comparison again.
5. Configuration name/handle/content comparison again.

The sequence preserves original leases, declaration times and lifetime Engine
byte allowance. It never reacquires an owner to renew its age. These completed
comparisons do not constitute an atomic snapshot of a changing host.

One four-second monotonic window starts before admission. Boundary wall checks
reject backward/negative observations and more than four elapsed wall seconds.
Cancellation and expiry are checked before and after operations; the existing
cancellation-aware adapters also poll the window. A refused window stays failed.
Synchronous work can block beyond the window: the collector refuses after it
returns, without claiming an independent kill deadline or cancellation latency.

`CollectedReadReport::as_bytes()` borrows one complete JSON body, serialized
directly from owner snapshots through the fixed 256 KiB resource-session writer.
Fields are `version`, `collection_started_at`, `budget`, `configuration`, `engine`,
`processes`, `physical`, `destinations` and `configured_graph`. The collection start
is a request boundary; each original observation time remains in its own report.
The window is checked again after serialization before exposing the body. The
opaque bytes cannot be imported as retained evidence or effect authority.
`Refusal` identifies the stage and typed adapter/resource/window failure.

The local tests exercise the private window/gates and a genuine public nonroot
refusal in an owned test child. Only that child lowers its own limit; its parent's
limit is checked unchanged. Tests provide no root-positive composition proof.
Existing adapter fixtures remain the evidence for each closed profile. See the
[design](plans/2026-10-13-combined-read-collection-design.md) and
[local acceptance packet](rewrite-evidence/p04/2026-10-13-combined-read-collection/README.md).

Next is independent worker supervision: single-flight ownership through cleanup,
blocked-work deadline/cancellation, parent death and descendant termination/reaping.
The standalone probe's combined case stays blocked until that boundary exists.
Installed root/capability/namespace cases, resource headroom and performance remain
unqualified. Service guards, P04/defect/cutover gates, locks, claims, effects and
recovery are unchanged. This slice is outside the frozen signed ARM64 candidate.
