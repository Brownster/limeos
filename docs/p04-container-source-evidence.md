# Retained container source evidence

The RW-040 storage library now resolves selected Docker sources to retained host descriptors. [The decision record](adr/0011-p04-container-source-descriptors.md) states its trust boundary and limits. [Validation](rewrite-evidence/p04/rw040-source-descriptors/README.md) binds the implementation to its tests. It adds no HTTP route, RPC request or executor command; the existing [declaration endpoint](p04-container-dependencies.md) keeps its existing behavior.

`inspect_container_sources(&inventory)` requires UID 0 in the host mount namespace and a fresh validated `ContainerStorageInventory`. It returns `ContainerSourceEvidence`, which owns the root/source descriptors until dropped. `snapshot()` borrows its report; `revalidate()` checks the retained identities, both source names, full raw mount table, host view and both age clocks again. A serialized report cannot reconstruct this object or authorize an operation.

The report binds the full declaration digest, host root mount ID and raw mount-table digest. Each source records its container, destination, declared/resolved paths, inode, device number, kind and possible mount IDs. Stopped containers and named volumes retain their declaration binding. Directory candidates include their current mount and every mount below the resolved path. Selected mount rows preserve filesystem roots and device numbers so aliases and nested filesystems remain visible. They are conservative host-view candidates; UUID, backing topology and running-container namespace binding still follow.

The resolver opens components without following links in the kernel. Ordinary link targets are read through retained descriptors on an explicit local-filesystem set and expanded with bounded steps. Every prefix must have directory semantics, including before `.` or `..`. Magic links, remote/FUSE/autofs filesystems, special-file sources, missing paths, stale/replaced sources, excessive output and failed cached lookup refuse the entire read. Link metadata reads may update atime; regular file data is never opened for reading or writing.

| Bound | Limit |
|---|---|
| Work per collection/revalidation | Two-second cooperative budget; future service process deadline required |
| Declaration age | Five seconds, with remaining monotonic lifetime |
| Containers / selected sources | 64 / 256, preserving existing admission rules |
| Links / walk steps per resolution | 40 / 512 |
| Declared, resolved and link-target bytes | 512 each |
| Aggregate candidate mount references | 512 |
| Raw mount table / report | 1 MiB / 48 KiB |

The local tests compare actual kernel identities, reject magic/cyclic/oversized links and FIFO sources, observe replacement/deletion/rename/retargeting, and preserve case and spaces. A failing pre-commit test exposed incorrect `file/../other` handling; the final implementation refuses it as the VFS does. Raw mount digests also detect parent/propagation/option changes that the older selected-row parser intentionally omits.

Next integration must use bounded worker admission, reacquire Engine evidence, join fresh UUID/topology observations, inspect running-container namespaces and propagate pool/protection/share dependencies. It must then derive shared claims, bind approval and independent root policy, and revalidate at effect time. Mount/fstab writes, unmount and runtime-loss shutdown remain disabled. DSK-001 and RT-001 remain open. This library slice has local workstation proof; installed/native ARM64 qualification is recorded separately.
