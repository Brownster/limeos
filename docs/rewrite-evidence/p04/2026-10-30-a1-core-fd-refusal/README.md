# Exact Core FD owner after durable guardian binding

Fixture source `02ba0d59df806ec493619d6de48a24c3083e77a1` separates four
bounded Engine warmup passes from subsequent proc-only quiet observation. It
retains the original three-second quiet requirement, 15-second lifetime and
96-observation cap. All **98 reference tests** pass natively. The new connection
churn regression fails with the expected timeout against the former `8f0c471`
helper and passes with the separated probe. This proves the modeled observation
defect; it does not identify the exact changing membership in the earlier guest.
Scoped Ruff, formatting, repository and diff checks pass.

The fresh canonical 1024-MiB guest reused unchanged schema-12 packages
`0.4.11+fdcontext.1`, built at
`f6f583e69518a732bcb8a34b2d09c7f4e3b7e23d`. All 687 inputs match the fixture
commit, with the four test-only differences from that package build recorded
explicitly. Product, binary, UI, package and service identities remain unchanged.
All 31 guest modules and the exact canonical runner are frozen in the manifest.

Docker setup completed four read passes and 44 proc observations, reporting
3.0508 quiet seconds in 4.4152 seconds. All three negative API cases passed,
then the real positive plan, approval and queue succeeded. Execution ended with
Core `needs_intervention` and now retains the actual Root receipt:

- Root stored `outcome_unknown`, with `pending` quiescence and guardian PID
  10248, start 27953, host namespace 4026531841, the actual executor image digest
  and `/system.slice/limeos-storage-mounts.service` cgroup. The guardian identity
  had been durably bound. There is no reaped proof or accepted `after` value.
- Root/Core each retain the same five resources, equal to Core's required set:
  configuration, the whole disk and partition, mountpoint and filesystem UUID.
- The later independent mount-table observation has no target row. That is a
  later observation, not proof of earlier effect history or helper quiescence.

Root's exact FD context identifies **PID 6331, TID 6333, start 18693,
namespace `(4, 4026532305)`, FD 19**. The underlying `link.open` returned
`ENOENT`; the wrapper correctly leaves errno unknown. Independent before/after
task witnesses match every identity field and identify
`/system.slice/limeos-core.service`. The actual FD link target, purpose and
precise close mechanism were not captured. This establishes the owner of this
run's refusal; owners of earlier FD failures remain unknown.

The source sequence and admission diagnostic point to the gated guardian's
release-time revalidation as the refusal stage. This is an inference from the
durable helper binding plus the error path, not a recorded GO/quiescence proof.
The unchanged recovery barrier remains required; the failed action was not
retried or repaired.

The fixture opens and closes a new Core HTTP connection for every job poll,
at 200-ms intervals. Investigate a scoped persistent HTTP connection for the
positive API flow, retaining ordinary authentication/CSRF and every production
census comparison. The concrete vanished FD is not proven to be that polling
socket. Other Core activity may still change its FD table. Do not filter Core,
ignore a missing FD, stop production collectors or renew admission ownership.

The detached guard completed 17:03:11–17:07:54 UTC on 30 October, exit 1,
unchanged inputs and no sampled memory stop. Minimum host availability was
5,367,726,080 bytes, above the 3-GiB floor. Its process was observed absent after
completion and VM scratch was removed. No accepted mount, UI, replay/restart,
ARM64 or cutover acceptance is claimed. Holly's Pi was untouched; nothing was
pushed.

`source-match.json` retains the actual execution observations. Compressed
original native/VM guards, manifests, failure and startup records preserve the
joins. Exact fixture, launcher and expected-red sources are archived.
`SHA256SUMS` covers every other file; private keys, packages, binaries and VM
disks are excluded. The package build's full reproduction packet remains in
[its original candidate evidence](../2026-10-30-a1-docker-startup-and-deadline/README.md).
