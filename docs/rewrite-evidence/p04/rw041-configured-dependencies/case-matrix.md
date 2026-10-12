# Declared dependency acceptance cases

All cases are local pure/synthetic configuration tests; none proves a live
filesystem dependency, installed namespace mapping or an operation effect.

| Case | Required result |
| --- | --- |
| Single disk | Direct media consumer; no pool/protection relations |
| Separate downloads | Independent media/downloads consumers; backup unused |
| Protected pool | Media consumer expands to 14 data members; parity is separate; 33 protection requirements |
| Ordered multi-parity | Pool consumer expands to d1/d2; p1/p2 filenames remain distinct; 12 requirements |
| Direct branch path | Only the declared device gets the container route |
| Disabled/stopped/read-only | All possible declared consumers and their flags remain visible |
| `/mnt` and `/` ancestors | All direct and pool routes count conservatively |
| Case/component boundaries | Similar prefixes or changed case create no lexical match |
| Multiple branches on one disk | Branch order retained; one pool route per destination |
| Named volume/multiple destinations | Name/driver/source identity and each destination preserved |
| Record reordering | Identical output while branch order and original collection timestamp remain meaningful |
| Invalid contract/pools/protection | Entire preview refuses; errors omit imported values |
| Disabled recursive/pool protection paths | Still refused |
| Stale/future/invalid/duplicate Docker facts | Refused before a result |
| 511/512 expanded matches | Complete results accepted |
| Further direct or pool expansion | Whole request refused, never clipped |
| Input/ownership preservation | Inputs unchanged; numeric UID/GID copied exactly |

The existing kernel-storage tests additionally need suitable filesystem fixtures.
The relocated Btrfs scratch reports a stat device different from its mount-table
device and directory link count 1. Those fixtures are refused by unchanged
storage identity checks. A single identical source test passes on tmpfs. The
storage suite therefore uses a small owned auto-cleaned tmpfs scratch directory;
worktrees, compiler/build caches, general test scratch and logs remain on disk.
Its sampled scratch size is recorded separately from process memory and is not
a universal peak-memory bound. Btrfs physical backing remains an open gate.
