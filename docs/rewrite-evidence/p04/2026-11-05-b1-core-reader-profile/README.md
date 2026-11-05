# Corrected Core profile passes the ordinary installed B1 scenario

Source `ce2e9163a7a64ce57f0936034c81cfdf72c439d3` applies the narrow correction
justified by the [installed Core-profile matrix](../2026-11-04-b1-core-profile/README.md).
Core's unit now removes `RestrictSUIDSGID` and explicitly allows `openat2`.
The previous profile deterministically returned `ENOSYS` for its protected
socket lookup. **The ordinary installed Core timer now publishes incident and
recovery notifications and preserves their identities across actual restarts.**

This explicitly removes the SUID/SGID file-bit restriction. `NoNewPrivileges`
retains its exec privilege barrier; it does not recreate that restriction.
Every other original Core directive remains, including the unprivileged
UID/groups, empty capability sets, private filesystem/device restrictions,
128 MiB, 32 tasks, 1024 descriptors, CPU quota and restart policy. Protected
descriptor/peer checks, authority, original deadlines, SQL schema 14, wire
contracts, Root policy/services and all Rust/frontend code are unchanged.

Distinct standard/shadow AMD64 packages `0.4.13+coreopen.1` are signed by
disposable public fingerprint `EFC2BC058C061EB8EAAE8DDA39F0E68E51782B39`; the
local repository signature is independently verified. Actual deb payloads have
the corrected units, root archive ownership and unchanged executable hashes.
Standard/shadow identity, state isolation and resource transforms pass their
contracts; this guest installs only the standard package.

Package and fixture source are both `ce2e916`, with matching **747-input** maps.
The five executables and built UI are the exact verified `e417751` Debian-build
bytes. Of that build's 740 recorded inputs, only the Core unit changes; every
application/Cargo/contract/frontend input matches. Seven test-only files have
been added since that original executable build. Executable and package source
identities stay separate. The
[original build packet](../2026-11-04-a1-installed-refusal-recovery/README.md)
retains compiler/library/footprint and UI-build evidence; those builds are not
rerun or credited as new tests here.

The frozen manifest inherited a field named
`test_only_changes_since_package_build` that actually describes additions since
the older **executable** build. The original manifest is preserved.
`provenance-correction.json` explicitly records the correct interpretation:
current package and fixture have no test-only source difference, use the same
747-input map and differ from the old executable build by precisely the Core
unit plus the seven added test files. The cached package-version wrapper is
independently hashed. No run inputs or assertions were changed to repair that label.

All **34 scoped reference cases** pass: two Core package-profile contracts,
five existing reader-profile cases, five profile-matrix boundaries, nine B1
fixture cases and 13 reused boundaries. Ruff, formatting, repository and diff
checks pass. The actual ordinary scenario uses 33 frozen guest modules and the
exact canonical non-build runner with 1,024 MiB RAM. There is no Core override,
probe unit, fake Engine/RPC server, test credential/clock flag, direct schedule
settlement or mutation retry.

The guest establishes these actual API/runtime results:

- Signed installation and schema 14 derived independently from delivered Rust
  source; actual password enablement/relogin and permission, CSRF, Origin and
  authentication refusals preserve observation history.
- The real default root reader and ordinary Core timer observe one actual
  stopped full-ID `unless-stopped` container. One occurrence/job/result and one
  delivered in-app incident are created. A real Core SIGKILL and automatic
  restart preserve the exact observation history through the stable check.
- Starting that same fixture container produces one clear alert and one
  delivered recovery bound to the same incident generation. Exactly two durable
  occurrence/job/results and two notifications remain; both deliveries have one
  attempt. These are observation/in-app effects; only the fixture starts Docker.
- Actual acknowledgment, pause and stale-revision refusal work. Another real
  Core SIGKILL/restart preserves the acknowledged notification, paused revision-2
  schedule and complete history. The original human session remains usable.

Core PIDs progress **984 → 6268 → 6463**. The same authority database is retained;
no jobs, claims, watermarks or notification receipts are deleted or fabricated.
The Root reader policy hash is unchanged. Coverage remains explicitly partial:
container completeness does not imply that host/SMART/mount/protection checks
or other operation adapters have become available.

The original VM guard runs **15:27:32–15:29:34 UTC on 5 November**, exit 0,
747 inputs unchanged and no sampled memory stop. Minimum host availability is
9,246,121,984 bytes. Serial native/package/VM phases retain 4/4.5-GiB admission
and the unchanged sampled 3-GiB floor. Original wrapper, runner and QEMU identities
and the exact owned disk paths were captured live. All seven unique recorded
native qualification PIDs are absent and that VM's scratch is removed. Older
unattributed scratch remains untouched. No unrelated process was stopped.

`installed-result.json` retains actual incident/recovery, before/after restart
history, paused/acknowledged state, real service/Engine/package identities and
original limits. `source-match.json` and the explicit provenance correction
bind package versus executable sources. Compressed native/package/VM guards,
public repository metadata, manifests, cleanup and reproduction sources retain
the exact run. `SHA256SUMS` covers every other packet file. Private keys,
packages, binaries and disks are excluded. The earlier failed B1 attempts and
profile matrix remain separate immutable packets.

This accepts the covered standard AMD64 API/runtime flow. Installed browser/UI,
current ARM64/Pi, upgrade/return, full workload footprint/soak, remaining health
sources, reviewed replacement/retirement, compaction, ordinary channels and
other A/B/P04 parity remain open. A1's task/FD churn and the historical inventory
performance failure are independent and unchanged. B1/P04 and P05/P06 are not
complete. Holly's Pi and frozen application were untouched. Nothing was pushed.
