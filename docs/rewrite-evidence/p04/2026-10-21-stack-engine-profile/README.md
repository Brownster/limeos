# Installed catalog and Engine reads

Source `5ef88977a1a2a5d1e6fe1ad128695333b11b6188` passes nine installed checks
against a genuine root Docker daemon, protected catalog and authenticated Core
API. It corrects the Engine syscall conflict and namespace-root rendering
regression exposed by real containers. **Full B2, P04 and Pi cutover acceptance
remain open.**

## Corrections and source identity

The dormant reader uses `RestrictSUIDSGID=no` and explicitly permits `openat2`
alongside `@system-service`. The preceding diagnostic demonstrated that an allow
alone cannot override systemd's SUID/SGID filter. The [profile design](../../../plans/2026-10-21-stack-engine-profile-design.md)
records this explicit exception: the profile no longer prevents setting those
file bits. `NoNewPrivileges` prevents privilege gains at exec; it does not replace
the file-bit restriction. Empty capabilities, fixed read-only dispatch, host
view, source/socket traversal, peer checks and original lifetimes remain in
force. No write request, mount syscall or capability was added. Service limits
remain 64 MiB, 256 descriptors and sixteen tasks.

After that change, empty-stack collection passed, but starting containers made
catalog acquisition refuse. A small disposable kernel probe captured a valid
namespace mount with root `net:[4026532191]`. Linux [nsfs_show_path](https://github.com/torvalds/linux/blob/v6.1/fs/nsfs.c#L254)
renders this non-path form. The regression fails before correction. Other
mounts' root renderings now remain opaque; the selected `/` root still requires
its absolute root, held descriptor's mount ID/device, current/PID 1 agreement and
retained current NSFS owner. All input bounds remain unchanged.

| Focused check | Result |
| --- | --- |
| Profile regression | Fails on the old profile; all 65 reference tests pass after correction |
| Namespace-root regression | Fails before correction; 357 container unit tests and seventeen compile-fail doctests pass afterward |
| Strict container Clippy | All targets pass with `-D warnings` |
| Final release build | All five Debian binaries built from clean `5ef8897`; source hashes match before/after and across the container bind |
| Final packages | Standard/shadow assembled and signed as `0.4.5+stack.4`; standard installed in the disposable VM |
| Final installed run | Nine checks pass, including executable-bound capability witnesses |

Native checks ran before the source commit while HEAD was `a9921a0`; their actual
input hashes match the committed source and release build. This is focused
qualification, not a new full-workspace, frontend or ARM64 result.

The earlier `.3` candidate records package source `7fb7058` separately from
binary source `7f35dd5`. Its 460 application inputs matched the verified build,
and actual standard/shadow archive payloads were checked before signing. It then
exposed the namespace parser bug. The final `.4` candidate contains a fresh
release build after that Rust correction; both candidate manifests are retained.

## Actual installed scenarios

The final guest has 512 MiB RAM and two CPUs. The runner changes only guest RAM
from the earlier 768 MiB selection; application limits and assertions are
unchanged. A real packaged Docker daemon owns its Unix listener; PID 1 socket
activation remains excluded. The final run verifies:

1. Signed installation supplies the expected immutable binaries/unit and leaves
   the reader dormant until the fixture starts it.
2. Genuine catalog/Engine collection shows one registered empty stack; a foreign
   UID is refused on the same root socket.
3. HTTP/RPC read bursts preserve original source timestamps.
4. Two ordinary replicas, one stopped, and one running one-off reach Core/API
   with counts `ordinary=2`, `running=1`, `one_offs=1`, `one_off_running=1`.
5. A writable catalog refuses fresh acquisition, retains explicitly stale facts
   and withholds counts. Protected recovery requires a new sample.
6. A world-writable genuine Engine socket refuses fresh facts while catalog data
   stays fresh; protected recovery likewise requires a new sample.
7. Core restart recovers the installed read path.
8. Reader restart obtains a newer genuine cohort; HTTP waits for that cohort,
   rather than accepting the prior Core cache.
9. Cgroup and runtime-image checks pass for these layouts and refusal phases.

The recorded kernel memory peak across observed service instances is 5,996,544
bytes. Sampled cgroup memory peaks at 5,648,384 bytes, tasks at five and descriptors
at seventeen per process. Before/after restart records show no memory-limit, OOM
or PID-limit events. Sampling does not establish exact descriptor/task peaks.
Binary verification warms images and restarts reset cgroup accounting; these
readings do not establish cold or maximum-layout resource acceptance.

Thirteen stable executable witnesses cover manager, guardian and worker. The
observer checks an unchanged executable link around status/hash acquisition,
matches actual image bytes to the candidate and verifies zero effective/bounding
capabilities plus `NoNewPrivs=1`. A trusted PID-1 launcher still executing systemd
is recorded separately as setup. It remains included in cgroup resource samples.

## Preserved failures and remaining work

All terminal attempts remain in the packet:

- Profile and namespace-root regressions fail before their respective fixes.
- Candidate `.3` passes genuine empty collection but fails the running layout:
  catalog acquisition refuses while Engine facts stay fresh and counts are withheld.
- Its follow-up mount capture stops at the unchanged 3 GiB host-memory floor
  before completing diagnostics; it supplies no further collection result.
- A 384 MiB kernel-only probe captures the real namespace row without installing
  or qualifying the application.
- The first `.4` installed run passes eight functional checks, then fails a
  capability assertion on an initial `(executor)` sample parented by PID 1 during
  setup. The passing rerun binds samples to stable actual images and retains
  launcher observations separately. Production bytes and functional assertions
  are unchanged between these two runs.

Commands retain source hashes, exact helpers, exit statuses and sampled host
headroom. Work and scratch stayed on repository disk. Owned build containers and
VMs are removed. The Pi and frozen Python source are untouched. Public repository
signatures are archived; private signing keys and Debian payloads stay outside Git.
`manifest.json` binds original/compressed hashes, both candidates, source patch,
builds and frozen guest inputs; `artifacts.sha256` covers the final packet.

Maximum layouts, cold footprint, exact FD/task headroom, worker/guardian/manager
death/deadline/late-output barriers, installed UI principal ownership, shadow
installation, native ARM64 and Pi hardware remain open. Compose source inspection,
whole-stack effects, A1 admission, B1 production and P05/P06 migration/return work
remain open. Stream A's bounded storage parser also assumes absolute roots for
every mount; it needs its own consumer review/regression before any A1 claim.
