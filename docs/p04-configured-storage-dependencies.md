# Declared pool and protection dependencies

`limeos_domain::configured_dependencies::configured_storage_dependencies`
combines a validated storage contract, pools, SnapRAID configuration and bounded
Docker declarations into a deterministic read-only preview. It accepts `at` to
check the original declaration age; it acquires no clock or live observations.

Each device row preserves its complete configured identity and reports:

- Pool IDs, mount roots, enabled state and branches on that configured device.
  Branch order is preserved. Multiple branches on one device produce one pool
  relation rather than duplicate container routes.
- Protection drive IDs, enabled state and the existing planner's generated
  source/data/content/parity requirements, including content files on parity
  drives and the correct multi-parity filenames.
- Full container identity/running facts, bind or named-volume source, destination,
  writable flag and the direct or pool route that produced the match.

A source at or below a pool root maps to every configured member. An ancestor
source such as `/mnt` or `/` conservatively includes pool and direct device
routes. A direct branch source includes only its declared device relation;
reading one branch does not imply consumption of every pool member. Path
components and case are literal. Disabled pools/protection and stopped containers
remain visible because configuration cannot prove their runtime absence.

The function validates all inputs before returning a preview. Missing/ambiguous
UUID, path or role relationships and malformed, future or stale declarations
refuse. Expansion is limited to 512 container matches across the entire request,
counting every device/destination/route. A 513th match refuses the whole result;
there is no clipping. Existing validators bound configurations and declaration
counts/strings. Devices, pool/drive records and matches are sorted, while branch
and protection-requirement order remains meaningful. Errors contain fixed field
names/codes. The preview preserves numeric media ownership, Engine identity and
the original collection timestamp.

This preview establishes declared relationships. Its structs hold no descriptors,
live lifetime, storage locks, claims or approval. An empty container list cannot
prove safe unmounting: named-volume aliases, symlinks, nested mounts, running
namespace paths and block backing need protected evidence. Shares are an explicit
missing input. The coordinator must compose all fresh physical/runtime owners,
retain their original lifetimes and bind complete configuration/claims/approval
before an effect. No planner result can replace that work.

[Local acceptance](rewrite-evidence/p04/rw041-configured-dependencies/README.md)
uses the four frozen layouts plus targeted refusal and propagation fixtures.
It adds no operation, dispatcher, schema migration, HTTP/IPC/CLI/UI registration,
service/capability change or host probe. DSK-001, MNT-001/MNT-002, RT-001,
SRA-001/SRA-002, BKP-001 and P04/cutover stay open. The prior full-inventory
61.401-ms result against the 20-ms budget remains failed.
