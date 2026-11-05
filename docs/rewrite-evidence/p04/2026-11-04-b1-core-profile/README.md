# Core's current profile blocks its protected socket lookup

Fixture `afc7a43f813544e19a72b66c9eba688510f1f3a7` implements the
[Core profile experiment](../../../plans/2026-11-04-b1-core-socket-profile-design.md).
The installed Debian 252 matrix confirms that `RestrictSUIDSGID=yes` blocks
the exact `openat2` lookup needed by Core's health client. An explicit syscall
allow alone does not override that filter.

The guest reuses signed AMD64 packages `0.4.12+gateabort.1` from `e417751`.
All 740 original product inputs match. The 746-input fixture map adds six test
files; all 32 guest modules and the exact canonical 1,024-MiB runner are frozen.
Actual installed executable/package hashes and schema 14 join the original
[package build](../2026-11-04-a1-installed-refusal-recovery/README.md).
No production unit or application code changes in this experiment.

The fixed probe runs as actual Core UID 104, RPC GID 112, with host/container
supplementary groups. It accepts no path or operation input and sends no socket
request. Its lookup uses the same `O_PATH | O_NOFOLLOW | O_CLOEXEC`, zero mode,
`RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` and fixed reader socket as the Core
client. Direct invocation establishes the Core-UID visibility/DAC control.
Four separately named units copy the entire installed Core profile, changing
only their probe executable, runtime directory and `Restart=no`, plus the
explicitly selected experimental setting. Exact unit text and effective systemd
properties are retained.

| Context | SUID/SGID restriction | Explicit `openat2` allow | Result |
| --- | --- | --- | --- |
| Direct Core-UID control | Outside unit | Outside unit | Exact socket opens |
| Current Core profile | Yes | No | `ENOSYS` (38) |
| Restriction removed only | No | No | Exact socket opens |
| Allow added only | Yes | Yes | `ENOSYS` (38) |
| Both changes | No | Yes | Exact socket opens |

Every successful descriptor matches the same root-owned socket's device, inode,
UID/GID, mode and link count. All four unit probes retain empty effective,
permitted and bounding capabilities, `NoNewPrivs=1`, seccomp mode 2, 128 MiB,
32 tasks and 1024 soft/hard descriptors. The direct control has no unit filter
and is not credited as equivalent confinement. The actual running Core's unit
hash, live PID 957, schema/generation and user/job/schedule counts are identical
before and after. No schedule or authority row is manufactured.

This establishes a deterministic incompatibility in the profile used by Core's
required lookup. It does not record the original timer's errno or prove that
all other Core stages will pass. The narrow correction already described in
the design is now justified: remove Core's SUID/SGID file-bit restriction and
explicitly permit `openat2`, preserving its remaining confinement and resources.
That restriction is genuinely removed; `NoNewPrivileges` prevents privilege
gains at exec but does not recreate the file-bit ban. Keep protected lookup and
peer validation. A distinct signed package must still pass the ordinary B1
incident/publication/recovery/restart scenario before timer acceptance.

All **27 scoped reference cases** pass: five new profile boundaries, nine B1
fixture cases and 13 reused boundaries. Scoped Ruff, formatting, repository and
diff checks pass. The native cases cover profile preservation, closed variants,
pre-syscall refusal, descriptor cleanup, bounded reports and wrong errno/
identity/privilege/resource refusal; they establish no kernel filter behavior.
The original VM guard runs **23:14:43–23:15:34 UTC on 4 November**, exit 0,
unchanged 746 inputs and no sampled memory stop. Minimum host availability is
9,594,650,624 bytes, above the unchanged 3-GiB floor. All scratch is on repository
disk and the serial native/VM admission thresholds remain 4/4.5 GiB.

Cleanup evidence has an explicit limit: observation occurred after the short
VM had finished, so its original QEMU PID/start ticks and temporary pathname
were not captured. Four recorded native wrapper/command PIDs are absent. The
canonical runner returned after its QEMU wait and temporary-directory cleanup;
later native inventory has no matching qualification QEMU. Seven unattributed
scratch directories with timestamps predating this run remain preserved. Do
not count their presence as this run's leak or invent missing QEMU identities.
No unrelated process was stopped or old evidence deleted.

`installed-matrix.json` preserves control, exact unit bytes, all four effective
profiles, probe outcomes and unchanged Core facts. `source-match.json` binds
the source/package and cleanup scope. Compressed native/VM guards, manifests,
observations and reproduction sources retain the full experiment. The earlier
package packet retains full build/repository reproduction. `SHA256SUMS` covers
every other packet file. Keys, packages, binaries and disks are excluded.

B1 ordinary timer/API/UI/restart acceptance, A1 and other A/B/P04 parity,
current ARM64/Pi, return/footprint/soak, the inventory-performance gate and
P05/P06 remain open. Holly's Pi and frozen application were untouched. Nothing
was pushed.
