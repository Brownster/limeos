# Stream A assistance integration candidate

Candidate branch `engineer/p04-a-assistance`, worktree `.worktrees/p04-a-assistance`.
Combined tested source: **`8b31f990103bdc7ce959048abba1b6c051fc847b`**, based on
main `7df3f7530865c31b7faefd1f7b12a25aa7655860`. Four completed assistance
branches were merged sequentially without conflicts. No new product edits were
made during assembly. Main merge/publication remains with the integrator.

| Included head | Contribution and handoff |
| --- | --- |
| `5dd0a2a97f2bf19417443fc442b2a2febc93fd98` | [A1 mount UI lifetime](p04-a1-ui-lifetime-handoff.md): owner/generation cancellation, approval/queue replay handling, read fences and browser commit-phase disposal |
| `0f7483c05fff7d36adbfdda5c436ff28f7adaf8c` | [Before-image preflight](p04-backup-capture-preflight.md): opaque original-source plan and component costs before private capture |
| `159556ecefb50a6414a907dd1748a9f3a0736781` | [Numeric scrub planning](p04-snapraid-scrub-planning.md): manual option intent plus complete source checks and existing content/parity requirement |
| `61965d17dde51d0541e1f55ca4563d6fd6ef06a1` | [Samba document editing](p04-samba-document-edits.md): bounded byte-preserving owned-block proposals, previous-render binding, collisions and section-boundary refusal |

The product tree matches each included branch exactly for its claimed paths.
Four `git diff --exit-code` comparisons passed. Only 14 product paths differ
from main: before-image source/tests, protection/scrub source/tests,
shares/document source/tests and mount UI/client/controller/tests. All shared
crate roots, contracts, workspace manifests/lock, Store/schema, policy/protocol,
read workers, routes, service and package inputs remain as on the base.

## Combined local validation

The [source-bound packet](rewrite-evidence/p04/2026-10-18-a-assistance-integration/README.md)
records the actual combined source, inputs and results:

- `cargo test --offline --locked -p limeos-domain -p limeos-backup-archive`:
  **331 checks passed**. This is 178 domain unit tests, 143 archive
  unit/integration tests and ten compile-fail doctests. No filtered or ignored
  cases are credited, and child process output/repeated runs add no cases.
- Strict all-target Clippy for both selected crates and changed-path Rustfmt
  passed. No whole-workspace, executor or installed test run is claimed.
- **129 frontend cases passed across eight files**, including 40 mount lifecycle
  cases. Original API assertions use pinned TypeScript output for the actual API
  module; only the import specifier is replaced in a scratch test entry. The
  other seven test files run directly, one at a time.
- `npm run build` passed TypeScript and the actual registered production UI
  bundle, including the mount view/controller: 22 modules. No scratch shell,
  backend or effect route was added for this build.

All checks ran sequentially on repository disk with one Cargo job/test thread.
Dependencies were reused at pinned versions; no install or dependency update ran.
Individual leaf evidence retains its original source/feature identity. Its input
checksums need that original checkout; the new packet binds this combined tree.
No local test total substitutes for installed acceptance.

## Integrator and qualification handoff

1. Review/merge this candidate under the existing shared ownership plan. The
   separately active `engineer/p04-mount-subphase-diagnostics` worktree was left
   untouched; its pending production and guest changes are absent here. Combine
   only its accepted commits, then freeze and record the resulting source before
   installed qualification. The accepted central B1 schedule work is also
   separate; this candidate adds no automation registration.
2. The existing test owner retains browser and disposable-guest A1 qualification.
   Its browser-fixture branch `engineer/p04-a1-browser-fixture` at `8c5b498` is
   not included or executed here. Reconcile it with the accepted candidate and
   run the installed API/UI/current-authority/CSRF, lost-reply/reconnect,
   disposable ext4 mount, refusal, guardian cleanup and restart/recovery matrix
   once at the final source. Local controller tests and an actual bundle do not
   prove browser commit ordering or installed mounting.
3. For A3, register scrub and Samba only through the accepted operation/claim/
   authority/supervision/journal path. Bind real retained physical/configuration
   owners, current approval and exact package/options. Scrub is an effect, not a
   read query. Samba needs complete include/registry/global semantics, fixed
   testparm, protected atomic apply/fsync, independent reload/share verification
   and a tested return path. Pure preview data supplies none of that authority.
4. For A4/A5, compose and actually reserve the complete simultaneous resource cost
   before capture. The 256-object before-image owner alone retains 258 FDs; the
   unchanged 256-FD service must refuse it or consume a separately reviewed
   bounded design. Encryption/provider/key recovery, complete managed inventory,
   online database snapshots, verified stack/service coordination, durable
   publication, actual restore/application/resume and installed crash/rollback
   remain required. The preflight adds no global reservation or durable receipt.

A1 installed acceptance, A2 lifecycle/loss handling, complete A3/A3b parity and
A4/A5 remain open. No defect, P04 or cutover gate closes. The historical inventory
61.401 ms p95 against 20 ms remains failed. No production Pi, service activation,
host effect, release publication or broad qualification suite ran here.

The subsequent [A3b preservation inventory](p04-remote-mount-preservation-inventory.md)
maps 30 remote-capability rows, 56 public symbols and seven synthetic
reconciliation cases. Its separate source-bound packet supplies contract input
for SSHFS, rclone S3 and the dedicated seedbox flow. It is documentation only;
the combined tested product and its local results above remain unchanged.
