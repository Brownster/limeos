# Configured dependency fixtures

`expected.json` is an independent golden projection for the four existing
`pool-protection/layouts.json` cases. Two synthetic declarations are used:
container `a` binds `/mnt/storage/Movies` to `/data`, and container `b` binds
`/mnt/downloads/Complete` to `/data`. Their Engine/process timestamps and IDs
are synthetic. These are lexical configuration fixtures, not live filesystem,
container namespace or operation evidence.

Single disk maps `a` directly to storage. Separate downloads maps `a` to storage
and `b` to downloads, with backup unused. Protected pool maps `a` through pool1
to all 14 data devices; parity has protection requirements and no declared
container route from that pool source. Ordered multi-parity maps `a` to d1/d2
through pool1 and preserves separate p1/p2 protection paths. Existing planner
requirements total 33 and 12 for the two protected layouts.

The existing source manifest binds 17 Python files at frozen commit
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. This slice reuses those configurations
without changing their import, rendering or source provenance. Additional Rust
tests exercise disabled/stopped facts, ancestors, case/components, named volumes,
multiple destinations, same-device branches, ordering, invalid inputs and request
expansion. None of these fixtures asserts physical dependency completeness.
