# RW-043 archive admission: evidence

Branch `engineer/backup-archive-admission` from `60e6309384c6a93caa63c0d578dc57d981897863`. Brief: [Engineer 2: RW-043 archive admission](../../../plans/2026-10-06-engineer-backup-archive-admission.md). Design and integration requirements: [backup archive admission](../../../p04-backup-archive-admission.md).

All work and testing ran locally, unprivileged, against synthetic data. Nothing ran on wybie, and the Python project was only read.

## Effort

| Item | Value |
|---|---|
| Estimate | 24 engineering hours; review point 36 hours |
| Actual | One agent session on 2026-10-06, from 16:01 to 16:35 BST. That's elapsed agent time, not human engineering hours, so it doesn't revise the estimate. |
| Infrastructure waits | Building `cargo-deny` 0.20.2 and `cargo-audit` 0.22.2 locally (not previously installed), about 15 minutes, overlapping other work |

## Frozen Python source read

Python source at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read with `git show`; nothing was edited or run. Hashes are in [`frozen-source-sha256.txt`](frozen-source-sha256.txt). Behaviour carried into this slice, or left pending, is mapped in the design document's parity and legacy-mapping tables.

Findings from that source:
- **Creation:** `cmd_backup_create` runs `tar -I zstd -cf` or `tar -czf` over absolute sources, so members carry no leading `/`. Its allowlist accepts `/home/`, `/opt/`, `/etc/limeos/`, `/var/lib/limeos/`, `/var/log/limeos/` and `/etc/pi-health.env`.
- **Restore:** `cmd_backup_restore` checks only the archive path's prefix, suffix and `..`, then runs `tar -x --overwrite … -C /` as root.
- **Overlapping sources:** `backup_scheduler._get_sources` lists `/etc/limeos` and also `media_layout.json`, `media_profile.json` and `credentials.env` inside it. With GNU tar 1.35 this produces self-referential hard links in every primary archive; see the fixture `legacy-primary-overlap.tar.zst`.
- **Existing tests:** the frozen tests covered argument validation only. No test exercised archive contents.

## Commands and results

Run from the repository root on the workstation (x86-64, Rust 1.88.0):

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Pass |
| `cargo test --workspace --locked --no-fail-fast` | Pass: 30 targets, 214 tests, 0 failures ([transcript](workspace-tests.txt)) |
| `cargo test --locked -p limeos-backup-archive -p limeos-domain -- --test-threads=1` | 70 named tests pass: 10 new domain admission tests among the domain crate's 51, plus 3 unit and 16 integration inspector tests ([transcript](crate-tests.txt)) |
| `python3 scripts/check_repository.py` | Pass, with `backup-archive` registered as depending only on `domain` |
| `python3 scripts/check_contracts.py` | Pass; generated contracts unchanged |
| `cargo deny check` (0.20.2) | `advisories ok, bans ok, licenses ok, sources ok`. Three duplicate-version warnings (`base64`, `getrandom`, `syn`) predate this branch ([output](cargo-deny.txt)) |
| `cargo audit` (0.22.2) | No vulnerabilities in 227 crates, against 1,290 advisories ([output](cargo-audit.txt)) |
| `uvx --from shellcheck-py shellcheck tests/fixtures/backup-archives/generate.sh` | Pass |
| `tests/fixtures/backup-archives/generate.sh`, run twice | Byte-identical output. Tools: GNU tar 1.35, zstd 1.5.7, gzip 1.13 ([`TOOLS`](../../../../tests/fixtures/backup-archives/TOOLS), [`SHA256SUMS`](../../../../tests/fixtures/backup-archives/SHA256SUMS)) |

## Fixture matrix

Every case runs through `inspect_and_admit`: real decoder, raw tar iteration, admission. Hand-built cases run under both gzip and zstd and must reach the same decision. Test quotas are small (1 MiB file, 4 MiB decompressed, 64 entries, 4 KiB per record), so a decompression-bomb regression stays bounded.

