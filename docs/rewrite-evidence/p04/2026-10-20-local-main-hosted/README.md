# Merged-source hosted qualification, 20 October 2026

Run [39429582703](https://github.com/Brownster/limeos/actions/runs/39429582703)
binds source `477b81f2ba78c9d100a6857950f50467079b406e`. Its native ARM64 and
AMD64 jobs both passed. Each ran **1,130 Rust workspace checks and 219 frontend
cases**, strict Clippy, actual contract generation/drift, dependency/advisory
gates and native Debian package builds. AMD64 also passed the browser lifecycle,
logs and keyboard-focus regressions. These are repeated qualification runs,
not extra unique coverage to add to the local test counts.

The complete run **failed** in Debian VM acceptance. The genuine A1 host-consumer
scan encountered a ninth mount namespace against its documented limit of eight
(`scan.namespace.limit`, observed 9, limit 8). Plan creation returned HTTP 503
before effect admission. This is not the earlier optional-Kthread ABI failure
or generated-contract drift. The first excess observation does not establish
the total namespace count or identify its owners. The downloaded evidence does
not supply that census. No production limit, consumer check, fixture assertion
or service profile was relaxed to obtain a passing result.

The VM job first completed the P01, P02 and P03 acceptance wrappers. Only their
fresh `ci-*` outputs and the current P04 failure file are copied from its artifact;
the artifact also contains old tracked evidence which is not credited as new.
The complete VM log preserves the embedded A1 refusal and teardown report.
The later dependency VM wrapper was not reached. There is no full installed
ARM64, crash-recovery, performance, P04 completion or Pi cutover acceptance.

`hosted-run.json` preserves terminal job/step conclusions. `hosted-artifacts.json`
records the platform's artifact IDs/digests. Native and VM logs are byte-preserving
gzip copies of the respective job-log endpoints. `artifacts.sha256` covers this
packet; `manifest.json` distinguishes native success from the failed installed
gate. The source and the final local audit differ only in documentation.

All committed local tips are ancestors of actual local main. Cleanup removed
78 clean merged worktrees and 88 merged local branches in total. Eight dirty
worktrees remain, with current recovery patches/archives under
`.cache/main-consolidation/final-audit/recovery/`. Full history is retained in
the verified `.cache/main-consolidation/refs-final.bundle` and earlier bundles.
The unfinished stack-inventory start was saved separately under
`.cache/main-consolidation/recovery/stack-inventory-start/`; it is not accepted
runtime source. Its committed design is included in main. Remote branches and
production/Pi services were not modified by this consolidation.
