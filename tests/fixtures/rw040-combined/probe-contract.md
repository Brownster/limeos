# Historical draft — replaced by the published producer contract

The draft below records the engineer's original proposal and is retained for
the frozen historical run. It is **not** the implemented interface. The current
owned harness consumes the unchanged producer documented in
[`docs/p04-rw040-combined-read-probe.md`](../../../docs/p04-rw040-combined-read-probe.md).
Its argv is `--exact qualification_probe --nocapture --test-threads=1`; inputs
are the closed `LIMEOS_RW040_PROBE_*` environment, and output uses prefixed
`LIMEOS_RW040_PROBE_JSON=` events. It has no stdin transition acknowledgement.

Supply `--probe DIR` with these three regular files: `manifest.json`, the unchanged
`combined_read_probe.rs` and its exact compiler-artifact executable named
`combined_read_probe`. The manifest's closed fields are:

```json
{
  "contract": 1,
  "source_commit": "<40 lowercase hex>",
  "source_sha256": "3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c",
  "binary": "combined_read_probe",
  "binary_sha256": "<64 lowercase hex>",
  "toolchain": "1.88.0",
  "profile": "release",
  "library_source": "<exact source of the installed package artifacts>"
}
```

The runner requires `library_source` to match the package supply record. It
records the source composition and verifies both files again in the guest.
These fields attribute a trusted supplied build; they are not independent proof
of how the compiler produced that executable. Keep compiler-artifact JSON and
the actual build logs with the supply evidence.

Fresh runs are limited to U0 unrestricted root and B0 unchanged installed reader
confinement. Empty and complete fixtures run standalone `engine`; complete
fixtures also run `engine-processes`, `engine-sources` and `storage`. The exact
case-specific combined/transition assertions, a protected dependency contract,
the combined worker and its acceptance remain explicitly blocked. The receiver
supports a collected callback during the existing fixed pause for later owned
transition wiring; none of those transitions is claimed qualified here.

## Original draft

The harness can't claim any RW-040 library success until the integrator supplies
the exact qualification probe. That probe is reserved as
`bins/executor/tests/combined_read_probe.rs`, and the base `2fd7420` doesn't contain
it. This page proposes what the harness needs from it. The integrator owns the
probe and may change any of this. The runner's `--probe` option refuses until an
agreed interface is implemented on both sides.

## What to supply

| Item | Purpose |
|---|---|
| Probe source commit and the SHA-256 of `combined_read_probe.rs` | Source attribution; the harness records it and never edits it |
| Built probe executable for Debian 12 amd64, with its SHA-256 | Built the same way as the packages, in the `rust:1.88.0-bookworm` build image. It uses only `bins/executor`'s existing dependencies and production public APIs. |
| `manifest.json` | The fields below, verified by hash before boot |

```json
{
  "contract": 1,
  "source_commit": "<40 hex>",
  "source_sha256": "<64 hex>",
  "binary": "combined_read_probe",
  "binary_sha256": "<64 hex>",
  "argv": ["{binary}", "--exact", "<test name>", "--nocapture"],
  "env": {}
}
```

## How the harness would run it

- **Modes:** each case runs as unrestricted guest root and, separately, under the
  reproduced storage-reader confinement. The confined run uses the installed
  unit's own `[Service]` settings, proven property-by-property equal to the
  installed `limeos-storage-reader.service` (`confinement-equivalence.json`).
  Nothing is granted or raised.
- **Environment:** the harness sets
  - `LIMEOS_RW040_CASE`: the case ID from `cases.json`
  - `LIMEOS_RW040_PHASES`: `collect` or `collect,revalidate`
  - `LIMEOS_RW040_REVALIDATE_DELAY_MS`: for the expiry case

  The probe chooses the Engine socket itself, and the harness expects the
  canonical `/run/docker.sock`. The probe must not accept a socket, policy or
  host-authentication override from the harness.
- **Transitions:** between phases, the probe prints the exact line
  `RW040-READY-FOR-TRANSITION` and reads one byte from stdin. The harness then
  applies the owned transition (restart, exit, root switch, namespace switch,
  swapped device or Btrfs consumer) and writes that byte. It records the
  transition's independent facts beside the probe result.
- **Output:** one JSON document on stdout, with:
  - `outcome`: `evidence` or `refused`
  - the refusal's typed failure, when refused
  - the original issue time
  - complete membership, process and host-source observations, as the
    production report types serialize them
  - the probe's own peak descriptor count and `VmHWM`

  The probe exits 0 for evidence and 3 for a typed refusal; anything else is a
  harness failure. A refusal must never be reported as empty or partial
  evidence.
- **External measurements:** the harness samples the probe's transient unit
  while it runs: descriptors, RSS, `VmHWM`, PSS, cgroup memory peak, `rchar` and
  `read_bytes`, wall time, and closure (the unit inactive, its cgroup empty).
- **Comparison:** the harness compares outputs with its independently gathered
  ground truth (`ground-truth/*.json`). It reports each case as matching,
  refused with the expected kind, or failed. It never relabels a refusal as
  success.

## Worker cases

`budget-concurrent`, `worker-cancel-death-deadline` and
`worker-blocked-synchronous` need the later bounded, killable combined worker
and its admission interface. The harness already runs and kills confined
transient units and verifies their closure. It can't claim single-flight,
cancellation or deadline behaviour until that worker exists.
