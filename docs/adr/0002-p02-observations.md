# P02 observations and shadow isolation

Accepted implementation of the existing P02 plan, 2026-10-04. Runtime code is Rust; React remains the browser UI. Python is restricted to build and test tooling.

The host and container executors own one collector each. The core reads bounded typed snapshots, caches them, and filters evidence against current identity grants before every HTTP response and SSE event. No public endpoint invokes a command or opens Docker. Root-owned ceilings separately enable host reads and container reads; legacy health-only ceilings default to denying the new reads. A fixed executor kind prevents granting one daemon the other daemon's adapter.

Docker uses GET-only Engine requests over its Unix socket, version negotiation, events with bounded reconnects, and periodic inventory reconciliation. Only selected fields cross the boundary. Stats run with four concurrent reads while a dashboard is active; absent accounting stays unknown. Mount and disk reads use fixed arguments and bounded output. Successful disappearance produces a missing resource; failed collection retains old evidence with an unavailable source state. These observations do not authorize future mutations.

Telemetry is disposable and separate from the authority database: minute samples, five-minute transactions, NORMAL synchronization, 31-day retention, bounded queues, bounded history ranges, and an 8 MiB database ceiling. Shadow history starts independently. Earlier history remains available in the frozen current build's System history screen; the UI and API explicitly explain this. No legacy files or databases are opened.

SSE delivers full authorized snapshots, including on reconnect, rather than claiming durable event replay. Readers have concurrency, age and byte bounds; identity is rechecked at least every five seconds. Slow readers cannot retain an unbounded event backlog. The normal HTTP connection lifetime deliberately ends streams, which EventSource reconnects.

Shadow packaging uses distinct package, service, account, configuration, socket and data names, plus loopback port 8004 and a separate HTTPS origin/login. Host observations can read kernel metadata but cannot change mounts, disks, packages or containers. Executor code has no mutation request variant. Reference-host installation and measurements still require the roadmap's operator-agreed quiet window; local disposable-VM evidence is recorded separately.
