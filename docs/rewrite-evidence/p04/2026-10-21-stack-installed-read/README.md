# Installed stack reader: build and refusal evidence

Source `54022bb2b9fe3a2f7570e4f00b9de07a1e9e334f` produced fresh AMD64 Debian
release binaries and standard/shadow packages. Signed standard installation
passed in a disposable Debian 12 VM. **Genuine stack collection still fails**;
this packet gives no installed B2 acceptance or Pi cutover credit.

## Candidate and build

The offline Debian builder used image
`sha256:af684f63adae359e95f9082cc3235bcc76208baaa9cd1a48b553784e226d5ba5`,
Rust 1.88, one Cargo job, a 2 GiB container memory limit and no network. All five
binaries came from the same clean source, including the new 1,229,664-byte stack
worker. Source hashes matched before and after compilation and on both sides of
the build bind mount. The owned container exited zero without OOM and was removed.
Sampled host available memory stayed above 3,689,598,976 bytes during this build.

Actual `packaging/build.py` assembled both profiles as `0.4.5+stack.1` on host disk.
The signing container verified the immutable Debian package hashes before
constructing the disposable signed repository. No old CI binary payload was
substituted. `candidate/inputs.json.gz` records every binary, package and frontend
asset hash. The complete packages remain in the local ignored cache; this packet
contains metadata and public repository signatures, without private signing keys.
The local test key expires on 21 October at 14:50 UTC; it is not a release key.

Package assembly used a 128 MiB signing container, but the outer assembly command
did not use the host-memory sampler. The guarded build and VM claims below do not
extend that sampling claim to package assembly. Builds, caches, packages, logs and
VM scratch stayed on repository disk.

## Terminal VM attempts

| Attempt | Guest RAM | Result |
| --- | --- | --- |
| `stack-installed-vm-1` | 1,024 MiB | Signed installation passed, then the sampled 3 GiB host-memory guard stopped the owned VM; exit 143 |
| `stack-installed-vm-2` | 768 MiB | Signed installation passed; fresh-source predicate timed out with both sources unavailable; exit 1 |
| `stack-installed-vm-3-trace` | 768 MiB | Same refusal; diagnostic trace verifies sealed guardian/worker execution and normal worker exit; exit 1 |
| `stack-installed-vm-4-file-trace` | 768 MiB | Same refusal; broader file trace identifies the two operations below; exit 1 |

Every attempt used the same candidate and preserved production source hashes.
Attempts 2–4 had no host guard stop. The 768 MiB runner changes only the disposable
guest's RAM from the tracked runner's non-build default; service limits and
assertions are unchanged. Each owned VM was torn down. The retained consolidation
audit also confirms all local and live remote branch tips are ancestors of this
clean main source, with eight unchanged dirty worktrees preserved separately.

The standard package keeps the reader dormant after installation, provides all
five root-owned immutable binaries, and enables the independent stack-read
ceiling in fresh policy. The fixture enrolls an authenticated local administrator,
creates a protected catalog registration and starts the unchanged reader unit.
It uses a real packaged Docker daemon with its own Unix listener; PID 1 socket
activation is excluded. The API keeps returning unavailable sources without
inventing stack counts.

## Observed refusals

Attempt 4 observes these failures in two successive sealed workers:

```text
open("/proc/1/ns/mnt", O_RDONLY|O_CLOEXEC) = -1 EACCES
openat2(..., "/run/docker.sock", ... RESOLVE_NO_MAGICLINKS|RESOLVE_NO_SYMLINKS, 24) = -1 ENOSYS
```

The catalog's `compose_files::host_root` opens the current namespace, then fails
to open PID 1's namespace under the installed empty capability set. The Engine
reader fails its protected socket open before transport. The exact origin of
`ENOSYS` still needs an outside-unit kernel comparison and inspection of the
installed syscall filter; this trace alone does not distinguish those causes.

The preceding trace confirms immutable image sealing, guardian admission,
descriptor limit 256→226, worker bootstrap, parent-death setup, worker resource
checks and normal exit/reaping. These observations locate collection refusal
after successful launch. They do not establish the death/deadline fault matrix.

The service remains alive with the existing 64 MiB, 256-descriptor and sixteen-task
limits. Attempt 2 records cgroup memory peak 4,812,800 bytes with zero memory-limit
or OOM events. That is a rejected early collection, not successful maximum-layout
resource qualification. Trace observers ran outside the target service; traced
attempts are diagnostic runs rather than performance acceptance.

## Reproduction limits and next work

`manifest.json` binds 61 compressed records, exact build/preparation helpers,
per-attempt commands, source hashes, limits, exit statuses and failure diagnostics.
Each compressed entry records both compressed and original SHA-256. The final
`artifacts.sha256` covers the complete packet. Trace capture retains selected
refusal lines and bounded tails, rather than the full trace file.

Attempt 1's exact cache-only guest script was not separately frozen. Attempts 2
and 3 retain post-run script snapshots. Attempt 4 hashes and copies its inputs
before execution. The common runner and all assertions remain available in the
snapshots. These limits are explicit in the manifest.

Next, establish a protected host-view proof compatible with the reader's empty
capability set, and determine why the installed socket open returns `ENOSYS`.
Retain namespace/source identity, no-symlink traversal, peer validation and all
resource limits. Rebuild a distinct immutable candidate after any correction;
do not reuse this candidate as a passing result.

Positive collection, replicas/stopped/one-off layouts, refusal/recovery, installed
browser ownership, worker/guardian/manager faults and complete resource acceptance
remain open. Shadow installation, native ARM64, A1 admission, B1 observation
production, whole-stack effects and P05/P06 migration/return gates are also open.
No production Pi, frozen Python application or production storage was touched.
