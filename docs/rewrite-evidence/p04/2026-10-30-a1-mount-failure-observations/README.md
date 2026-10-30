# Bounded observations for an uncertain mount

Fixture source `e2012bec0987d524f9c72712ee7789b6b920ad72` adds independent
post-failure, pre-stop reads of the queued action's Root receipt/quiescence,
Root/Core claims, required resources and later target mount rows. SQL admits
stored byte lengths before materializing text: one receipt up to 64 KiB and
32 resources of at most 256 bytes per section. Unavailable sections remain
explicitly unknown. Root-only disposable-host and exact action guards precede
all reads. These later observations are neither atomic across databases nor
execution/recovery authority; they do not retry, repair or release claims.

Real SQLite tests exposed retained query handles. A connection's transaction
context does not close it. The fixture now explicitly closes every read-only
connection on return or error. With garbage collection disabled, the new native
regression performs 32 reads plus a failing query and requires unchanged FD
membership. It passes at this source. The isolated expected-red probe runs that
same regression against the original `7796359` query helper: **33 descriptors
remain**, producing the expected single failure. No source or real service was
modified by that probe. This identifies a fixture defect, not the owners or
causes of earlier installed FD/process refusals.

All **97 reference tests** pass natively, including real receipt/claim reads,
unchanged database bytes, independent errors, byte/count bounds and host/action
refusals. Scoped Ruff, formatting, repository and diff checks pass. The native
record predates the commit, but its 687 recorded input hashes match this fixture
commit. No Rust product input changed.

Two distinct canonical 1024-MiB guests reused unchanged schema-12 packages
`0.4.11+fdcontext.1`, built from
`f6f583e69518a732bcb8a34b2d09c7f4e3b7e23d`. Manifests record the four
test-only source differences and all 31 guest modules. Both ended before a
positive job, so **installed execution capture remains unexercised**:

- First run: Docker setup completed, then the first plan returned HTTP 409.
  Root's initial PID census grew from 100 to 101, adding PID 7697. Its later
  witness read returned `FileNotFoundError`/`ENOENT`; the actor is unidentified.
- Second run: the unchanged Docker setup probe reached its 96-pass limit
  without reporting three quiet seconds. No plan or positive job was created.
  The first failure's original files were hashed before this separate launch
  and verified afterward. Neither run reused prior disk or operation state.

The guards completed 16:43:05–16:47:38 UTC and 16:51:54–16:56:12 UTC on
30 October, both exit 1, unchanged inputs and no sampled memory stop. Minimum
host availability was 5,050,105,856 and 5,322,559,488 bytes. Both guard processes
were observed absent after terminal completion; VM scratch was removed.

The current setup probe opens fresh HTTP connections while checking FD
settling. Separate a fixed warmup from the subsequent quiet proc observations
to remove that source of measurement activity. The exact changing FD/task
sequence in the second run was not retained, so this is a fixture design
correction rather than an established cause of that refusal. Keep the same
quiet duration, attempt/deadline bounds and every production comparison.

The uncertain mount in the preceding packet still requires actual Root
receipt/quiescence capture. No accepted mount, UI, replay/restart, ARM64 or
cutover result is claimed. Holly's Pi was untouched and nothing was pushed.
Compressed original records, exact reproduction sources and `source-match.json`
retain the joins; `SHA256SUMS` covers every other packet file. Private keys,
packages, binaries and VM disks are excluded.
