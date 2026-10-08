# RW-040 Engine-only admission: exact-source publication review

Review date: 2026-10-08. Reviewer: engine_evidence_review.

Decision: GO for the six-file Engine-only library source and its completed local gates. No source correctness blocker remains. The integrator may publish the atomic source commit. Actor supervision, storage admission, combined installed capacity and effects are outside this decision and remain open.

This review used read-only source, Git scope, metadata and captured output. I ran no tests, repository verifier scripts, host commands or effects. A sandbox setup quota error prevented initial reads; a narrow permitted read recovered access. An earlier report-write tool stalled and was interrupted; that interruption supplies no test result.

## Source binding

Worktree: /tmp/limeos-engine-admission. Branch: integrator/rw040-engine-admission. Base: 1a2947eb1dc9fa91c1bef4ebf311d78898bb01d6.

I independently verified all six actual files against source-final.json in /home/marc/Documents/github/lime-os/target/engine-admission-proof, initially and again after every required gate completed. Manifest SHA-256: 17f1b0de5a089288c8b4a881b0737dabd369214e510be066a7e93a268168a6e6.

| Path under crates/executor-container/src/ | SHA-256 |
| --- | --- |
| docker.rs | bb501302fc95cf2a09e108c1370bda94449fdee9c84f1613a2e402fbe44742bc |
| docker/process_evidence.rs | 8079c39b3021cfa7d540bef78be57b6d501cd9fabcbc0e8d5d6057ab4c1d30cf |
| docker/storage.rs | 9d5c2d544e34c4f9af83bb81992a025e9aab6ef922ef81d56b27d9824f636eb5 |
| docker/read_admission.rs | 6dfa1b16148109cc5c861e4822accebd3590fe24b02e48d9c7370f2fecab6e24 |
| docker/process_evidence/tests.rs | 98ee7706d5ec378a1753b49867b7c717e9bace5e896afe16f55ef54b7c7f98de |
| docker/read_admission/tests.rs | e48a318f1d7e2f8dc3b63d8fdc68460df6825732a3aa08c7a090a446c96f10ea |

Read-only Git status showed only these files. The existing docker/storage.rs change factors selected parsing inside executor-container; executor-storage, dependencies/features, commands, services, wire contracts, package/workflow and other engineers' source paths are unchanged.

## Correctness findings resolved

The closed CombinedV1 public Rust seam admits no numeric limits, caller UID override or wire selection. Public root peer policy, retained no-follow socket identity and authorization before HTTP remain fixed. Standard protected collection and advisory parsing retain their previous semantics.

The constrained path checks complete membership, including stopped/mountless consumers: 16 consumers, eight running processes and 32 selected bind/volume declarations; repeated sources still count. Excess membership refuses the complete result. Double inspection and final list/Engine comparison remain intact.

The retained owner shares its cumulative 2 MiB HTTP-body budget through fresh revalidation, including bytes consumed by failed reads. Overspend saturates the remainder to zero and the next request refuses before HTTP. Shared atomic live reservations account retained/fresh overlap, fallible output copies, response buffers and decoder scratch. Cancellation/error drops release live reservations; consumed input bytes never replenish.

Collection concurrency is an immediate Unavailable refusal, with no hidden queue. The collection guard ends when collection returns; the fresh result remains live-accounted while subsequent comparison runs. The real-socket cancellation/concurrency fixture proves refusal without another HTTP request, peer EOF after cancellation and successful later reacquisition. This is not a lock around every instruction of the entire public revalidation method.

Original wall issue time and monotonic expiry survive all revalidation. Collection uses the earlier of that expiry and the existing four-second limit; requests keep the existing two-second limit. No admission wait resets age.

All JSON fields, unknown nested values, duplicates and keys use the bounded visitor. Excess sequence/map tokens now use a rejecting seed that does not deserialize their contents; exact closing delimiters remain accepted. The controlled rollback demonstrates why IgnoredAny was unsafe: 6,049 total reader bytes were consumed despite a 33-byte admitted prefix. The final formatted logs contain the corrected passing regression.

Oversized borrowed endpoint paths now refuse before Docker/path clones, reservation or open. Running semantic snapshots use fallible charged string copies. String/Vec reservation checks reported capacity before refunding the old charge, so the ledger cannot refund unreserved capacity.

## Accounting scope

