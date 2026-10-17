# Network-group observation fixtures

The inspect and health fixtures belong to the existing supplied-reader slice.
`wire.json` adds a synthetic cached read report with a provider, two actual members
(including a stopped one-off), and one unresolved member carrying an unverified
dependency hint. Case-sensitive project paths and complete IDs are preserved.

The new API unit test checks this file against the existing Rust `observe` output.
Frontend tests consume the same file through bounded wire admission. The fixture
is ordinary supplied metadata, not a live Engine/namespace/VPN capture or a
protected owner. Unknown, stale and failed observations must not be reconstructed
as an empty successful inventory.

Run the [combined B checks](../../stream-b/README.md) with repository disk scratch,
then frontend type/unit checks sequentially using pinned dependencies. The
[handoff](../../../../docs/p04-network-read-ui-handoff.md) covers pending cache,
route, generated-contract and authenticated-shell registration.