| Group | Cases | Expected and observed |
|---|---|---|
| Valid | Representative legacy layout in both formats, both produced by GNU tar; POSIX/pax format; GNU long names over 100 bytes; a pax `path` override to a valid name | Admitted; identical sorted manifests across formats and repeated runs; bound to the archive SHA-256, policy revision and per-file SHA-256 |
| Traversal and names | `../`, embedded `..`, absolute, `./`, `//`, terminal escapes, C1 controls, backslash, non-UTF-8, trailing `/` on a file, unmapped `etc/shadow` and `home/pi/.ssh` | `parent_component`, `absolute_path`, `dot_component`, `empty_component`, `unsafe_character`, `non_utf8_name`, `trailing_slash_on_file`, `unmapped`. GNU tar `-P` fixtures confirm absolute and `../` names. |
| Extension overrides | GNU long name and pax `path` replacing an innocent header name with an escape | The override is the name that gets checked: `parent_component`, `absolute_path` |
| Ambiguity and unsupported extensions | Long name plus pax path; two long names; two pax records; a repeated pax key; pax `size`; `SCHILY.xattr.security.capability`; `GNU.sparse.*`; pax global; an extension with no member; NUL in a long name; a malformed pax record | `ambiguous_name`, `unsupported_extension`, `sparse`, `extension_without_member`, `unsafe_character`, `malformed` |
| Links and types | Relative and absolute symlink escapes, a symlink chain, a hard link, a self hard link (including via long link), character and block devices, FIFO, GNU sparse, contiguous, volume and dumpdir; GNU tar fixtures for a symlink, FIFO, sparse file and the helper's overlapping sources | `symlink`, `hardlink`, `legacy_self_hardlink`, `device`, `fifo`, `sparse`, `unsupported_entry_type` |
| Destinations | A duplicate file; a duplicate directory; a file then directory, in both orders; a file then children below it; a file replacing a resource root; a directory with data | `duplicate_destination`, `file_directory_collision`, `parent_is_file`, `directory_with_data` |
| Malformed and truncated | Bad header checksum; non-octal size and mode; cuts inside a header, inside data and before the end marker; only one end block; a hidden member after the end marker; compressed streams cut short or followed by junk; V7 header; plain tar; bzip2; empty input | `malformed`, `truncated`, `trailing_data`, `unsupported_header_format`, `unrecognized_compression` |
| Oversized declarations | 1 GiB long-name record, a pax record over the limit and a 1 TiB file, all with no body behind them; size 2^63; size `u64::MAX`; five records adding up past the total | `metadata_limit`, `file_size_limit` (refused before any read), `malformed` (`size overflow`), `total_metadata_limit` |
| Floods and bombs | 65 entries; five 900 KiB zero files (compressed under 64 KiB) past 4 MiB; 8 MiB of zero padding after the end marker; a compressed limit of 64 bytes; GNU tar zstd fixture declaring 64 MiB (2,214 bytes) | `entry_limit`, `decompressed_limit`, `compressed_limit`, `file_size_limit` |
| Decoder memory | zstd frame declaring a 2^23 window under a 2^21 policy | `decoder_memory_limit`; admitted when the policy allows 2^23 |
| Boundaries | A file of exactly the limit and one byte over; exactly 64 entries; decompressed and compressed limits equal to the archive's own size and one less | Admitted at the limit and rejected one over, in each case |
| Cancellation | Cancel after a few reads; cancel immediately | `cancelled`; no report or manifest |
| Limited reader (unit) | An endless source under a 100,000-byte limit; exactly at the limit; nested `io::Error` wrapping | No more than limit + 1 bytes ever requested; the exact limit is admitted; stop reasons survive wrapping |
| No effect on destinations | Eight hostile archives in both formats, against a temporary directory holding sentinel managed destinations | Every one rejected; the directory tree is byte-identical before and after |

## Footprint

Peak resident memory (`VmHWM` read by the process itself) of the release `inspect` example, under [`measurement-policy.json`](measurement-policy.json) (zstd window 2^21). Raw values are in [`peak-memory.json`](peak-memory.json):

| Archive | Compressed bytes | Peak memory |
|---|---|---|
| `legacy-valid.tar.zst` | 786 | 2,772 KiB |
| `legacy-valid.tar.gz` | 834 | 2,592 KiB |
| 64 MiB of zeros, zstd | 2,205 | 5,088 KiB |
| 64 MiB of random data, zstd | 67,110,937 | 4,936 KiB |
| 64 MiB of random data, gzip | 67,119,335 | 2,592 KiB |

Memory doesn't grow with archive size: it's about 2.6 MiB plus the 2 MiB zstd window. These are workstation x86-64 figures, not Pi measurements. The recorded times were taken while `cargo-audit` was compiling, so they aren't performance claims.

The stripped release example is about 800 KB, including `serde_json`. The libraries aren't linked into any shipped binary yet, so a package size change will be measured at integration.

## Remaining integration requirements

These are owned by later slices or the integrator:
- **Restore executor:** the requirements listed in the design document (staged immutable copy, digest and policy-revision rebinding, re-streaming through this inspector, protected-descriptor destinations, policy-owned ownership and modes, atomic writes, recovery snapshots, receipts, fresh approval, service stop and verify).
- **Legacy primary backups:** decide whether to admit the redundant self hard link; until then they're rejected.
- **Production policy values:** limits from the P00 archive-size inventory, with the zstd window at least 21; legacy mappings from each installation's inventoried `config_dir` and `stacks_path`.
- **Native ARM64:** build and test of libzstd and the inspector, through the existing CI matrix.
- **Repository check:** `scripts/check_repository.py` currently skips any crate missing from `ALLOWED`. Asserting that every `crates/*` member is registered would stop a new crate escaping the boundary check.

BKP-001 stays open.
