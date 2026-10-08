# Independent merged-source review

Verdict: **GO for the reviewed source composition at `0e2cf08a882035b69cb65efead5323b01d03152c`**. This source review does not assert that pending merged-source tests or CI have passed.

Read-only comparison main before integration: `91909a40a3c0da7aa5da6b138889d4388969309c`. At inspection, the working tree was clean and matched the reviewed merged commit.

## Exact integration

All 70 changed paths from corrected custody tip `a869842a0273677f088d4fa563216ba091f1af29` and all 66 changed paths from process tip `cb5aa668f48d29cf21ee1c90a63a39724cc3230a` match the merged commit and working files byte for byte. Their union is the entire 136-path merge delta: no missing or extra path. This comparison includes code, fixtures, interface docs and all historical evidence; no original proof file was rewritten.

All four reviewed Engine code/test files retain their exact approved hashes and are byte-identical to comparison main:

| Source | SHA-256 |
|---|---|
| `crates/executor-container/src/docker.rs` | `e3c50eff69700a74d73c8775500f73fb27cae49f87447081104c29b3500bde32` |
| `crates/executor-container/src/docker/storage.rs` | `aa6d4d462481ee7dc92d805a7a66f531a0d6703151c58f5f211348d9125dd5f0` |
| `crates/executor-container/src/docker/process_evidence.rs` | `4119a317cc1e44aa3e138cbdecaf9561ef4b3eb82b3c2e9f47948b7743ed7aa5` |
| Engine private tests | `3921dac5333ea9eb19ca76745e8dd3eeb5dc4b6a77e58e12eb5de66401f3a9c2` |

Corrected custody source remains `a89218676b0fd9f88e1ebaa1b3b2dd74c96f632e86aff0f321cf52a149c7e77d`; its tests remain `d4fc2e3af33b96b8796d026e74744fef1ae9be137d91585a6242d7b165f3979a`. Process source remains `44e33fdf2ae5bf0c56c58af08ff4427424e51e80ccda073cb56bc191c0b10ab3`; its tests remain `723c50154f0cb92b384031e3ffaba00fc35e4a8ac16d2ece9ac1c479eab38f94`.

## Dependency and exposure checks

All 211 external package records, versions, source identities and checksums are unchanged. The lockfile still has 227 packages. Its sole local record change is the approved existing `serde` edge for `limeos-backup-archive`; `serde_json` moves from that crate's dev dependencies to regular dependencies. The process crate enables the permitted feature on already pinned rustix. The workspace manifest and package versions are unchanged.

The reviewed public additions are library exports `custody` and `processes`. The merge delta does not alter API/core/executor runtime, executor protocol, contracts, packaging, workflows, repository scripts or UI. Source equality confirms no new owner import/provider, weaker public policy, route, runtime operation, capability or effect exposure beyond the reviewed seams. Engine/process ownership and physical composition remain separate unfinished integration gates.

## Qualification limits

Original custody 312-test/RSS evidence retains its historical implementation attribution. Original process evidence retains 320 unit/integration plus two compile-fail tests and its private host-authentication substitution. Engine evidence retains its prior 315-test source qualification. These are separate branch/source histories, not results for the merged source. Full merged-source required gates, exact-source CI and final publication evidence still need their own recorded results.

The corrected JSON path bounds canonical output-buffer allocation; it does not bound every preceding admission/typed serde allocation. The process library's maximum retained row payload and isolated RSS do not sign off P00 footprint/latency. SIGKILL/restart fixtures do not prove physical power-loss durability. Installed UID-0/cross-UID/capability and authenticated Engine-to-kernel composition remain unqualified. No defect or phase gate closes.

This review only read Git objects/files and compared source/dependency data. No tests, repository/Git mutations, host/SSH/Pi/wybie/Docker access, frozen Python changes or delegation ran. The report is the only new file, under `/tmp`.
