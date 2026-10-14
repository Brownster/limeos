# Strict Compose JSON source admission

`limeos_executor_container::compose_source::admit_json_sources(operator, managed)`
accepts operator bytes and optional current managed-file bytes. It parses a closed
JSON subset and computes the supported effective project. It is pure, unregistered
library logic: no path read, environment lookup, command, Engine request or write.
The recognized Compose operator basenames may contain JSON; general YAML syntax,
anchors/tags, `.env`, includes and interpolation remain unsupported.

Each document is at most 64 KiB and requires a services map. The optional top-level
volumes map permits empty standard definitions only. All fields are closed.
Unknown/duplicate fields, escaped equivalent keys, nulls, wrong types, missing
required fields, trailing documents and invalid UTF-8 refuse. String visitors
check length before owned clones; map/array admission refuses excess before its
value is decoded or added. JSON escape scratch remains within the raw byte bound;
peak allocator/RSS is unqualified.
Every struct boundary explicitly requires object shape. Restart/network mode,
port protocol and mount kind explicitly require literal enum strings up to
sixteen bytes. Serde's alternative array structs and tagged enum objects refuse.

Supported services require a digest-pinned image. Optional numeric `uid:gid` is
canonical unsigned decimal, preserving the full u32 range. Restart/privileged/host
mode use typed fields. Missing scalars mean default or inheritance; explicit null
never clears them. Only host network mode is supported. Ports require explicit IP,
canonical string published port, numeric target and TCP/UDP protocol. Mounts use
long form with literal sources/targets and explicit read-only values. Binds require
`bind: {"create_host_path": false}`. Devices require literal
`source:target:rwm`. Ambiguous short syntax, alternate permissions, custom volume
drivers/external names, commands, secrets and arbitrary networks refuse.

Existing P03 normalization validates every service, including overwritten operator
settings. Every named-volume declaration is checked, including unused ones. The
operator must be valid independently; combined references must exist in the union
of declared standard volume names. No source body is retained or rewritten.

The supported [Compose merge rules](https://docs.docker.com/reference/compose-file/merge/)
preserve unrelated services and omitted scalar settings. Equal complete port keys
merge, new ports append, same-kind mount targets replace and distinct devices
append. A bind/volume kind switch or any inherited device-target overlap refuses;
the latter is a conservative supported-subset restriction. Combined validation
refuses wildcard/listener conflicts and mount/device target collisions. It never
silently truncates a project or treats an absent managed file as corrupt content.

Bounds apply to individual documents and their combination: sixteen services,
eight ports/mounts/devices per service, 128 declared named volumes and 192 KiB for
the counted borrowed compact summary. At a full set, equal port keys, same-kind
mount replacements and duplicate standard volume declarations can remain within
the bound. New excess entries refuse before collection growth.

`ComposeJsonSources` privately owns checked operator/current projects, declared
volume names and exact raw byte counts/SHA256 for each document. `summary()` borrows
serialize-only facts; it omits bodies, leases, descriptors, receipts and approvals.
Neither owner nor summary can deserialize. Hashes identify input bytes, without
establishing protected file freshness, management ownership or recovery custody.

`plan(desired)` invokes the [pure override compiler](p04-compose-override-compiler.md),
then decodes and combines its generated replacement against the checked operator
and requires the exact normalized desired project. Every existing unresolved
execution gate remains present. Removed managed services retain shutdown
requirements; empty desired projects grant no directory or volume deletion.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-source/README.md)
covers bounded/closed decoding, source hashes, supported merges and compiler
round trips. Six [shared fixtures](../tests/fixtures/compose-source/conformance-cases.json)
also pass an independent standalone Compose configuration parser, pinned to observed
AMD64 version 5.3.0 and [exact executable hash](../tests/fixtures/compose-source/conformance-pin.json).
The opt-in [qualification tests](../tests/qualification/compose/test_json_conformance.py)
run only version/config with explicit scratch files/project, cleared environment,
disabled `.env`/environment-file/interpolation/path resolution and a nonexistent
Unix daemon endpoint. Complete canonical outputs are retained. This qualifies
the selected parser/subset, not publisher trust, protected production launch,
other versions, ARM64 installation or broader YAML support.

See the [design](plans/2026-10-14-compose-json-source-design.md). Next is integrating
byte admission with the genuine [retained catalog/file/Engine binding](p04-compose-management-binding.md)
under its original freshness/read/resource limits. Principal/fresh executable
approval, physical consumers, locks, atomic writes/recovery, verified shutdown,
independent review and installed/performance/cutover gates remain open. Core/Store,
generated contracts, services, packages and the frozen signed ARM64 candidate are
unchanged. No Pi, SSH, production Docker, raw block probe or guest was used.
