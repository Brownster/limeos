# LimeOS

Rust replacement for the frozen Python application at `/home/marc/Documents/github/pi-health` (`80593b2`). All development happens here. The deployed reference host, wybie, still runs Python at `618ce92` until cutover.

The [delivery roadmap](docs/plans/2026-10-04-rust-rewrite-roadmap.md) defines the sequence and acceptance gates. The [defect register](docs/plans/2026-10-04-rust-rewrite-defect-register.md) carries 50 requirements into the new implementation.

P01's Rust foundation is implemented: core-owned identity and durable authority, bounded HTTP/RPC, local enrollment, separate executor services, generated contracts and Debian packages for x86-64 and ARM64. The [P01 evidence](docs/rewrite-evidence/p01/2026-10-04-execution-tracker.md) records the tests and remaining qualification gates. The existing Python build remains frozen and deployed unchanged.

The disposable Debian VM passed fourteen acceptance groups. After authentication, core plus both executors used 5.19 MiB combined PSS without swap; five restarts reached readiness within 132 ms. These are VM measurements, not Pi results. The [P00 footprint experiment](spikes/footprint/README.md) separately [passed on the reference Pi 5](docs/rewrite-evidence/p00/2026-10-04-rw005-footprint-result.md).

See [build/install/recovery instructions](docs/p01-operations.md), the [foundation ADR](docs/adr/0001-p01-foundation.md), and the [ownership/dependency audit](docs/rewrite-evidence/p01/dependency-boundary-audit.md). Browser access requires a configured HTTPS proxy; executors accept only health in P01. Dashboard observations and parity operations follow in P02–P04.
