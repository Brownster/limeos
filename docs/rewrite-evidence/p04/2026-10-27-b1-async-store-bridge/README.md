# Asynchronous Store bridge, 26–27 October 2026

Source `7b44a0158ffc5ec044121b4aa914786cf65d08e4` implements the
[Core bridge](../../../p04-b1-async-store-bridge-handoff.md) to the existing sole
Store worker, including bounded original-job rediscovery after restart or a lost
reservation reply. No production collector or timer is registered.

| Final check | Result |
| --- | --- |
| Full Core unit suite | 37 passed |
| Core integration suites | Six backup-binding tests and one password-worker test passed |
| Full persistence unit suite | 144 passed |
| Persistence compile-fail doctests | Two passed |
| Focused Core bridge tests | Six passed; included in the full Core unit count |
| Strict Clippy | All targets of Core and persistence passed with `-D warnings` |
| Workspace formatting, actual contract drift, repository hygiene | Passed |
| Source identity | Six final snapshots each match 687 recorded tracked inputs in the implementation commit |

The seven new regression functions cover real worker concurrency, restart,
private leases, current scope/grant and pause checks, partial observations,
in-app publication/retention, expired queued work, a response deliberately held
after an actual commit, bounded cursor coverage and query-only job pages.
Fixture signals are not genuine installed protected health collection. The new
queued-job page is tested under SQLite `query_only=ON` and excludes terminal jobs.

`source-match.json` records commands, timestamps, memory minima and input
identity. The working candidate was based on `72ce6be`; the recorded dirty input
hashes are verified against the final implementation commit, rather than
pretending the tests ran after that commit. Compressed records retain complete
before/after snapshots and raw output. `source.patch.gz` preserves that commit;
`guard.py` preserves the actual runner. `SHA256SUMS` covers every other packet
file. The initial successful formatting mutation is retained separately from
final qualification. The interrupted formatting invocation before the goal
resumed created no attempt directory and establishes no test result.

Final checks used one Cargo job and one test thread, disk scratch/caches and the
unchanged 3-GiB memory guard. Their recorded available-memory minima stayed above
6.4 GiB. The obsolete backup-staging debug target cache was cleared to make disk
space; its release directory remained present.

These are local native worker/Store checks. Protected health collection,
configured policy and concrete-resource replacement, timer supervision,
alert/notification HTTP/UI, installed packages, ARM64, Pi and whole-service
resource/idle-write acceptance remain open. Existing schedule metadata still
reports awaiting producer. No production mutation or push ran; this packet does
not close B1, P04 or cutover readiness.
