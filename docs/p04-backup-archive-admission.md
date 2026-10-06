# Backup archive admission

This is the first RW-043 slice. It supplies the checks BKP-001 requires before an archive can reach a restore executor: a bounded streaming inspector for legacy `.tar.gz` and `.tar.zst` backups, and a pure admission planner. The planner returns either a digest-bound restore manifest or a typed rejection.

Nothing here restores, writes, follows a link or resolves a destination. **BKP-001 stays open.** It closes only when the installed restore path enforces these checks and proves recovery.

| Component | Contents |
|---|---|
| `limeos_domain::backups` | Closed admission policy, archive limits, the managed-resource registry, trusted legacy mappings, inspected entry facts, typed findings, admission and the restore manifest. Pure, with no filesystem, process or decoder dependency. |
| `limeos-backup-archive` | `inspect` and `inspect_and_admit`. Streams one archive through counting, hashing and limited readers, a gzip or zstd decoder and raw tar iteration. |
| `crates/backup-archive/examples/inspect.rs` | Inspects one archive under a policy file and prints the manifest or rejection, for operators and evidence. |
| `tests/fixtures/backup-archives/` | Archives produced by GNU tar with the frozen helper's own compression flags, a generator script and SHA-256 sums. |

## Integration interface

```rust
// Trusted policy in; manifest or typed rejection out. `cancelled` is polled on every read.
pub fn inspect<R: Read>(source: R, policy: &AdmissionPolicy, cancelled: &dyn Fn() -> bool)
    -> Result<InspectionReport, Rejection>;
pub fn inspect_and_admit<R: Read>(source: R, policy: &AdmissionPolicy, cancelled: &dyn Fn() -> bool)
    -> Result<RestoreManifest, Rejection>;
pub fn admit(policy: &AdmissionPolicy, report: &InspectionReport) -> Result<RestoreManifest, Rejection>;
```

`inspect` returns a report only after the whole archive has been read and verified:
1. the tar end-of-archive marker
2. nothing but zero padding after it
3. the decoder's own integrity check (gzip CRC and length, zstd content checksum when present)
4. an exact end to the compressed input

Any failure, including cancellation, returns a `Rejection`, so a partial inspection never yields a usable plan. `admit` revalidates the policy and the report's own consistency before mapping anything.

A `RestoreManifest` contains:
- the SHA-256 of every compressed byte, the format and the policy revision
- compressed and decompressed byte counts and the header count
- file and directory counts and total file bytes
- one entry per member: managed resource, relative path, kind, size, SHA-256, archived permission bits and effective archive path

Entries are sorted, and the manifest has no timestamps or ownership, so the same archive and policy always produce the same manifest.

### Requirements on the future restore executor

**A manifest is evidence that one exact archive passed inspection under one policy revision. It is not permission to restore.** The executor slice must:

1. Copy the selected archive into an immutable staging file that only the executor can write. Open it with `O_NOFOLLOW`, `fsync` it, and record the staged digest.
2. Inspect the staged copy, never the original, with the current trusted policy. Require the manifest's `archive_sha256` to equal the staged digest and its `policy_revision` to equal the policy in force, both at approval and at apply time. If either changed, re-inspect and get a new approval.
3. Write file data only by streaming the staged copy through this same inspector, checking each file's size and SHA-256 against the manifest before committing it. Never pass the archive to system `tar` or another extractor: the bytes written must be the bytes inspected, and parser differences are a known attack class.
4. Resolve every destination from the registry (resource ID to destination root), not from manifest strings. Use protected descriptor traversal (`openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS` and `RESOLVE_NO_MAGICLINKS`, as `executor-storage` already does), and refuse missing, symlinked, foreign-owned or unexpected-filesystem roots.
5. Set ownership and modes from trusted per-resource policy. Archived permissions are informational. Never apply archived uid/gid, setuid/setgid/sticky bits, extended attributes or ACLs.
6. Write each file to a temporary file in its destination directory, `fsync` it, and rename it into place. Keep a recovery snapshot of every replaced path, and a durable receipt before any effect, so power-failure recovery never replays blindly.
7. Exclude database files from file-level restore until the database-aware snapshot slice exists.
8. Bind the job to the manifest digest, staged digest and policy revision, with fresh authorized approval of that exact manifest. Stop affected services before applying, then verify afterwards.

## Supported subset

