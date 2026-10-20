# A1 installed namespace admission, 20 October 2026

This packet records **progress, not A1 acceptance or Pi readiness**. The final
source is `187b248`; production Rust is unchanged from the diagnostic executor
source `0934eb4`. The namespace correction is `509c05a`. Later source changes
repair and improve the installed-test fixtures.

## Runtime correction

Baseline `477b81f`, qualified by [CI 39429582703](https://github.com/Brownster/limeos/actions/runs/39429582703),
passed both native architectures but failed the installed mount scenario at nine
namespaces against the initial limit of eight. The exact CI packages and Debian
image reproduce that refusal locally. A bounded later census observes the host,
five ordinary systemd service namespaces and three LimeOS service namespaces,
with zero Engine containers. Those census facts are diagnostics, not admission
or failed-scan authority.

A1 now selects the trusted `InstalledMountV1` configuration profile: one root
plus four fixed three-component paths retain thirteen FDs. Deeper aggregate
paths refuse before host acquisition. General combined configuration keeps its
33-FD profile. Byte/report limits and protected reads stay identical. Sixteen
namespace anchors fit a **213-FD aggregate reservation**, within the unchanged
226 admission model and 256-FD/64-MiB/16-task service profile. All original leases,
complete userspace/task/namespace comparisons and poisoning rules remain.

The new executor passes the old namespace refusal and reaches host revalidation.
The latest completed A1 attempt returns HTTP 409 before plan creation:
`scan.recheck.user_tasks`, 116 tasks observed against 118 previously retained.
The disappearing task identities are **not established**. No comparison was
removed or refreshed to obtain success. Subsequent witness attempts did not
produce a completed A1 before/after process witness.

## Completed local checks

| Check | Result |
| --- | --- |
| Complete storage unit suite | 216 passed, none failed/ignored/filtered |
| Complete executor library suite | 44 passed, none failed/ignored/filtered |
| Storage compile-fail doctests | 13 passed |
| Reference suite | 61 passed |
| Affected storage/executor all-target Clippy | Passed with warnings denied |
| Rust formatting and diff whitespace | Passed |

The earlier focused 28 configuration, seven/eight namespace and ten refusal-probe
cases overlap these totals. Repeated runs add no unique coverage. A mistaken
binary-only filter ran zero admission tests; its raw output remains and supplies
no qualification. The final library run covers the actual admission module.
The first full reference run exposed an undeclared shared fixture import in its
mocked schema check; the corrected run passes all 61 cases.

Compilation, caches and general scratch stayed on repository disk. As in the
existing storage-fixture procedure, only tiny compiled-test fixtures use a
private runtime tmpfs directory, with 16-MiB per-file/sampled bounds. All 216
storage tests pass there, sampled data peaked at 57,496 bytes and cleanup was
verified. This does not qualify the existing Btrfs source-resolution limitation.

## Installed attempts and reproduction limits

Two offline Debian-compatible executor builds completed in the pinned existing
Rust 1.88 builder image, with one build job and owned-container removal. The
second verifies all 656 frozen input hashes **inside** the container before
compilation. Build records include before/after hashes, commands, image identity
and sampled host headroom. The builder is Docker Desktop's VM; its 2-GiB container
limit and the Fedora host's sampled 3-GiB stop threshold are separate bounds.
They do not measure the application's whole-service resource peaks.

Derived fixture packages reuse the unchanged CI Core, CLI, password worker, UI,
contracts and units; their complete binary source closures are unchanged. Each
replaces the executor with its recorded new hash and updates profile prose.
Exact package/binary hashes, guest scripts, public signing fingerprint and
package handoff comparisons are retained. These are isolated fixture artifacts,
not a newly published full release. Private signing/SSH keys are not archived.

The first package assembly and a later checksum handoff failed on Docker
Desktop's shared staging tree; subsequent reads matched host bytes. Immediate
comparison values were not saved, so their precise coherence mechanism remains
unproven. Final package assembly uses host dpkg and hands off complete immutable
archives with matching hashes. Preserve the original failed staging trees in
`.cache/a1-namespace-review/candidate-package-attempt-*`.

The userspace-witness run then exposed an earlier UID-refusal test race: a genuine
early peer close raised EPIPE before the assertion. The shared foundation probe
now handles buffered refusal/EOF/reset. Target tests use the **same allowed
inspection** for Core and nobody, requiring successful authorized reads before
and after denial. Unsupported Health previously returned forbidden for either
UID and could not establish this boundary. The updated probe passed installed
checks in the later 768-MiB run.

A 1024-MiB witness guest and a later 768-MiB guest were stopped by the unchanged
3-GiB host memory threshold. Their owned guests were removed. One earlier
768-MiB run failed at tune2fs without useful exit/stdout; diagnostic reporting
was improved, and the next run passed that command but later hit the memory
threshold. Its original cause is not claimed fixed. The last attempt passed the
predecessor installed claim/recovery/package checks but produced no complete A1
result. No failed or interrupted attempt is relabelled as an accepted mount.

Next qualification must finish the synchronous process witness on a host with
adequate headroom, identify the actual userspace change, and then pass the real
plan/approval/effect/independent-verification/replay/restart assertions. The
remaining crash, consumer/namespace, installed ARM64, peak-resource, performance
and P04/P05/P06 gates remain open. No production Pi access or push occurred.

`attempts.json` preserves terminal outcomes. Each attempt keeps raw output and
before/after input hashes; `manifest.json` separates native checks from installed
failure. Fixture package inputs and exact private wrappers are retained for
reproduction. Paths in captured helpers identify this workstation; adjust those
paths without changing the recorded input source when reproducing elsewhere.
`artifacts.sha256` covers every file in this packet.

Guest source directories are stored as verified `*.tar.gz` snapshots to avoid
duplicating the same fixtures in Git. Every member was compared byte for byte
before replacing the loose copy. Extract the appropriate archive to reproduce
its recorded guest directory. `artifacts-before-compression.sha256.gz` preserves
the original loose-file checksums; `compress-packet.py` records this final step.
