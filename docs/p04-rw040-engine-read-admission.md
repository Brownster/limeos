# Closed Engine read admission for RW-040

The additive trusted-library seam is `Docker::inspect_storage_processes_with_profile(EnginePeerPolicy::root(), EngineReadProfile::CombinedV1)`. It returns the existing opaque retained owner. Profile selection has no numeric limits, UID override or wire representation. The standard protected collector and advisory API retain their behavior. The canonical endpoint, kernel peer authorization before every HTTP request, complete double inspection and original five-second dual-clock lifetime are described in the [Engine interface](p04-container-engine-process-evidence.md).

| Closed CombinedV1 bound | Scope |
|---|---|
| 16 consumers, 8 running, 32 selected mounts | Complete membership includes stopped and mountless consumers; excess refuses the whole collection |
| 8 KiB version, 64 KiB other JSON responses | Individual HTTP response bodies |
| 2 MiB cumulative HTTP body bytes | Initial acquisition and every fresh revalidation share one spent allowance; failed reads spend bytes and overspend latches exhaustion |
| 2 MiB counted live payload | Retained facts, temporary response/parser buffers, typed copies and comparison overlap share atomic reservations |
| Depth 32, 2,048 nodes, 256 sequence/map entries | Every response visits unknown fields as well as selected facts; the root list is limited to 16 |
| 128 KiB decoded key/string bytes, 16 KiB per decoded string | Per response; caller-owned strings use fallible reservation before copying |
| Existing 48 KiB report, 4-second collection, 2-second request | Original wall/monotonic lifetime survives reacquisition; no deadline or report limit is raised |

Excess sequence items and map keys use a rejecting seed. Closing the exact-size collection succeeds; the next token refuses before its nested or escaped contents are decoded. The selected list/container semantic parser is shared with existing callers. Complete membership, PID validation and selected mount validation have one source of truth.

A retained owner keeps the same admission state through revalidation. Concurrent collection on that owner receives immediate `Unavailable`; it does not wait or queue. Temporary reservations release on failure, cancellation and drop. Lifetime input bytes do not replenish. The live ledger includes overlap between the retained snapshot and fresh comparison copies.

Before each response, the collector counts an allowance for raw-body Vec growth (three times its cap), escaped serde decoder scratch (`4 × cap + 64`) and 64 KiB transport scratch, then separately charges decoded and typed output growth. The scratch model is conditional on serde_json 1.0.145, Rust 1.88.0 and the resolved alloc/default/raw_value/std features. The [pinned audit](rewrite-evidence/p04/rw040-engine-admission/serde-json-scratch-audit.md) records the requested-buffer growth and old/new overlap model. Changed features, typed 128-bit number decoding or another toolchain require a new audit.

Vec/String output reservations are fallible and refuse a surprising reported capacity. BTreeMap has no fallible reserve: entry/cardinality payload is charged before insertion, while node splitting, allocator metadata, stack and allocator abort remain outside this counted-payload guarantee. Each bounded launch and collection also reserves a fixed 8 KiB allowance for endpoint copies, URI/version formatting, validator sets and small request/error bookkeeping. These conservative allowances do not claim exact accounting of every std allocation, total heap, RSS or cgroup headroom.

The [source-bound evidence](rewrite-evidence/p04/rw040-engine-admission/README.md) records 17 new tests, 62 crate tests and 392 unit/integration plus two compile-fail workspace tests. Isolated Unix listeners cover exact and next-member boundaries, lifetime byte spending, production peer rejection before HTTP, retained-owner concurrent refusal/cancellation and standard 64-consumer/advisory compatibility. These fixtures do not qualify an installed root Engine or combined collector.

The integrator estimate remains 16 human engineering hours with scope review at 24; P04 remains 320/480. Agent activity, tool waits and human effort were not independently timed. Bounded storage/process/source adapters, retained composition, independently supervised synchronous reads, parent/descendant cleanup, installed resource qualification and namespace-destination binding remain prerequisites. No runtime route, RPC, schema, command, service allowance or effect changes. DSK-001, MNT-001/MNT-002, RT-001 and BKP-001 remain open; the historical 61.401 ms inventory result still fails 20 ms.
