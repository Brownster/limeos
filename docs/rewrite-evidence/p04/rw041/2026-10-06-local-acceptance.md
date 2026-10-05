# RW-041 planning foundation: local acceptance

Delivered on `rw-041-pool-protection` from `589cf9a6de587d301ecaacff9eeb92869af40e7c`, in `/tmp/limeos-rw041`. This is the bounded assignment in [the engineer plan](../../../plans/2026-10-05-engineer-pool-protection-planning.md), not completion of all RW-041. [The source/feature mapping](../../../p04-pool-protection-planning.md) records supported imports, limitations and the read-only interfaces needed from RW-040.

The assignment estimated two to four engineer days. Actual automated local work took approximately 45 minutes, including source reading, implementation, cold builds and local validation, from worktree creation at 2026-10-05 22:26 UTC to final checks around 23:11 UTC (2026-10-06 in Europe/London). This is agent wall time, not an estimate of human engineering or runtime integration effort. No live or privileged storage qualification was attempted.

| Check | Result | Evidence |
|---|---|---|
| `cargo test --workspace --locked --offline` | 161 passed; 30 new planner/parser regressions | [Rust test transcript](rust-tests.txt) |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed | [Strict Clippy transcript](clippy.txt) |
| `cargo fmt --all -- --check` | Passed | [Validation record](validation.json) |
| `uv run scripts/check_repository.py` | Passed: dependency direction and hygiene | [Gate transcript](gates.txt) |
| `uv run scripts/check_contracts.py` | Passed: generated schemas and TypeScript match Rust | [Gate transcript](gates.txt) |
| Frozen source manifest | 17 pi-health files verified at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204` | [Fixture manifest](../../../../tests/fixtures/pool-protection/manifest.json) |
| Final source binding | SHA-256 for 26 implementation/contract/fixture inputs | [Rust and fixture source hashes](source-sha256.json) |

The first compile needed compiler-cache access outside the filesystem sandbox; existing socket tests also require that access profile. An initial regression exposed Serde accepting an empty sequence as a defaulted struct. Imports now verify top-level and nested JSON shape while still rejecting duplicate keys through direct typed deserialization. A strict Clippy counter-loop finding was corrected with enumeration. The recorded final run passes all tests and checks.

Pure domain/configuration checks and supplied fixture observations cover SRA-001, SRA-002 and the real-result handling rules behind MFS-001. No defect-register row is closed by these tests. The fixtures are synthetic configurations/output and do not establish live mount, host-effect or recovery safety.

RW-040 still owns protected collection, shared dependency locks, authorized operation versions and executor recovery. Unresolved admission work includes additional legacy mount options, block/hash sizes, cron expressions and status/log variants, plus the distinct audited threshold override. Mount/unmount/balance, sync/scrub/repair, package/lifecycle setup, fstab writes, schedules, registry integration, core migrations, executor effects/protocol, HTTP and UI are outside this delivery. Wybie and the frozen Python project receive no changes or testing.

Merge the implementation before its isolated generated-contract commit, then the evidence/documentation commit. No branch was pushed or merged into the integrating worktree.
