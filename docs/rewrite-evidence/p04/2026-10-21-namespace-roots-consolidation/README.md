# Namespace roots and local main re-audit, 21 October 2026

Product source: `b28bdb19789abeaf2255dee2c31bd7d8ace8c5c1`, with design
`b8654f3`. Local main fast-forwarded from `d7f09e1`; the clean feature branch was
deleted. The following evidence-only commit changes no product inputs.

The captured kernel diagnostic shows the real root `net:[4026532191]` on an
`nsfs` mount. Storage now classifies that rendering separately from filesystem
paths in Standard and Combined host/process readers. Namespace rows remain in
digests and mount references, cannot become physical backing/destination paths,
and cannot stand in for an independently inspected mount namespace. The existing
uninspected bound mount-namespace effect refusal remains explicit.

| Check | Result |
| --- | --- |
| Pre-fix regressions | Two expected failures; original log retained |
| Initial focused run | 17 passed, one existing Btrfs fixture acquisition conflict; retained |
| Disk compilation followed by private runtime tmpfs fixtures | 225 unit tests passed; fixtures removed |
| Storage compile-fail doctests | 13 passed |
| Storage all-target Clippy | Passed with `-D warnings` |
| Workspace Rust formatting | Passed |
| Actual Rust schema generator / tracked contract comparison | Passed on merged main |
| Repository dependency and hygiene checks | Passed |

The successful compile, runtime, Clippy and doctest records each match **671
tracked input hashes** against the committed source, despite having run before
the code commit was created. `source-match.json` records that reconciliation;
the original full before/after hashes remain in the compressed records.
One Cargo job and one test thread ran at a time. All builds, caches, logs and
general scratch stayed on repository disk. Only tiny storage runtime fixtures
used private tmpfs: sampled peak 53,248 bytes, with a 16-MiB sampling threshold
and per-file ceiling. The host memory guard retained its 3-GiB stop floor.

The genuine kernel rendering capture is a diagnostic from the preceding work,
not a runtime execution of this new storage binary. There is no new installed
service, package, ARM64, effect, recovery or performance acceptance here.
The unfinished standalone VM probe passed Clippy but lacks a controller and
runtime result; its four draft files were moved intact to the local recovery
directory named in `draft-recovery-manifest.json`. They are not merged source.

The refreshed branch audit proves all local and fetched remote branch tips are
ancestors of actual local main. The other engineer's schedule/contracts commit
`ca13ffb`, ABI fix `4baf338` and handoff `71df2a6` were already included.
`post-cleanup.json` records a clean root checkout, nine local branches (main
plus eight dirty worktrees), no clean secondary worktree and no push.
Fresh recovery copies preserve all eight drafts; the full history bundle was
verified before deletion. Remote branches were left unchanged.

A1's userspace-task revalidation conflict and the remaining A/B, P05 and P06
requirements remain open. No Pi access or production operation ran.

`SHA256SUMS` covers every other file in this packet. Original failed attempts
are kept separately from passing checks; repeated runs are not added to totals.
