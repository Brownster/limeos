# Store observation settlement, 22 October 2026

Source `11843c3076cb1918d533b5822af896565f948193` implements the
[Store settlement boundary](../../../p04-b1-observation-settlement-handoff.md).
Schema 12 binds owner-ordered observations to atomic result, alert, outbox, job
and audit settlement, with restart validation and bounded in-app publication.

| Final source-bound check | Result |
| --- | --- |
| Full persistence suite | 143 unit tests and two compile-fail doctests passed |
| Strict Clippy | All workspace targets passed with `-D warnings` |
| Formatting | Workspace check passed |
| Contract drift | Rust schema producer output matches committed contracts |
| Repository hygiene | Passed |
| Full reference suite | 84 tests passed |
| Source identity | Each of six final snapshots matches 685 recorded tracked inputs in the implementation commit |

The ten new completion test functions are included in the 143-test run, not an
additional independent count. The reference suite includes all 11 frozen
notification inventory tests after their final formatting. These checks run in
the isolated implementation worktree against actual Rust/bundled SQLite, using
repository-disk caches and scratch. One Cargo job and one test thread are used;
the host guard still stops below 3 GiB available. Final checks stayed above
7.45 GiB available. The operator authorized gracefully stopping the unrelated
Pixel_10 emulator to free RAM before these checks; Docker workloads were retained.

`source-match.json` records commands, input identity and preliminary attempts.
Compressed records retain full before/after hashes, memory minima and raw output.
`source.patch.gz` preserves the implementation commit, and `guard.py` preserves
the actual runner. `SHA256SUMS` covers all other files in this packet.

The first full persistence run passed 136 tests and failed seven older migration
fixtures. Their shared schema-removal helper retained schema-12 tables in an
older-schema setup. The final commit corrects that helper; all migrations and
the populated schema-11 upgrade test then pass. The earlier focused ten-test
run preceded that helper correction and is retained as preliminary evidence.

Formatting attempt 2 and Clippy attempt 1 were terminated by the unchanged
memory floor. Formatting attempt 3 and contract generation completed while
intentionally changing inputs. The first successful persistence Clippy check
also preceded final source. Those records are preserved without being credited
as final source qualification.

This packet establishes local native Store behavior. It does not qualify a
registered protected collector, timer, Core worker, alert/notification API/UI,
configured-policy migration, external delivery, installed packages, ARM64, Pi or
whole-service resource/idle-write behavior. Default alert policy remains explicit;
concrete-resource replacement and current policy/destination binding need their
own acceptance. No production Pi mutation or push was performed. P04 and the
P05/P06 cutover gates remain open.
