# Local consolidation evidence, 19 October 2026

Assembled source: `b587621a2cde11442ce318728f57faad6fc384e7`, based on local main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Later documentation-only commits retain
these product bytes. See the [handoff](../../../p04-local-main-consolidation.md).

This packet records local source integration and its actual limitations:

- 219 frontend cases passed across eleven files, one process at a time with a
  256 MiB Node heap limit; strict TypeScript and the 22-module Vite build passed
- 57 reference cases passed outside the sandbox after its local Unix-socket
  sends returned EPERM; the original failed run remains archived
- 63 ARM64 qualification harness cases passed; no native ARM64 host ran here
- One Rust schema-generator regression passed; the actual generator emitted the
  contract outputs. All-source formatting and repository hygiene passed
- The first full workspace attempt stopped before compiling because the default
  offline cache lacked locked rustls. The next used the existing project cache,
  then the unchanged 3 GiB owned-child guard stopped compilation before tests
- Fresh combined workspace Clippy, installed/package/VM, hardware, recovery,
  performance and final hosted qualification remain unrun

The earlier guard snapshots bind their own before/after source inputs. They
predate the later UI, B-harness and documentation corrections and are not whole
final-source qualification. Frontend results bind the final unchanged frontend
bytes. The initial Node `--test` launcher only reported one failing file; its raw
output is retained and receives no contract-regression credit. Direct native
case reporters subsequently recorded the actual passing case counts.

`inputs.sha256` covers a conservative tracked source/configuration/fixture
superset, excluding documentation. Check it from the assembled worktree root;
hashing an input does not imply its suite ran. `manifest.json` records the exact
attempts, commands and limits. `artifacts.sha256` binds this packet, logs, lock,
collector and cleanup/relocation procedures. Compressed original ref identities
and a hash of the local complete Git bundle support cleanup recovery; the bundle
and dirty-file copies stay in the repository disk cache.

The relocated network/tools preview packets retain original README/manifest
bytes and unchanged asset hashes. Existing source-bound packets retain their
historical identities; old or repeated counts are not added here. No production
effect, phase/defect closure, Pi activation, external message or remote push is
credited. The pending hosted-branch permission is separate from local merging.
