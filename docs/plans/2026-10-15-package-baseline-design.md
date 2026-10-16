# B5 bounded APT baseline assessment

Engineer B, 15 October 2026. Follow HOST-001 branch `65ed86e` without editing
shared roots. B1 still needs the frozen central Store/runtime contract; A's mount
integration has active changes. Independent package assessment needs neither seam.

Port the frozen `limeos_packages.py` baseline policies into pure domain proposals:
present, absent, minimum Debian version, and pinned upstream version with an APT
hold. Retain critical/pinned pending-update reporting and feature-owned scope.
Unknown installed state must remain distinct from known absence. Unsupported
managers and self-updater controls remain explicitly blocked. This slice does not
install the frozen Python/provider manifest on Rust hosts or enable later providers.

Use a native bounded Debian comparator. Reusing the frozen best-effort comparator
would mishandle epochs, tildes and revisions; invoking dpkg from domain planning
would couple pure logic to a host process. Compare native results with the local
read-only dpkg oracle. Validate a conservative Policy-compatible version subset,
including epoch/revision, with epochs limited to 999999999 and no integer
conversion of potentially large numbers.
Upstream pins deliberately retain the frozen revision-insensitive match rule;
minimum versions use full Debian ordering. Revision-only pending updates stay
suppressed as in the reference; this is not security-update policy.

Accept bounded plain declarations/facts, never arbitrary operation argv or shell.
An opaque proposal lists exact offered versions and hold/removal steps, with
unresolved trusted-manifest/source/origin/approval/simulation/claims/migration/
space/recovery gates. Pinned targets must actually be offered; minimum/present
installations use the declared APT candidate. Downgrades, changing held packages,
pending removal selections and broken dpkg states require intervention. No plain
approval overlay, automatic unhold or unchecked unversioned install is provided.
An install-then-hold proposal requires independent verification between effects.

Add an executor read-only parser for a fixed successful single-package dpkg-query
projection: Package, Architecture, Status-Want, Status-Status, Status-Eflag, Version.
Only explicit not-installed/config-files states establish absence. Failed/empty/
truncated/malformed/multirow/mismatched queries are unavailable, not absent.
Half-installed/unpacked/trigger-pending/reinst-required states require recovery.
This parser launches no commands, accepts no protected owner, reads no filesystem
and registers no worker. Actual collection/origin/available-version resolution and
whole-worker budgets stay central. Report facts cannot become execution evidence.

Bounds: 32 unique manifest entries/facts, 8 offered versions per package, 128-byte
package names and versions, 32 KiB aggregate input text, 64 KiB report, 1 KiB query
output. Reject oversized fields before clones. Sort rows/steps deterministically.
DTO deserialization is a fixture convenience, not bounded IPC admission.

Focused tests cover frozen policy/revision/feature cases, missing vs unknown,
unavailable candidates, holds, downgrade refusal, incomplete dpkg states, parser
identity/architecture/error cases, unsupported managers/fields and bounds. Retain
exact source/fixture identities and logs. No package changes, production Pi work,
installed upgrade/interruption, ARM64, performance or broad qualification run.
Migration compatibility, safe self-update and recurring authorization remain open.

Primary references: [Debian version policy](https://www.debian.org/doc/debian-policy/ch-controlfields.html#version),
[dpkg-query](https://manpages.debian.org/bookworm/dpkg/dpkg-query.1.en.html),
[apt-mark](https://manpages.debian.org/bookworm/apt/apt-mark.8.en.html).
Frozen reference: `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`,
`limeos_packages.py`, `tests/test_limeos_packages.py` and
`agent_actions/packages.py`, read as source only.