| Area | Admitted | Rejected (finding code) |
|---|---|---|
| Compression | gzip (magic `1f 8b 08`) and zstd (`28 b5 2f fd`), detected from content and limited to the policy's `formats`. Filenames are never consulted. | Anything else (`unrecognized_compression`); a format the policy excludes (`format_not_allowed`) |
| Tar headers | GNU and ustar | V7 or unknown magic (`unsupported_header_format`) |
| Member types | Regular files (`0` and NUL) and directories (`5`) | Symlinks (`symlink`), hard links (`hardlink`, or `legacy_self_hardlink` when a member links to its own path), character and block devices (`device`), FIFOs (`fifo`), GNU sparse members and pax sparse keys (`sparse`), contiguous, volume, dumpdir and unknown types (`unsupported_entry_type`). Sockets can't be stored in tar. |
| Extension records | GNU long name `L`. GNU long link `K`, only to classify a link member. Pax local `x` with `path`, `linkpath`, and the informational `mtime`, `atime`, `ctime`, `uid`, `gid`, `uname`, `gname`. | Pax global `g`, pax `size`, extended attributes, ACLs or any other key (`unsupported_extension`). Repeated keys, two records of one kind, or a GNU name together with a pax path (`ambiguous_name`). A link name on a non-link (`ambiguous_name`). A record with no member (`extension_without_member`). |
| Names | Effective names after extension records: UTF-8, relative, literal and case-sensitive. A single trailing `/` only on directories. | Non-UTF-8 (`non_utf8_name`); absolute (`absolute_path`); empty, `.` or `..` components (`empty_component`, `dot_component`, `parent_component`); C0, DEL or C1 controls, backslash or NUL (`unsafe_character`); trailing `/` on a file (`trailing_slash_on_file`); length, depth and component limits |
| Destinations | Members inside a trusted legacy mapping, each landing on a distinct resource path | Unmapped members (`unmapped`), duplicates (`duplicate_destination`), the same path as both file and directory, or a file replacing a resource root (`file_directory_collision`), anything beneath a file (`parent_is_file`) |
| Stream integrity | The second end-of-archive block followed only by zero padding, an exact compressed end, and decoder checksums | Checksum, size or mode fields that don't parse, or overflowing arithmetic (`malformed`); cut streams or a missing end marker (`truncated`); members after the end marker (`trailing_data`) |

Directories carrying data are rejected (`directory_with_data`). Unicode-equivalent spellings stay distinct, because restore targets are byte-literal Linux filesystems. Path checks for admission run in the pure domain; findings about the stream and member types stop inspection at the first problem.

## Limits

The trusted policy owns every limit, and archive metadata can't relax any of them. The inspector checks a declared size before reading or allocating: file sizes, extension-record sizes and running totals. All counts use checked arithmetic. Each limited reader asks its source for at most one byte past its limit, so an over-limit archive is never read further. Skipped and padding bytes count toward the decompressed limit like everything else.

| Field | Bounds | Test value |
|---|---|---|
| `max_compressed_bytes` | Compressed bytes read, including trailing input | 1 MiB |
| `max_decompressed_bytes` | Every decompressed byte: headers, extension records, data, padding | 4 MiB |
| `max_file_bytes` | One regular file's declared size | 1 MiB |
| `max_entries` | Tar headers of every kind | 64 |
| `max_path_bytes`, `max_path_depth`, `max_component_bytes` | Effective member names | 256, 16, 128 |
| `max_metadata_bytes` | One GNU long-name, long-link or pax record | 4 KiB |
| `max_total_metadata_bytes` | All extension records together | 16 KiB |
| `max_zstd_window_log` | zstd decoder window memory, 2^n bytes (10 ≤ n ≤ 27) | 21 |

Policy validation rejects zero limits, a file limit above the decompressed limit, and inconsistent name limits.

**The zstd window must be at least 21.** The frozen helper ran `tar -I zstd` at zstd's default level, which declares a 2 MiB window; the fixtures generated with those flags confirm it (`zstd -lv`: `Window Size: 2.00 MiB`). A policy below 21 rejects every legacy zstd backup with `decoder_memory_limit`.

Production values must come from the P00 inventory of real archive sizes, not from these test values. On the reference host the frozen helper wrote about 2.2 GB a day, so primary archives may be gigabytes.

Measured peak memory (`VmHWM` of a release build, workstation x86-64, window 21):
- **Small archives:** 2.6–2.8 MiB, gzip or zstd.
- **64 MiB of data:** 5.0 MiB through zstd and 2.7 MiB through gzip.

Memory is about 2.7 MiB plus the zstd window, whatever the archive size. Retained metadata adds at most the entry and metadata limits. See the [evidence](rewrite-evidence/p04/rw043/README.md).

## Trusted legacy mapping

The frozen helper archived absolute source paths with `tar -cf ARCHIVE [--exclude …] SOURCES…`. GNU tar strips the leading `/`, so `/etc/limeos/core.json` becomes the member `etc/limeos/core.json`.

Admission accepts a member only when a trusted `LegacyMapping` places it in a registered managed resource. The longest matching prefix wins, on a component boundary. Policy validation rejects:
- prefixes with fewer than two components
- `home/<user>` prefixes with fewer than three
- known system roots (`var/lib`, `var/log`, `usr/lib`, `usr/local`, `etc/systemd`, `etc/ssh` and similar)
- overlapping destination roots, and mappings to unknown resources

