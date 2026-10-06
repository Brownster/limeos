# Fresh container storage declarations

RW-040 exposes `GET /api/v1/storage/container-dependencies` to a current human administrator with the explicit `storage_manage` grant on `storage:configuration`. It returns Docker's current storage declarations, including stopped and unmanaged containers. The route accepts no query parameters or caller-supplied paths. POST is refused. GET creates no intent, approval, job, claim, audit event or Engine mutation.

Core asks the existing container executor to collect directly from Engine. It does not use dashboard caches or open Docker itself. The request carries only the operation and protocol version. The existing `allow_container_read` ceiling must permit collection; enabling host or storage reads does not grant Docker access. The same route works through an isolated read-only shadow container executor. Authority is checked before collection and again before returning private paths, so revoking the storage grant during a read prevents disclosure.

The response contains an `inventory` and its SHA-256 `digest`. Inventory version 1 contains the Engine identity, collection-start timestamp and a list of consumers. Each consumer includes the full container resource ID, image ID, running state, start time and selected mounts. A bind source contains its absolute host path. A volume source also includes its bounded name and driver. Every mount includes the container destination and read/write flag. Validated tmpfs entries have no host source and are omitted. Environment, labels, raw inspection output and Engine error text are excluded.

Collection reads the full container set, inspects each container twice and requires identical selected facts. Final enumeration and Engine identity must match the opening reads. These checks detect observed changes; they cannot create a transaction against external Docker clients. Missing, malformed, unsupported or oversized evidence fails the entire read. An empty successful inventory is returned only when both complete enumerations establish an empty Engine, never as a substitute for unavailable evidence.

| Bound | Limit |
| --- | --- |
| Complete collection | Four seconds inside the five-second core RPC deadline |
| Individual JSON request | Two seconds |
| Engine identity JSON | 128 KiB |
| Enumeration or inspection JSON | 512 KiB |
| Containers | 64 |
| Mounts per container | 64 |
| Aggregate selected host-source mounts | 256 |
| RPC frame | 64 KiB |
| Evidence age | Five seconds from collection start; future timestamps refused |

Each process admits one such expensive storage collection at a time. It refuses overload without queueing unbounded readers. Limits cause refusal rather than clipping or pagination. A valid failed executor receipt preserves its specific error: observed changes produce a conflict, unavailable evidence produces an unavailable response, and denied ceilings remain forbidden. Unsupported protocol versions and unsuccessful receipts without a specific error remain unavailable.

The pure `declared_container_storage_dependencies` helper compares contract mountpoints with declared source paths by components. Exact paths, descendants and ancestor binds such as `/mnt` or `/` count as possible dependencies. Stopped containers count because a later start may use their source. Literal case and spaces are preserved; `/mnt/storage-old` does not match `/mnt/storage`.

These declarations do not establish complete physical dependency coverage. Symlinks, bind aliases, replaced source paths, named-volume backing mounts, nested filesystems and actual running-container mount namespaces still require protected kernel evidence. Pool branches, protection paths and shares require separate collection and propagation. A successful read or digest authorizes no mount, unmount or fstab effect. Those operations remain gated until complete fresh evidence, derived shared claims, approval and effect-time verification are qualified.

[Installed acceptance](rewrite-evidence/p04/rw040-dependencies/2026-10-06-acceptance.md) binds the frozen source and exact AMD64 packages. Native ARM64 CI builds and installed AMD64 VM results are separate from installed ARM64/Pi qualification and comparable Python footprint measurements. The Python checkout and production Pi remain unchanged.