Before a response, the code reserves separate raw-body requested-allocation overlap, conditional serde scratch of 4B+64, and bounded transport allowances; decoded/typed outputs are charged separately. The scratch audit applies to Rust 1.88.0 and serde_json 1.0.145 with alloc/default/raw_value/std. The fixed 8192-byte launch/collection allowance covers bounded formatting, validators and request/error bookkeeping conservatively.

These are counted requested-payload allowances, not exact allocator/cgroup heap. BTreeMap/BTreeSet node splitting and metadata, allocator rounding, stack and infallible decoder allocations are not made fallible or empirically bounded by this milestone. The 2 MiB live ledger is not a universal RSS ceiling or installed service-capacity proof.

The scratch tests establish standard Value semantics, accepted Unicode/reused scratch/default long-number cases, late-escape refusal and reservation release. They do not measure peak allocation. The private counted from_reader test isolates excess-token traversal and is separate from the production from_slice scratch model.

## Completed final gates

All following raw hashes were independently verified against paired metadata. The complete final workspace log independently totals 392 unit/integration plus two compile-fail tests = 394, with zero failed or ignored. Final crate output is 62/62, 17 above the prior 45-test crate.

| Final captured gate | Raw SHA-256 | Outcome |
| --- | --- | --- |
| engine-crate-final.log | 9b6c9a2cae825ba72fb6b720a491986137d689af5f34f2f5765ebff5dace5e65 | 62/62, exit 0 |
| engine-workspace-passed.log | 3c4b3517cf272420355d50e431ea9010e64da75a3f263caae36ec4ccf1420523 | 392+2=394, exit 0 |
| engine-workspace-clippy.log | 734a832ddd7e4129b17f251c4718d1d9987bd5aef403e5114aa501df03d23529 | Strict workspace/all-targets, exit 0 |
| engine-fmt.log | e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 | Exit 0, empty output |
| engine-contracts.log | cbaba9d65028978b135ad576ac3121dac062134bf2f3da7e3bff91e29ae15d3f | Exit 0, generated contracts match |
| engine-repository.log | d777bbc8d5a10fe768fb3feb6a83476a4c0114ca044268b8932ed5c0946c54df | Exit 0, boundaries/hygiene pass |
| engine-deny.log | b99220389bbe935db4ea400c5fca145abcd5ce80690af62f1ace482ec5b4e49a | Offline exit 0, existing duplicate warnings |
| engine-audit.log | 5ab8a6f15ea0c9a169c08077d4e8ffa23218a435690b7d7c3df000b5ef4e11c9 | No-fetch exit 0, 1290 cached advisories/227 dependencies |

The implementer separately reports a passing diff check. No new online advisory freshness is implied.

## Historical attempts remain separate

The unfrozen WIP counts and compile/Clippy errors, including the earlier 61-test stage, do not qualify the settled files. The focused historical post-reject capture contains only its 103-byte initial compile chunk, no completion/exit proof (raw SHA 797ddcb01bbbf95e52027ed64ac32a009d4806dd4d84bcf5a0379cbc91504003). The complete final 62-test and workspace outputs supply the settled pass.

The first same-source full workspace attempt used /home btrfs runtime TMPDIR and failed 15 unchanged storage fixtures (raw SHA 72b6957154c83202ca00e8eb6b6abe5a772685ff8444b05c2079fe3b39c82ac7). Two built unchanged storage tests passed under normal /tmp (cc737757413a91ada714b8b04aac43ac8b021e759e90175ee1803d885f45ffd7 and e9fd214422affc1116a9fcc7ef65a0df16bbe2735bd0770f6e1dda81823e7c1b); these are causal diagnostics, not the full gate.

The next same-source full attempt failed with explicit EDQUOT in backup fixture writes (18bfa2d4337d87c4e602f72625f5da4c5d509a28f35298b88f04fee17901fce5). The captured parent-authorized cargo clean names only the completed integrator /tmp/limeos-rw041/target and reports 7028 generated files/2.9 GiB removed (c48289133a22a694de22692b004479573951adea033d17a8931fb5cde20deba2). The final complete 394-test run uses normal /tmp runtime fixtures; source hashes stayed unchanged.

## Open qualification

This GO authorizes source publication, not installed admission/capacity. No actor/guardian lifecycle, independent synchronous deadlines, descendant cleanup, bounded storage/process/source composition, privileged named probe, namespace-to-host mount equivalence, runtime authority or effects gate closes. Existing service limits remain unchanged. Source CI and final evidence publication are later separately attributed steps.
