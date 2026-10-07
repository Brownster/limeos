# Archive custody fixtures

`policy.json` and `limits.json` are synthetic measurement inputs for the
[custody example](../../../crates/backup-archive/examples/custody.rs). They are
not production defaults: production admission policy and custody limits still
come from the P00 inventory and the caller's own quota/reservation decision.
All destination roots are `/synthetic/...` metadata; nothing writes there.

The custody tests build their archives in-process with fixed tar metadata and
real gzip/zstd encoders (`crates/backup-archive/tests/custody.rs`), and reuse
the unchanged, hashed GNU tar fixtures in `../backup-archives/`, including the
inert legacy self-repeat in `legacy-primary-overlap.tar.zst`. No archive is
committed here, so there is nothing to regenerate.

`docs/rewrite-evidence/p04/rw043-custody/measure_custody.py` generates the
measurement archives deterministically: SHA-256 of little-endian 64-bit
counters as payload, a fixed USTAR header, gzip with mtime zero and
`zstd -3 -T1`. The generated archives stay temporary; their digests and the
complete raw output are recorded in the evidence directory.
