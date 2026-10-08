# RW-040 public probe: final exact-source CI review

2026-10-08. Reviewer: `engine_evidence_review`. Read-only source/evidence review; no test, host, repository write or Git mutation.

**GO for the final producer CI claims and their publication, after refreshing the enclosing proof manifest with this note.** The source remains `dbf1aac8d1ffb3293dc85e890636a114ecbe5eca`, probe SHA-256 `3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c`. Existing source/default-framing reviews remain applicable. This adds the completed exact-source CI proof; it does not approve a combined collector or privileged named probe cases.

## Independently verified capture

The frozen capture `/tmp/limeos-probe-ci-37813669102/evidence` and repository `docs/rewrite-evidence/p04/rw040-combined-read-probe/ci` contain the same 25 byte-identical files, including the manifest. All 24 manifest entries verify. Capture manifest SHA-256: `3ec25231ca07e777f1f0930a94900c50b05ae43aefc5ed17d92c80fb2fac606a`; summary SHA-256: `8cbdb548227d4c8fb9189651a579998ab8a425edcc6771a023f96be5ffcc880d`.

Run `37813669102` and all three jobs completed successfully at exact head `dbf1aac8`; the installed job completed `2026-10-08T17:36:16Z`. Each compressed log decompresses to the recorded raw hash and byte count. Raw Cargo summaries independently total **375 unit/integration plus two compile-fail tests = 377 per native architecture**, with zero failed/ignored. Both native jobs record **33** qualification-harness tests; the installed job records **15** reference tests. The ordinary native probe test is the default contract case, not opted-in host collection.

The five exact fresh installed paths contain **14/6/61/50/49 checks = 180**. Each body matches its selected artifact ZIP member, recorded digest/length and common Debian image identity. The successful job log records completion of each named suite. The 33 historical artifact paths are excluded, and are not substituted for the fresh results.

All three downloaded artifact ZIP digests match the capture and API metadata. Independently extracted package data confirms **six** standard/shadow Debian package hashes, exact control bytes, architectures and **24 embedded ELF64 binary hashes**. Every installed result's reported package/binary hash matches the corresponding current native package; all five images agree. Reading package tar contents executed no installed program or maintainer script.

All **305** input-manifest entries match their exact `dbf1aac8` Git blob, SHA-256 and byte count. Current runtime/build/package/fixture/test inputs match that source; the main README prose was subsequently updated and is separately recorded below. Production code, dependency identities, package policy and the probe remain unchanged. The historical 372-test `2fd74209` result and pre-format local five-test attempt remain explicitly separate.

## Final prose hashes reviewed

- Main `README.md`: `50ddb5f2dbf5087c8a7899d137e9ca8bd37ec5b5323800d7d456e7ec8d4d1366`
- Producer `docs/p04-rw040-combined-read-probe.md`: `4b361f15970308ea14530899b2c41140d4bb2b67d9bc7e2071743b4b6a14bfce`
- Supplied-prerequisite confinement brief: `9e2e1fa86842ebd57db803cc6c533cba8e1aacda92a1a48126436f54c8454f70`
- P04 execution tracker: `1523958fdabd642cfe003429aa8ef4934d7ef8904656e68ee35c25dcac5a7b89`
- Probe proof README: `bcc1fe6e693ccdca3a589cc4ecf0088911c8a3f2a8d2c6e7050001f5082f5e2c`

The claims distinguish the exact native/default probe and existing installed-suite success from the engineer's privileged guest/root/Engine/process/source/confinement work. The original `2fd74209` harness pin is unchanged; supplied probe composition and binary identity remain explicit. No private authentication provider, root-policy override or engineer ownership transfer is authorized.

## Open gates

The `combined` probe case remains blocked. Pre-allocation admission, killable synchronous worker composition, original delivery expiry, caller-death/descendant reaping, whole-cgroup 256-FD/64-MiB/16-task headroom, privileged named cases and actual service confinement remain unqualified. Namespace-local IDs do not prove host destination/source/UUID identity. No restore/effect/claim/pool/protection/share/defect/cutover gate closes, and the historical inventory budget failure remains unchanged. No Pi/wybie/SSH/workstation Docker or frozen Python operation was run by this reviewer.
