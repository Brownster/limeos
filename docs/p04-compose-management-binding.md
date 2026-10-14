# Retained declared Compose management binding

`limeos_executor_container::compose_management::inspect_compose_binding` binds one
explicitly selected stack registration to genuine retained management catalog,
protected Compose file and authenticated Engine owners. It takes a catalog path,
stack ID and cancellation callback. This is an unregistered read-only library;
no command, RPC, route, Core/Store job, service or package activation invokes it.

The [P03 catalog](p03-compose-planning.md) holds planning snapshots and approvals,
without working-directory or Engine mappings. A separate version-one management
catalog supplies those declarations. Its [synthetic fixture](../tests/fixtures/compose-management/catalog.json)
declares an expected Engine ID and one to eight stack registrations. Each names
a P03-compatible stack ID, Engine project, working directory, recognized operator
basename/raw SHA256, explicit managed-file state/raw SHA256 and one to sixteen
distinct supported services. No catalog is installed or synthesized by default.
Root-controlled configuration establishes declared management intent, not a
principal permission, human approval or executable plan.

Every registration is validated, including unselected entries. Stack IDs and
services retain the P03 lowercase name grammar and 48-byte limit. Engine project
names use the selected label grammar with a 64-byte limit; expected Engine IDs
are nonempty ASCII alphanumeric/dot/underscore/colon/hyphen, at most 128 bytes.
Directories and derived operator/managed paths use the existing file selection
rules. Duplicate stack/project identities, duplicate services, duplicate or
component-nested working directories, malformed digests and unsupported names
refuse the whole catalog. Literal case and meaningful file/service order survive.

Catalog decoding refuses unknown and duplicate fields, including escaped key
equivalents. String and collection visitors refuse excess before owned string or
vector growth; a ninth registration or seventeenth service is refused before its
value is decoded. The explicit `absent` managed state uses an empty struct variant
so additional fields refuse too. Internally tagged variant buffering and parser
scratch still lie within the raw file bound; peak allocator/RSS is unqualified.

`catalog::inspect_management_catalog` requires effective UID 0 before namespace,
root or target access. It reuses the [protected file reader's](p04-compose-file-evidence.md)
fixed PID-1-relative nsfs/root policy and descriptor-relative no-follow traversal,
protected directories and regular single-link files. Catalog paths are canonical
absolute UTF-8 names up to 512 bytes/16 components. Classify O_PATH before a
nonblocking read open and compare descriptors before content reads. Missing,
symlink, FIFO, directory, hardlink, unsafe mode/owner or invalid content refuses;
none becomes an empty map and no file is written.

The catalog is at most 16 KiB. The original four-second wall/monotonic lease,
two-second cooperative validation pass, 1 MiB lifetime requested-read ledger and
4 KiB compact metadata report remain fixed. The owner retains root, namespace,
ancestors and file, with at most 18 descriptors. It keeps checked declarations
and metadata/raw hash, then drops the raw parser body. Revalidation freshly walks
names and compares fresh/held identities and hashes, then rewalks ancestors and
the named leaf after content checks. Directory timestamps remain outside identity.
Every failed, cancelled, reentrant or unwound check permanently poisons that owner.

The binding selects the registration's directory, operator name and managed-file
inclusion; callers cannot override child UID, endpoint, root, clock, file hash,
service set or numeric ceilings. Genuine fixed-endpoint [Engine membership](p04-compose-membership-evidence.md)
must match the expected Engine ID. The catalog and file owners must have the same
root and mount view. Actual raw file hashes, directory and ordered config-file
names must match exactly. Every selected project member must name a registered
service. A differently named project overlapping the selected root or file paths
refuses rather than acquiring the selected stack identity.

`ComposeBindingEvidence` retains the actual three child owners. `snapshot()`
returns a borrowed serialize-only report with the selected `stack:<id>` declaration,
registration, catalog metadata, original file facts, complete Engine membership
and bounded indices for selected members. Full container identity, replicas,
one-offs, stopped and mountless rows remain in the original Engine report.
Unrelated and non-Compose rows remain visible; their stack files are not bound.
A registered stack may have zero observed members, without safe-absence or
deletion authority. Reports and owners cannot deserialize or reconstruct leases.

An original four-second composition window starts before collection. Collection
revalidates before returning; subsequent checks bracket genuine Engine revalidation
with catalog/file checks and repeat the mapping. Child issue times, expiry and
spent bytes never renew. Cancellation is checked at phase/read boundaries; dropping
in-flight async work closes its Engine connection and poisons the binding. Timeout,
failure, concurrency, reentrancy or unwind permanently invalidates it. A first
check cannot restore readiness after another refuses. Child-owner accessors are
absent; borrowed reports remain past facts after expiry or invalidation.

Preflight at most 38 retained descriptors: 18 catalog, 19 selected files and one
authenticated Engine socket. Count the complete borrowed compact report against
80 KiB without cloning child snapshots or allocating a report buffer. Existing
Engine 16-complete/eight-running/32-mount and HTTP/payload/report bounds stay intact.
Catalog/file owners keep their own original spent read ledgers. These counts
qualify no combined-worker resource model, service headroom or installed behavior.
Synchronous kernel work still requires independent supervision; multiple files
and Engine checks do not create a writer transaction.

[Local acceptance](rewrite-evidence/p04/2026-10-14-compose-management/README.md)
covers catalog rules, protected scratch descriptors and genuine Engine owners
collected through authenticated owned Unix listeners. Successful fixtures use
explicitly private root/UID/clock substitutions. Public nonroot factories refuse
before callback or target/canonical Engine access. The retained failed first run
records the `absent` variant issue and an all-stopped fixture's nonzero first PID;
the variant is closed and the fixture now has zero PID, with production guards
unchanged. See the [design](plans/2026-10-14-compose-management-binding-design.md).

This binds declared selected management identity, not actual combined Compose
semantics, independent daemon/physical-host identity, complete physical consumers,
principal permissions, fresh executable approval or acquired Store claims.
YAML/`.env`/includes/interpolation, custom overrides and empty-stack discovery need
later supported adapters. P03 previews/approvals, locks, effects/recovery, worker/
probe integration, independent review, root/foreign-namespace/installed/resource/
performance qualification, P04/defect gates and cutover remain open. Reserved
recovery paths, generated/wire schemas, service guards and the frozen signed ARM64
candidate are unchanged. No Pi, SSH, real Docker, raw block probe or guest was used.

The [pure override compiler](p04-compose-override-compiler.md) now checks whether
supplied typed operator/current/desired projects can be expressed by a replacement
managed JSON file. It remains separate from these retained owners; actual supported
source admission and executable approval still require later integration. The
[strict JSON source adapter](p04-compose-json-source.md) now validates supported
byte slices and compiler round trips; it supplies no descriptor freshness or
Engine binding until composed with these genuine retained owners. The later
[retained JSON source binding](p04-compose-retained-json.md) now performs that
composition with original time/read budgets and complete service association;
deployed configuration equivalence and execution authority remain separate.
