# Approved container restarts

P03's restart path is available only on a disposable test host. Wybie continues to run the frozen Python application. Start/stop, logs and the Compose plan foundation remain phase work.

The standard package installs with `allow_restart: false` and an empty `managed_containers` list in `/etc/limeos/system-policy/container.json`. On the test host, an operator may set `allow_restart: true` and list the full 64-character IDs of the containers LimeOS may manage. Preserve the package's `core_uid` and read capabilities. Validate with `limeosctl check-ceiling /etc/limeos/system-policy/container.json CORE_UID`, then restart only `limeos-containerd`. Core and the assistant cannot edit this root-owned policy or open Docker's socket.

The shadow unit always includes the `read-only` argument and refuses a write-enabled ceiling at startup. Its accounts, sockets and receipt state use the separate `limeos-shadow` prefix. It must never be used for write qualification on the reference host.

Select **Restart** in the container list. The preview binds the full container ID, image and last start time and expires after five minutes. **Approve and restart** issues a one-use human approval and queues the durable job. Task callers can propose or submit an already approved plan; they cannot approve it themselves. The current grant is checked again before dispatch, and the executor independently checks its allowlist and the resource's fresh identity.

Recent operations survive browser reload. Progress comes from stored, owner-scoped job events. A queued restart can be canceled. In-flight cancellation returns a conflict, because the host effect may already have begun. An interrupted queue response can be retried with its original approval, plan and request key; it returns the existing job without another Engine call.

`succeeded` requires an accepted Engine response, a new running incarnation of the same container/image and a further executor inspection. `needs_intervention` retains the resource lock. Missing, prepared or ambiguous receipts never authorize replay. Inspect the job, executor journal and protected receipt before making a recovery decision. There is no automatic unlock command in this slice. Never delete the receipt database or reset job state to queued to retry an uncertain operation.

Core authority schema v3 stores receipts and verification in `/var/lib/limeos/core/core.sqlite`; executor receipts live in `/var/lib/limeos/executors/container/receipts.sqlite`. These are separate private stores. Older binaries refuse newer authority/receipt schemas; package downgrade alone is not a supported recovery across the migration.

The Engine adapter follows Docker's [versioned API](https://docs.docker.com/reference/api/engine/) and [restart contract](https://docs.docker.com/reference/api/engine/version/v1.40/#operation/ContainerRestart), negotiates a supported API, uses fixed selected inspection and restart endpoints, bounds response size/time and never retries a POST after losing its response.

Run local Rust checks with the pinned toolchain and `cargo test --workspace --locked`. The isolated browser fixture is `uv run tests/frontend/restart.py` after building the UI; it expects Google Chrome at `/usr/bin/google-chrome`. For real Engine qualification, build a trusted source/toolchain bundle with `uv run tests/privileged_vm/build_bundle.py --output /tmp/limeos-p03-build.tar.gz`, then use the existing VM runner with `--guest-script tests/privileged_vm/p03_guest.py --build-bundle /tmp/limeos-p03-build.tar.gz --build-output dist/p03-debian`. The runner requires KVM, uses a verified Debian image and destroys its own VM afterward. The test-only signing key is generated inside that VM and is never a release key.
