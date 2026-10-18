# A3b source inventory evidence, 18 October 2026

Inventory capture starts from assistance candidate
`59f39873ca8e127a644e73e123829c9846a98095`. Product bytes remain the combined
tested source `8b31f990103bdc7ce959048abba1b6c051fc847b`. Frozen reference:
pi-health `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`.

This is the required preserved remote-capability inventory and contract input.
It adds no Rust implementation, registered route/operation, credentials,
activation, migration, installed acceptance or defect/phase closure. No reference
module, helper or test was executed or imported. Deployed usage/configuration was
not inspected. Every matrix row remains `unimplemented_unqualified`; no
retirement has been accepted.

The [handoff](../../../p04-remote-mount-preservation-inventory.md) explains the
source disagreements, exact fields/defaults, replacements and central handshakes.
Current systemd documentation supports the unit-naming and automount concerns
linked there; it does not qualify any installed systemd/package version.

| Artifact/check | Scope/result |
| --- | --- |
| `inventory.json` | 30 preservation rows across generic SSHFS, S3/S3-compatible rclone, separate seedbox and shared/adjacent behavior |
| `coverage.json`, `coverage.md` | All 56 selected public methods/commands/handlers map to at least one row; 81 resolved source anchors; no missing selected surface |
| `anchors.json` | Exact file/start/end and symbol/file SHA-256 for each anchor; links into the read-only frozen repository |
| `source-extraction.json` | Literal provider schemas/defaults; saved record field expressions; helper constants/get-call defaults; 12 helper dispatch entries and ten declared HTTP handlers |
| `reconciliation-cases.json` | Seven synthetic examples linked to matrix rows; requested/effective options, credential/unit aliases, incomplete sources, literal paths and uncertain runtime/dependents |
| `audit-reference.py`, `audit-*.log.gz` | AST/static structural verification only; no frozen imports, evaluation of dynamic expressions or helper calls |
| `REFERENCE_INPUTS.sha256` | 18 frozen source/UI/test files; tests are read inputs, not test runs |
| `CONTEXT_INPUTS.sha256` | Four assignment/coordination/roadmap/defect documents |
| `format*.log.gz`, `ruff*.log.gz` | Audit helper formatted/linted with cached Ruff 0.17.0 |

The public-surface set is the nine concrete base methods, 11 SSHFS methods,
nine rclone methods, five seedbox methods, 12 helper commands and ten API handlers.
Abstract base declarations and private implementation details are not counted as
extra capabilities. Additional anchors bind relevant private helpers, discovery,
provider lifecycle and UI sources. Declared routes/decorators/dispatch are source
facts, not runtime registration/authentication or effect proof. Structural
coverage does not automatically validate the curated prose or every deployed
capability. Synthetic cases are acceptance inputs, not executed reconciliation
tests.

The audit parses source with `ast.parse` and applies `ast.literal_eval` only to
literal schema/default/constant data. It never uses `eval`, `exec`, imports from
the frozen repository, unit managers, mounts, network endpoints or helpers. It
also checks all 30 row IDs/statuses, source anchor resolution, saved-field
omissions, dispatch coverage and example references. The manifest records the
capture head, immutable product source, compiler-independent helper versions and
zero functional/installed test credit.

Run from the candidate worktree with repository disk scratch/cache:

```sh
UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp uv run --offline --no-project docs/rewrite-evidence/p04/2026-10-18-remote-preservation-inventory/audit-reference.py
```

The helper verifies the fixed frozen source and unchanged combined product tree.
A fresh run updates the capture-head metadata and artifact hashes; preserve this
committed packet for its original identity. It needs no Cargo/frontend suite or
external dependency fetch. Verify `REFERENCE_INPUTS.sha256` from the frozen
pi-health root, `CONTEXT_INPUTS.sha256` from the candidate root, and `SHA256SUMS`
from this packet directory with `sha256sum --check`.

The previous combined packet retains its 331 Rust checks, 129 frontend cases
and actual UI build at its own source. Documentation-only inventory work did not
repeat them. Its existing product-input and artifact hashes remain independently
verifiable. No new native, installed, API/UI, hardware, resource, performance,
crash-recovery, encryption or migration result is credited here. Main, active
central registration worktrees and production hosts remain untouched.
