# RW-040 public-library guest probe

The integrator supplies `bins/executor/tests/combined_read_probe.rs` at source **`dbf1aac8d1ffb3293dc85e890636a114ecbe5eca`**, SHA-256 **`3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`**. It uses the executor bin's existing dependencies and production public APIs. The source commit adds only this test file; runtime/library/package/policy inputs remain those qualified at `2fd74209`. [Evidence](rewrite-evidence/p04/rw040-combined-read-probe/README.md) records 377 passing local workspace tests and independent review. [Its exact-source full CI](https://github.com/Brownster/limeos/actions/runs/37813669102) passed 377 Rust and 33 harness tests per native architecture, 15 reference tests and 180 checks across five fresh installed AMD64 suites. This qualifies the default probe tests and existing installed suites; privileged guest probe cases remain separate.

This supplies the missing current-library prerequisite in the [confinement brief](plans/2026-10-08-engineer-rw040-confinement-qualification.md). The combined worker remains unavailable. Use only owned disposable guest/CI resources under that brief; no workstation Docker, wybie/Pi/SSH or frozen Python operation is included.

## Build and invocation

From the exact supplied source, build the integration-test executable:

```sh
cargo test -p limeos-executor --test combined_read_probe --locked --release --no-run --message-format=json
```

Choose the `compiler-artifact` JSON record whose `target.name` is `combined_read_probe`, whose `target.kind` contains `test`, and whose `executable` is non-null. Invoke that executable with:

```text
--exact qualification_probe --nocapture --test-threads=1
```

Record its exact source, binary hash, build profile/toolchain and guest image identity. Preserve the qualification branch/worktree; the engineer may consume the integrator-supplied test file/commit unchanged, with its SHA verified, rather than editing that reserved path. Record that source composition explicitly if the harness is built on the earlier pinned `2fd74209` branch. This does not authorize private authentication/provider substitutes or shared runtime edits.

The trusted guest harness sets these environment inputs:

| Input | Contract |
|---|---|
| `LIMEOS_RW040_PROBE_CASE` | Default `contract`; one of the cases below, at most 32 UTF-8 bytes |
| `LIMEOS_RW040_PROBE_SOCKET` | Trusted in-process endpoint, default `/run/docker.sock`, absolute UTF-8 path at most 512 bytes; production root socket/peer policy remains fixed |
| `LIMEOS_RW040_PROBE_CONTRACT` | Absolute protected storage-contract path at most 512 UTF-8 bytes, required for dependencies; read through production `read_contract` |
| `LIMEOS_RW040_PROBE_PAUSE_MS` | Zero through 5000 milliseconds, at most four decimal bytes; default zero |

`std::env::var_os` copies OS values before Config checks these limits. The pause follows the flushed collected event while owners remain retained. It permits real transitions and FD/RSS sampling, consumes the original lifetime and changes no clock/provider. The harness independently bounds the probe process and owns cleanup.

## Cases

| Case | Public-library scope |
|---|---|
| `contract` | Default no selected host collection; available cases and missing supervision prerequisites |
| `engine` | Genuine canonical Engine/socket authentication, complete declarations/PIDs and fresh retained-owner GET revalidation |
| `engine-processes` | Engine-derived complete running bindings and retained kernel process/root/namespace observations |
| `engine-sources` | Engine-derived declarations and retained host-source observations |
| `storage` | Retained block/mount/fstab inventory and revalidation |
| `engine-dependencies` | Protected contract, Engine-derived declarations and retained host-source/filesystem binding |
| `combined` | Explicit `blocked` result; combined worker/admission/supervision is not supplied yet |

After the pause, Engine-backed selected-owner cases perform **fresh retained Engine validation → selected retained owner revalidation → fresh retained Engine validation**. Engine-only performs its fresh validation; storage-only revalidates storage. No owner is recollected to mint a later issue time. The original timestamp is at `observations.engine.declarations.observed_at`; stopped/unmanaged/mountless membership remains in its complete declarations.

## Event and acceptance contract

Each event line starts with `LIMEOS_RW040_PROBE_JSON=` and contains one JSON object with `version: 1`, `case`, `phase`, `status`, `stage`, `error`, `effective_uid` and `observations`. Ignore libtest's separate progress output. Require a **successful probe exit plus exactly one final event**, the expected case and expected final status. The final status is `ok`, `refused`, `invalid_input` or `blocked`; refusal carries the stable library/code tag and stage. A collected event is provisional and never qualifies final success. Successful libtest execution alone does not imply collection success.

The writer fallibly reserves a fixed 256 KiB event buffer and serializes borrowed owner reports directly into it. It writes no prefix until the full bounded event is encoded. Each case emits at most a collected and final event, for at most 512 KiB of event bodies plus fixed framing. Serialization/write/flush errors invalidate execution. A flush error can occur after valid JSON was written, so JSON alone cannot qualify success.

Final `ok` proves completed selected-owner revalidation. Blocking stdout delivery can outlive that check: external supervision and current-age checks remain required. Serialized reports do not retain or reconstruct owners and provide no effect authority. Kernel socket credentials plus retained endpoint identity do not independently prove a Docker daemon PID; record socket-activation/endpoint facts separately.

## Remaining qualification

Local tests ran the no-host default/contract/input/output paths as UID 1000. Guest UID-0/public-library success, cross-UID/nondumpable/capability cases, actual resource headroom and transitions belong to the engineer's separate proof. The current service caps remain unchanged. The process-only maximum already holds 261 descriptors before other owners and its maximum-row fixture reached 65.78 MiB; the combined design must reject whole requests before unsupported resource acquisition rather than omit consumers or raise 256-FD/64-MiB service limits.

Namespace-local mount IDs are not host filesystem/UUID identity. Destination mapping, combined worker admission/deadlines/parent-death/descendant reaping, pool/protection/share propagation, claims/approvals/effect-time checks and live restore remain open. No P04/defect/cutover gate closes. The [probe-first design](plans/2026-10-08-rw040-combined-read-design.md) separates this delivered prerequisite from that later implementation.
