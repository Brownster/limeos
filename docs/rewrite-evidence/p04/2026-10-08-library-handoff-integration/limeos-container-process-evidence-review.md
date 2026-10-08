# RW-040 retained process evidence independent review

Verdict: **GO for integration of the library slice at the source below**, subject to the integrator's full merged-source gates. This review found no required correction. It does not qualify installed collection, authenticate Engine ownership or close a defect/phase gate.

## Reviewed source and scope

- Worktree: `/tmp/limeos-container-process-evidence`
- Branch: `engineer/container-process-evidence`
- Base: `d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4`
- Implementation and Rust tests: `ad5ff6e2ae75c98dc261c951b084a14f59c3b5e1`
- Clean reviewed handoff tip: `cb5aa668f48d29cf21ee1c90a63a39724cc3230a`
- Comparison main: `91909a40a3c0da7aa5da6b138889d4388969309c`

The diff stays within the published engineer brief's ownership paths. Shared exceptions add only `pub mod processes;` and the already pinned rustix event feature. No changed-path overlap exists with comparison main since the assigned base. The complete lockfile package records are unchanged: 227 packages, 211 external and 16 local. No RPC, route, command, schema, service policy, capability, Docker acquisition, effect or archive source changes enter this slice.

| Frozen input | SHA-256 |
|---|---|
| `crates/executor-storage/src/processes.rs` | `44e33fdf2ae5bf0c56c58af08ff4427424e51e80ccda073cb56bc191c0b10ab3` |
| `crates/executor-storage/src/processes/tests.rs` | `723c50154f0cb92b384031e3ffaba00fc35e4a8ac16d2ece9ac1c479eab38f94` |
| `crates/executor-storage/Cargo.toml` | `645dac84c75992784d5242d7cd97eca793f3e86b25fd25b7c9f1b71c82a738e4` |
| `crates/executor-storage/src/lib.rs` | `8ddc7ea2339f2d9c439e76dc5b3895360a3d7bf2a708eee31dfd619e486349f7` |

## Source findings

Complete running membership includes mountless consumers. Missing, extra, duplicate, stopped or mismatched full snapshots refuse before handles open; PID 0/1 and values outside the documented supported range refuse. Original declaration and sorted binding digests include the full inputs. Public reports are Serialize-only observations; private owner fields and the absence of an import/provider seam prevent reports from constructing retained owners.

Each owner retains a pidfd, proc directory, nsfs mount-namespace descriptor and root descriptor per process, plus five protected host handles. Live pidfd polling and the bounded fdinfo PID tie the retained handle to the numeric directory in the held procfs view. Start and numeric credential reads, fresh/retained proc-root-namespace identities, complete raw table digests and liveness are bracketed during collection and revalidation. Any revalidation failure latches invalidity. Owned descriptors close on partial failure and Drop. Namespace mount IDs and Btrfs stat/table device values remain separate observations; the code does not infer host or physical backing identity from them.

The production seam requires effective UID 0, protected fixed root/proc paths, real procfs, the collector/PID-1 namespace match and existing host-root check. Record paths use retained-directory relative openat2 restrictions; only fixed kernel namespace/root links are followed. Missing primitives, denied access and unsupported records refuse. No signal, namespace entry, mount or data write occurs in production collection.

The original five-second declaration lifetime uses remaining monotonic time and wall-clock checks; revalidation never renews it. Two-second work checks are cooperative and preserve collection's initial deadline. Records, table bytes, rows, tokens, serialization and descriptors have explicit caps. The documentation correctly distinguishes per-pass aggregate table bytes from total collection traffic, and acknowledges that blocked synchronous syscalls still need an independent worker deadline and single-flight admission.

## Evidence inspected, without rerunning tests

Independent data checks matched all nine source/fixture input hashes against both the worktree and implementation commit, all 67 manifest hashes, and ten required passing check-log/source bindings. The historical workspace log contains **320 unit/integration tests and two compile-fail tests**, including **29 new process tests**. This is baseline 291 plus 29 unit tests and two compile-fail tests; it is not a current-main or merged-source test count. Qualification records capture 19 workspace test artifact hashes. Cached advisory revision, excluded live yanked checks and x86-64 environment are explicit.

Read tests cover real owned child lifetimes, exit between pidfd/proc acquisition, retained dead handles, Drop closure and ordinary/detached descriptor refusal. A private adapter substitutes host authentication on the UID-1000 workstation while delegating child primitives to the production Linux adapter. The source and documentation state this limit consistently. PID reuse, namespace changes, UID/GID changes, table changes, clock interleavings and many exhaustion cases use private injected proof. Compile-fail tests reject snapshot deserialization and external owner construction. No reviewed claim treats those fixtures as authenticated Docker ownership or installed capability proof.

The admitted maximum uses 262,144 rows and 8,059,904 raw table bytes per pass. The recorded retained row heap payload is 32,505,856 bytes (31 MiB); isolated test peak RSS is 67,356 KiB (65.78 MiB), including fixture and harness overhead. Frozen private clock qualification does not prove production two-second performance. The real 64-child run records 261 additional retained descriptors, a peak of 265 and return to baseline after Drop. A tighter 32,768 aggregate row cap is a documented proposal, not an implemented guarantee. These figures do not sign off P00 production footprint or latency.

## Remaining integration work

Full exact-source merged checks and CI remain the integrator's responsibility. Installed UID-0, cross-UID, nondumpable, namespace-switch and service-confinement success remain untested. Authenticated Engine acquisition/reacquisition, namespace/destination/source/UUID/backing composition, pool/protection/share propagation, approvals, complete claims and root ceilings, bounded runtime admission, effect-time checks and execution remain open. BKP-001 and DSK-001/MNT-001/MNT-002/RT-001 and applicable P04 gates receive no closure from this review. The historical 61.401 ms result against the 20 ms inventory budget remains unchanged.

The reviewer performed source/document/log reads and independent artifact data comparisons only. No tests, repository/Git mutation, host/SSH/Pi/wybie access, Docker operation, frozen Python changes or delegation ran. This review note is the only new file, under `/tmp`.
