# Independent Engine/PID evidence review

Decision: **GO for the additive library source, acceptance tests and interface documentation.** No remaining source or documentation blocker was found. Publication still requires completion of the implementer's remaining exact-source repository/dependency gates and source-bound publication evidence.

Reviewed on 2026-10-07. The reviewer read repository instructions, design, all four source/test files and the focused raw result. No applicable AGENTS.md was found. The reviewer ran no tests, changed no repository files or Git state, and made no host, Docker, SSH, Pi or frozen Python calls. The local review account reports UID 1000.

## Exact reviewed source

SHA-256 hashes independently read from the settled files:

| Path | SHA-256 |
| --- | --- |
| `crates/executor-container/src/docker.rs` | `e3c50eff69700a74d73c8775500f73fb27cae49f87447081104c29b3500bde32` |
| `crates/executor-container/src/docker/storage.rs` | `aa6d4d462481ee7dc92d805a7a66f531a0d6703151c58f5f211348d9125dd5f0` |
| `crates/executor-container/src/docker/process_evidence.rs` | `4119a317cc1e44aa3e138cbdecaf9561ef4b3eb82b3c2e9f47948b7743ed7aa5` |
| `crates/executor-container/src/docker/process_evidence/tests.rs` | `3921dac5333ea9eb19ca76745e8dd3eeb5dc4b6a77e58e12eb5de66401f3a9c2` |

Focused raw log: `docs/rewrite-evidence/p04/rw040-engine-process-evidence/logs/focused-tests.txt`, SHA-256 `ba84a4b42a76557e41fd8bb26b79670d3f7675b93ddc0757e9612bf9470d7531`. It records **45 passed, 0 failed, 0 ignored**, including 24 new cases, in 4.00 seconds. This is inspected implementer proof, not an independent rerun or installed qualification. Compilation/approval attempts remain separate evidence.

The same four source hashes still matched when the reviewer subsequently inspected all workspace result summaries: **315 passed**, with every suite successful and zero failed/ignored cases. Workspace log SHA-256: `ce3beda63789422d8e35db2128ed08efcc62460baadc83d61def1e317e6d7f74`; passing Clippy result log SHA-256: `d6f521ef8908fd87a28387297dff940bd3bf25320d20906b5ff476e1314d6e2e`. These remain inspected implementer results, not independent reruns.

Interface documentation `docs/p04-container-engine-process-evidence.md`, SHA-256 `cbd3300ceb04bb97c3b0996979f6d642e7bdec3a25cb56609d2ba462c1edd5a2`, correctly states the canonical `/run/docker.sock` obligation and unchanged legacy `/var/run/docker.sock` adapter, body/header/trailer budget scope, private fixture authentication boundary, original issue time and open runtime/cutover gates.

## Findings resolved before approval

The outward declaration timestamp now uses the original lifetime stamp captured before socket acquisition. Fresh reacquisition compares facts without replacing that stamp or the original monotonic expiry. Retained socket link/ownership/mode changes invalidate the owner with Conflict while initial policy refusals remain Forbidden. The authenticated HTTP variant now uses a 32 KiB connection buffer and 64-header bound; existing advisory/action/log transport keeps its prior defaults.

The new acceptance cases distinguish the four-second total collection budget from two-second individual requests using 750 ms responses; admit all 64 running consumers and observe all 128 inspection requests; preserve initial issue time; and reject actual unprivileged kernel peer credentials under the production UID-zero peer rule before HTTP bytes. That last test uses a private socket-owner seam so it can reach the unchanged peer rule without a root-owned fixture. It is not successful production-root Engine qualification.

## Source and test conclusions

- Public policy construction is root-only; private fields and absence of Deserialize/import methods prevent safe public UID overrides or evidence reconstruction. The selected adapter and no-follow socket descriptor remain privately owned across revalidation.
- Every authenticated connection checks SO_PEERCRED UID before Hyper handshake or request transmission. Socket identity is checked before/after connection and response. Numeric peer PID is never reported as retained daemon identity; socket activation remains explicitly outside that claim.
- Complete sorted membership, Engine identity, full selected snapshots/mounts and strict running/stopped PID state are collected twice. Duplicate running PIDs, unsupported PID forms, missing declarations and incomplete responses fail the whole result. Running consumers with no storage mounts remain covered; stopped consumers have zero PID and no running binding.
- Reacquisition preserves original wall and monotonic expiry, checks around I/O, compares complete selected facts and cannot return an empty fallback. Wall rollback, final-response age crossing and unchanged-wall monotonic expiry have meaningful regressions.
- The eight-MiB allowance counts HTTP response-body frames across both inspection passes before appending to the caller body vector. It does not claim eight MiB of total HTTP wire traffic. Per-response bodies, selected count/mount limits, a streaming 48-KiB report check, request and collection deadlines separately bound the admitted work. Hyper's locked trailer parser has its independent byte limit; it is not charged to the body allowance.
- Cancellation and request timeout abort the connection task and the real isolated peer observes EOF. Ordinary Rust drop owns socket descriptors and streams; no process signals or effects are introduced.
- Existing selected parsers were extracted without changing the advisory wire or missing-PID behavior. Existing action/log/receipt tests also passed in the inspected focused log. No RPC, route, command, schema, UI, policy ceiling or runtime registration was added.

## Qualification limits

`openat2` rejects symlink ancestors. A future protected caller must choose the canonical package endpoint, normally `/run/docker.sock`; the existing advisory runtime's `/var/run/docker.sock` path is unchanged and commonly traverses the `/var/run` symlink. Protected ancestors, host namespace and package selection remain runtime obligations. Root listener credentials authenticate the trusted endpoint class; they do not distinguish Docker from another trusted root process or freeze its state.

Successful local fixture parsing uses private test policy. No Docker daemon, installed root collector, running-process kernel owner, namespace-to-source/UUID/backing composition, single-flight worker, independently killable process deadline, pool/protection/share propagation, claim/ceiling/approval binding or effect-time qualification was performed by this slice. Synchronous path and JSON work still requires the later service boundary. Serialized reports and fresh Engine reads grant no operation authority.

No P04 defect or cutover gate closes. The historical 61.401 ms versus 20 ms inventory failure remains unchanged, and the prior wybie window is expired.
