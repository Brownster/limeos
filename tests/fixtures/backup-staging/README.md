# Verified staging fixtures

`policy.json` is a synthetic measurement policy, never a production default.
All paths are metadata; no test writes to the policy's destination root.

The staging tests in `crates/backup-archive/tests/archives.rs` construct bounded
tar members with the existing test builder and compress them with real gzip
and zstd encoders. They also replay the unchanged, hashed GNU fixtures in
`../backup-archives/`, including the primary legacy inert repeats. Private
writer/fsync faults are in `crates/backup-archive/src/staging/tests.rs`.

`docs/rewrite-evidence/p04/rw043-staging/measure_staging.py` generates deterministic
128 KiB and 64 MiB payloads from SHA-256 of little-endian 64-bit counters.
USTAR headers have fixed names, modes, IDs and timestamps; gzip mtime is zero.
The zstd executable only compresses generated fixtures. A separate 1,000-file
empty-payload case distinguishes catalog metadata from streamed file buffers.
Generated archives remain temporary; their hashes and full measurement output
are recorded in the evidence directory.
