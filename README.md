# LimeOS

Rust replacement for the frozen Python application at `/home/marc/Documents/github/pi-health` (`80593b2`). All development happens here. The deployed reference host, wybie, still runs Python at `618ce92` until cutover.

The [delivery roadmap](docs/plans/2026-10-04-rust-rewrite-roadmap.md) defines the sequence and acceptance gates. The [defect register](docs/plans/2026-10-04-rust-rewrite-defect-register.md) carries 50 requirements into the new implementation.

P01's Rust foundation is implemented: core-owned identity and durable authority, bounded HTTP/RPC, local enrollment, separate executor services, generated contracts and Debian packages for x86-64 and ARM64. The [P01 evidence](docs/rewrite-evidence/p01/2026-10-04-execution-tracker.md) records the tests and remaining qualification gates. The existing Python build remains frozen and deployed unchanged.

The disposable Debian VM passed fourteen acceptance groups. After authentication, core plus both executors used 5.19 MiB combined PSS without swap; five restarts reached readiness within 132 ms. These are VM measurements, not Pi results. The [P00 footprint experiment](spikes/footprint/README.md) separately [passed on the reference Pi 5](docs/rewrite-evidence/p00/2026-10-04-rw005-footprint-result.md).

P02 adds shared host, disk, pool and container observations, scoped inventory/history APIs, bounded SSE, and a native read-only dashboard. Its independent shadow package passed six disposable Debian VM acceptance groups with 6.25 MiB combined PSS. [P02 evidence](docs/rewrite-evidence/p02/2026-10-04-execution-tracker.md) distinguishes local acceptance from the pending Pi measurements and reference-host shadow installation. Wybie remains on the frozen Python build.

See [foundation recovery instructions](docs/p01-operations.md), [shadow installation instructions](docs/p02-operations.md), the [foundation ADR](docs/adr/0001-p01-foundation.md), and the [ownership/dependency audit](docs/rewrite-evidence/p01/dependency-boundary-audit.md). Browser access requires a configured HTTPS proxy. Mutations follow in P03–P04 on a disposable test host.
