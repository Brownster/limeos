# Current A1 candidate: initial-census refusal

The disposable AMD64 guest ran source
`22434b16631b55154a7c9f69a9d84db71159d50d` with fresh
`0.4.6+current.1` standard/shadow packages, all five Debian-built binaries and
the current 31-module UI. The build and VM each retain unchanged input snapshots;
all 687 recorded inputs match that commit. Installed binary hashes and
the schema-source hash join the guest's failure packet to the external build
provenance. The guest intentionally makes no unsupported whole-source SHA claim
from a version label alone.

## Observed result

The fresh repository signature verified before launch. The installed guest
reported schema 12 and passed the preceding storage planning, target preparation,
current-authority, claim/recovery and real process-death scenarios listed in
`source-match.json`. Those preceding pass labels do not make this failed suite
an A1 acceptance result.

A1 stopped at `pre_effect_api_refusals`: the first genuine mount-plan POST
returned HTTP 409 `conflict`, where the fixture required 201. Root logged
`scan.initial_collection` / `target.host_consumers.capture`, with no exact
task-difference record. No successful mount plan or approved-mount result is
credited. The later approval/queue, independent mount proof, replay and Core
restart cases were not reached.

The fixture's bounded udev setup wait observed its two initial workers exit
naturally and left udev running. The later post-refusal witness saw a disappearing
PID and an incomplete census. That later diagnostic is not the failed scan and
does not identify which initial comparison failed. Four unlabelled comparisons
in the current collector can explain this phase: kernel-task identity,
userspace task identity, process start/thread membership, or process membership.
The [next diagnostic change](../../../plans/2026-10-27-a1-initial-census-diagnostics-design.md)
retains refusal semantics while recording their actual compared values.

## Retained provenance and limits

The guarded VM exited 1 with **no host memory-guard trigger**, unchanged inputs
and at least 8,440,733,696 bytes available. The runner terminated/reaped its VM
and its private scratch directory is absent. The guest stopped the mount unit
without claiming that stopping proves effect quiescence or permits unmount;
disposal of the complete VM remained the cleanup boundary.

`checks/` preserves UI, packaging and failed-VM records/output. Build records,
package/fixture manifests, the parsed A1 failure, startup/service diagnostics and
the exact harness and compressed guest sources are retained separately.
`guest-sources.tar.gz` binds all 31 guest modules to the manifest;
`schema-source.rs.gz` preserves the supplied schema declaration source.
`source-match.json`
records the joins and negative result. `SHA256SUMS` covers every other packet
file. Private fixture keys, binaries, packages and VM disks are excluded; their
public hashes remain in the manifests. The disposable signing key expires on
3 November 2026 and is not a production signing key.

This run used a 384-MiB disposable guest and four empty virtual disks. Only guest
RAM differs from the tracked runner; assertions, service budgets, authority,
helper ownership and safety barriers are unchanged. The offline build kept its
existing 3-GiB host-memory and 8-GiB disk stop floors; the VM kept the existing
3-GiB memory floor and additional admission margin. Upgrade qualification was
disabled explicitly. No production Pi, host mount, release publication or push
was performed. Approved-mount, installed interactive UI, broader A1/ARM64,
A/B parity and P05/P06 cutover gates remain open.