The frozen helper's own allowlist (`/home/`, `/opt/`, `/etc/limeos/`, `/var/lib/limeos/`, `/var/log/limeos/`, `/etc/pi-health.env`) is deliberately not reproduced.

| Frozen source (`backup_scheduler.py`) | Member prefix | Suggested resource | Status |
|---|---|---|---|
| `config_dir` (default `/home/pi/docker`, configurable per installation) | e.g. `home/pi/docker` | `app-config` | Map from the installation's inventoried value, never a default |
| `stacks_path` (default `/opt/stacks`) | `opt/stacks` | `stacks` | Supported |
| `/etc/limeos` | `etc/limeos` | `limeos-config` | Supported |
| `/var/lib/limeos` | `var/lib/limeos` | `limeos-state` | Mapping is possible. Whether Python-era state is restored at all is a migration decision; its SQLite files need the database-aware slice |
| `/etc/limeos/media_layout.json` and `media_profile.json`, listed again explicitly | — | — | **Rejected today:** GNU tar stores the repeated file as a hard link to itself (`legacy_self_hardlink`) |
| `/etc/limeos/credentials.env` (`include_env`) | — | — | Also a repeated source, so a self hard link. Secret material should stay unmapped until credential import is designed |
| Plugin archive: `/etc/limeos/storage_plugins`, `/var/lib/limeos/storage_plugins` | as above | inside `limeos-config` and `limeos-state` | Supported |
| Plugin archive: `/var/log/limeos/snapraid` | `var/log/limeos/snapraid` | — | Logs are not restored; leave unmapped (`unmapped`) |

### Known gap: current primary legacy backups are not admitted

The frozen primary backup lists `/etc/limeos` and also files inside it (`media_layout.json`, `media_profile.json` and, with `include_env`, `credentials.env`). GNU tar 1.35 stores each repeated occurrence as a zero-length hard link to the same path. The fixture `legacy-primary-overlap.tar.zst` reproduces this with the helper's flags.

This slice admits only regular files and directories, so every such archive is rejected with `legacy_self_hardlink`. Plugin archives don't overlap and are admitted.

A narrow later rule, for the integrator to decide on, could treat a zero-length hard link whose target is its own path, and which follows an already-inspected regular file of that path, as a redundant duplicate with no new destination. It shouldn't be enabled without that review; until then a legacy primary backup can't be restored through this path.

## Parity with the frozen restore

| Frozen behaviour (`80593b2`) | This slice |
|---|---|
| Archive chosen by name; `backup_service` rejected `/` and `..` in the name. The helper required a `/mnt/` or `/backups/` path ending in `.tar.zst` or `.tar.gz`. | Format comes from content, not the name. Selecting and staging the archive is the executor's job (pending). |
| `tar -x --overwrite -f ARCHIVE -C /` as root: GNU tar decided which members to write, anywhere under `/`, recreating links, devices, ownership and modes. | No extraction. Only admitted regular files and directories inside mapped resources reach a manifest. Ownership and special bits are dropped. |
| No limits on size, entries or expansion | Every limit is policy-owned and enforced while streaming |
| Stacks stopped before restore and restarted after; `last_restore` recorded | Pending, in the executor and job integration |
| Separate plugin-archive restore | Same inspector under a plugin policy |
| Backup creation, schedules, retention, excludes | Unchanged and frozen; Rust backup creation is a later RW-043 slice (encrypted backups) |

Frozen source hashes are recorded in the [evidence](rewrite-evidence/p04/rw043/README.md).

## Dependencies

| Crate | Pin | Notes |
|---|---|---|
| `tar` | `=0.4.46`, default features off | MIT or Apache-2.0, MSRV 1.63. Raw entry iteration is used because the library's processed mode reads extension records of any declared size into memory. Its next-header arithmetic is checked (`size overflow`). |
| `flate2` | `=1.1.10`, `rust_backend` | MIT or Apache-2.0, MSRV 1.67. Pure Rust (`miniz_oxide`). Gzip header name and comment fields are capped at 64 KiB by the library. |
| `zstd` | `=0.14.0`, default features off | BSD-3-Clause, MSRV 1.64. Wraps `zstd-sys 2.1.0+zstd.1.5.7`, which builds the vendored C libzstd. `window_log_max` bounds decoder memory. |

Transitive additions are `filetime`, `crc32fast`, `miniz_oxide`, `adler2`, `simd-adler32`, `zstd-safe` and `jobserver`; no existing lockfile version changed. libzstd is native code, and `unsafe_code = "forbid"` still applies to all first-party crates. The aarch64 build of libzstd goes through the existing cross linker setting but hasn't yet been built natively for ARM64 in this slice. `cargo deny check` passes. The release `inspect` example is about 800 KB stripped, including `serde_json`.
