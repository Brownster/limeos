# Namespace roots in storage observations

The genuine kernel probe behind the stack qualification captures an `nsfs`
mount whose root is `net:[4026532191]`. Storage's bounded host parser, process
parser and source-table validation currently treat every root as an absolute
filesystem path, refusing an otherwise ordinary Docker host.

## Interpretation and authority

Keep mount rows, original root strings, raw table digests, device/mount identities
and existing wire layouts intact. Introduce a borrowed, internal classification:
ordinary filesystem path or a validated namespace kind/inode. Recognize only
the current kernel namespace kinds and canonical nonzero inode spelling for
`nsfs`; arbitrary relative roots and mismatched filesystem tags still refuse.
The actual `/` entry must remain a directory-style absolute root. Mountpoints
remain absolute and existing source-path canonicalization stays in force.

Use the same classification in Standard/Combined host parsing, process parsing
and source-table validation. Namespace names remain display/kernel metadata;
they do not supply source filesystem paths, physical backing, namespace owners
or effect authority. Direct or nested namespace sources remain unsupported by
physical dependency/destination binding. Do not drop their rows or references
to manufacture an apparently complete physical map.

The host-consumer effect check explicitly refuses bound mount-namespace handles
whose contents this version does not independently inspect. A name such as
`mnt:[inode]` is not a substitute for a retained namespace descriptor/table.
This preserves the previous unknown-consumer barrier while allowing unrelated
network namespace rows. It does not resolve A1's userspace-task revalidation
conflict or qualify named mount-namespace handle discovery.

## Bounds and verification

Keep table byte/row limits, decoded root/path limits, requested allocation
budgets, cancellation/deadlines, descriptor ownership, process identity,
namespace/root corroboration and whole-table revalidation unchanged. The
classification borrows strings and does not enlarge retained mount rows.
No public imported report can reconstruct evidence ownership.

Regress the captured network namespace row in both host profiles and process
profiles. Test canonical kind/inode spelling, zero/overflow/unknown kinds,
ordinary relative roots, invalid selected roots and exact/excess input bounds.
Exercise source-table validation and confirm namespace references cannot become
physical dependencies or destination paths. Keep the uninspected mount-namespace
effect refusal explicit. Run focused storage checks, appropriate fixture-backed
storage tests and strict Clippy, then freeze a new installed candidate as needed.

The preceding B2 package keeps its source-bound acceptance. New storage code
inherits no installed/ARM64/effect acceptance from it. All work stays on repository
disk with one local build/test command and the 3 GiB host-memory stop floor.
No Pi or production storage activation is implied; A/B, P05 and P06 remain open.
