# B5 package baseline integration handoff

Engineer B, 15 October 2026. Branch `engineer/p04-package-planning`, based on
HOST-001 `65ed86e`; tested package code `94a6e33f28b270b2690cb6d4ff390243be1e63d3`.
This is an unregistered domain planner and read-only byte parser. It does not
complete RW-044, safe self-update, package execution or installed API/UI parity.

## Delivered behavior

`crates/domain/src/deployment/packages.rs::plan_baseline` accepts a proposed
approved manifest, selected package facts and feature lifecycle state as plain
data. Those arguments establish no trust or execution permission. The opaque
plan exposes only a bounded version-1 review summary and unresolved gates.

- Present/absent/minimum/upstream-pinned policies preserve the frozen baseline
  rules. A pin matches upstream text while deliberately ignoring epoch/revision,
  as the reference does. Minimum versions use full Debian ordering. These pins
  must never substitute for an exact LimeOS release identity or migration policy.
- Proposals name an exact offered version. Present/minimum use the current
  declared APT candidate; a pin chooses the highest offered matching upstream
  version. Equal-order spellings have a stable tie-break. Sort rows/offers for a
  deterministic summary. Unknown/unoffered candidates cannot become an
  unchecked unversioned install or arbitrary requested version.
- Missing facts, failed reads and unavailable repository data remain unknown.
  Explicit absence can yield an installation proposal; an unknown installation
  cannot. Broken packages, removal selections, held changes and downgrades yield
  intervention with no proposed steps. No automatic unhold is introduced.
- A compliant upstream pin needs a hold unless one is observed. An install/hold
  pair requires verified exact-version installation before the later hold. The
  `compliant` field describes presence/version policy only; row status also
  includes holds, unavailable controls and intervention.
- Critical/pinned pending updates preserve the frozen revision-only suppression.
  Unavailable facts, no candidate, no newer upstream and a pending candidate are
  distinct states. This reports baseline drift; it is not security-pocket policy.
- Feature scope is inactive unless its server-owned lifecycle is enabled. This
  does not activate AI providers or remove feature packages. Pip/npm/Claude
  managers and package-specific self-updater controls are explicitly blocked.
  The frozen Python/provider manifest is not installed on Rust hosts by this code.
- Strict tagged input variants reject extra fields, including on empty policy/
  observation variants. Fixture deserialization is not bounded IPC admission.

`bins/executor/src/updates/packages.rs::parse_dpkg` checks a successful fixed
single-package projection of Package, Architecture, Status-Want, Status-Status,
Status-Eflag and Version. It preserves installed version/selection, distinguishes
native amd64/arm64 and architecture-independent `all`, and refuses foreign
identity/architecture, malformed/multirow/truncated/non-UTF-8 output, warnings and
failed queries. Explicit not-installed/config-files states establish absence;
missing package/query exit 1 does not. Partial installation, trigger-pending and
reinstall-required states require recovery. No command or filesystem read runs.

Bounds: 32 unique manifest entries/facts, 8 offered versions each, 2–128 byte
Debian-style names, 128-byte versions, epochs at most 999999999, 32 KiB aggregate
input text, 64 KiB serialized summary and 1 KiB query output. Admission precedes
projected fact/spec clones. The comparator handles large digit runs without
integer conversion. Unsupported version syntax is unavailable/refused, not
best-effort ordering. Central process/worker/reply ceilings remain separate.

## Registration and execution requirements

1. Add `pub mod packages;` to the existing domain deployment module and register
   the domain root if absent. B2/B4 also supply `deployment/mod.rs`; combine their
   exports rather than replacing the file. This branch follows HOST-001 and adds
   `packages` beside `prerequisites` in its executor updates module. Declare the
   shared executor root only through integrator registration. Existing serde
   dependencies suffice; shared manifests and the production lockfile are unchanged.
2. Supply a root-owned manifest with a migration decision for every preserved
   baseline entry. Retire/replace Python-only dependencies explicitly. Resolve
   feature scope from current server lifecycle, not request JSON. Existing legacy
   pin approvals are not fresh executable authority and cannot be replayed here.
3. Collect complete fresh dpkg selections/status and APT candidate/offered versions
   through the common protected worker. Apply its time/FD/memory/output bounds and
   original lifetimes. Parser inputs and `Repository::Known` are plain facts, not
   evidence owners. Treat excess offered versions, partial enumeration, warnings,
   errors or incomplete origin data as unavailable; never silently truncate.
   Explicit absent rows need a genuine database observation or another accepted
   absence proof. A failed per-package query alone cannot establish absence.
4. Verify the signed repository origin and exact artifact through P01's trusted
   installed-package path. Run a bounded APT simulation, resolve complete package/
   dependency/service/storage claims and reject unexpected removals or expansion.
   The listed steps are review data; never translate arbitrary DTOs directly into
   privileged command lines. This leaf has no executor operation or effect method.
5. Bind complete manifest, facts, candidate/artifact and simulation to current
   principal/grants, durable review, fresh executable approval and common locks.
   Critical/removal/held/downgrade transitions need explicit central policy. No
   ordinary baseline repair may automatically clear a hold or override a refusal.
6. Require state-space/headroom, online DB snapshot and backup/recovery readiness,
   accepted migration compatibility and trusted previous-version availability.
   A supplies backup/storage facts; the integrator owns this shared recovery seam.
   These prerequisites remain unresolved, not caller-supplied approval booleans.
7. Execute registered fixed operations as durable `202` jobs, verify each outcome
   independently and reconcile interruptions/lost responses without blind replay.
   Verify exact installed version before applying its hold. Register status,
   intervention, pending updates and alert transitions in API/UI. Schedule adapters
   consume the same registered operation with current recurring authority.

## Focused evidence and remaining acceptance

[Local evidence](rewrite-evidence/p04/2026-10-15-package-baseline/README.md):
25 Rust tests (19 domain, 6 parser), 10 frozen policy cases and a 35-version matrix
of 1,225 read-only dpkg comparisons, strict Clippy, Rustfmt and harness checks.
The matrix includes epochs, tildes, leading zeros, revisions, punctuation/case,
stable backports and long numeric runs. It is a bounded corpus, not a proof of
all dpkg inputs or installed package compatibility. Reference Python was read as
source only. HOST-001 tests were not rerun and retain their earlier source identity.

Qualification owner still needs registered API/UI permission/error/reconnect,
trusted package collection, candidate/origin changes, hold behavior, exact-version
installation, APT simulation/dependency safety, unavailable/low space, migration
refusal, online DB recovery, interrupted upgrades/downgrades, pending-state alerting
and recurring authorization. No package/service/host mutation, broad suite,
hardware, ARM64/Pi, performance or cutover qualification occurred here.

Stream B's central queue remains: B1 `485206b` (Store/runtime/API/UI), B2 `5e275ca`
(shared read cohort/API/UI), B4 `c712486` (catalog execution/recovery), HOST-001
`65ed86e` (trusted collection/fixed repairs), then this follow-up baseline slice.
B4 includes B2; this branch includes HOST-001. The independent leaves do not
close their registered milestones. A's active mount files were left untouched.

Primary references: [Debian versions](https://www.debian.org/doc/debian-policy/ch-controlfields.html#version),
[dpkg-query](https://manpages.debian.org/bookworm/dpkg/dpkg-query.1.en.html),
[apt-mark](https://manpages.debian.org/bookworm/apt/apt-mark.8.en.html).
