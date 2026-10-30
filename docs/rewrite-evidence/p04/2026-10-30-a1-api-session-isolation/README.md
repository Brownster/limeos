# Isolate fixture HTTP connection lifetime

Fixture source `97b117e9b8f9c1f6150a074cfc5b83695a81eda0` holds one HTTP
connection from positive planning through approval, queueing, observation and
verified replay. Only the already-verified Core restart boundary creates a new
session. Closed/replaced transports fail the fixture without reconnecting or
retrying a POST. An ambiguous queue response retains the reviewed action ID for
read-only failure evidence. Ordinary session/Origin/CSRF fields and response
bounds use the same request helper; no production admission rule changes.

All **102 reference tests** pass natively. Real loopback HTTP tests show the
existing individual calls opening two connections and a session retaining one
socket inode through POST plus three reads. They cover authentication/CSRF
header preservation, closed/malformed/oversized replies, no POST retry, socket
replacement refusal and explicit new-session construction. These are transport
tests, not Core authentication or mount acceptance. Scoped Ruff, formatting,
repository hygiene and diff checks pass. The precommit native record's 687
input hashes match this fixture commit.

One fresh canonical 1024-MiB guest reused unchanged schema-12 packages
`0.4.11+fdcontext.1`, built from
`f6f583e69518a732bcb8a34b2d09c7f4e3b7e23d`. The manifest records four
test-only differences from that build, all 31 guest modules and the unchanged
canonical runner. Product, package, binary, UI, service and safety identities
retain their original source provenance.

The guest completed setup and all three negative API cases. Its positive plan
request then returned HTTP 409. Root recorded one new task: PID 7087, TID 8333,
start 24285, namespace `(4, 4026531841)`. PID 7087 matches the authenticated
fixture Docker peer. The positive session recorded one completed HTTP exchange
on socket inode 33711 and zero reconnects. A completed HTTP exchange is not a
successful plan: **no positive plan or job was created**.

Execution was not reached, so the Core polling-FD hypothesis remains untested
in the installed path. This run neither identifies the earlier FD's purpose nor
establishes production reliability under normal process/FD activity. More
fixture warmup cannot establish that reliability. Review the complete-consumer
capture and refused-gate recovery design while preserving all safety invariants;
do not ignore changing tasks, exempt Core or extend original lifetimes.

The detached guard completed 17:57:14–18:01:20 UTC on 30 October, exit 1,
unchanged inputs and no sampled memory stop. Minimum host availability was
5,157,785,600 bytes, above the unchanged 3-GiB floor. Its process was observed
absent after completion; VM scratch was removed. No previous disk/action state
was reused. Holly's Pi was untouched and nothing was pushed.

Compressed original guard, fixture, package and failure records plus exact
reproduction sources preserve the joins. `source-match.json` records the narrow
result; `SHA256SUMS` covers every other file. Private keys, packages, binaries
and VM disks are excluded. A1/API/UI acceptance, ARM64 and broader A/B/P05/P06
cutover gates remain open.
