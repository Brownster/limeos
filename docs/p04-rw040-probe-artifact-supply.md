# Debian-compatible public-library probe supply

The native AMD64 CI job builds the existing public-library integration test in the same resolved `rust:1.88.0-bookworm` builder image as the packaged executables. The test source remains unchanged, SHA-256 `3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`. This avoids supplying an Ubuntu-linked test executable to the Debian 12 qualification guest. It does not execute a privileged probe or start a guest.

Each native CI architecture also runs the seven artifact identity fixtures and the corrected console harness's unit guard suite. These exercise parser, lifecycle and delivery regressions; they do not launch a QEMU guest or qualify the probe under confinement. The producer workflow is integrated together with the reviewed harness, whose source is required by that guard step.

`scripts/build_rw040_probe_artifact.py` consumes the complete Cargo JSON output from the locked release `--no-run` build. It requires one successful build and one correctly named integration-test executable with the exact target/package source and release path/profile. Before publication it verifies a clean exact workflow source, source-file hash, native AMD64 ELF, executable mode, Rust 1.88.0, build libc 2.36, the GNU ELF interpreter, GLIBC requirements no newer than 2.36, and dynamic libraries supplied by Debian's libc6/libgcc-s1 packages. It refuses absent, duplicate, malformed or incompatible artifacts. It verifies all three selected package identities and hashes and binds their tested source to the same workflow head.

The `rw040-public-library-probe` artifact has three files:

- `combined_read_probe`: the compiler-artifact executable, renamed without changing its bytes.
- `combined_read_probe.rs`: the unchanged public probe source.
- `manifest.json`: the closed supply schema agreed with the confinement harness.

```json
{
  "contract": 1,
  "source_commit": "<exact workflow head: 40 lowercase hex>",
  "source_sha256": "3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c",
  "binary": "combined_read_probe",
  "binary_sha256": "<SHA-256 of the executable>",
  "toolchain": "1.88.0",
  "profile": "release",
  "library_source": "<same exact workflow head as the selected packages>"
}
```

The separate `rw040-probe-build-evidence` artifact retains complete Cargo messages/stderr, resolved builder image inspection, detailed build provenance, source-input hashes, package hashes/control fields, rustc `-Vv`, build libc and actual ELF dynamic/version/program-header output. Raw compiler diagnostics are uploaded even after a failed probe build. The builder's content-addressed image ID is used for both the package and probe builds; its recorded ID need not be a registry manifest digest. External collectors additionally verify the GitHub artifact ZIP/API digest, run/head identity and each supplied file hash. These build records are trusted CI provenance; they cannot establish privileged runtime success or authorization by themselves.

The guest invokes the supplied executable using only the fixed [public probe interface](p04-rw040-combined-read-probe.md): `--exact qualification_probe --nocapture --test-threads=1`. A consumer must verify the package and probe source agree, the eight-field manifest is closed, and both file hashes match before boot. It must preserve this artifact's actual run/source/compiler/image identities, use only the owned console/QMP guest, and require the probe's successful exit plus exactly one valid final event. The unchanged-confinement and unconfined guest-root baselines remain separate.

Preparation of this producer does not prove that its first real CI build passed. No installed Engine/process/combined headroom, worker supervision, live restore or P04/cutover gate closes. Production unit capabilities and resource ceilings remain unchanged. The historical 61.401 ms storage-inventory p95 against 20 ms stays failed.
