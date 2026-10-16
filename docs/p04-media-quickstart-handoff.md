# Media quickstart preparation handoff

Engineer B, 16 October 2026. Branch `engineer/p04-media-quickstart`, base `f4514d4`.
Tested product source `684aa81a6a5a9117d34c07abcd057974e30ccfb1`. This continues
the indexer/download-client/settings/local-config/media/catalog lineage. Other B
branches retain separate source/results. No A or central runtime change was needed.

## Delivered

`domain::deployment::quickstart::prepare` produces a bounded preparation preview
for the frozen `media-server` bundle. It requires an existing typed storage
contract, exact stack/Engine/project target, typed options and optional catalog
observation. It performs no reads, filesystem/ownership effects, template rendering,
Compose operation, seeding, credential access or profile save.

- Preserve eight members in frozen order: VPN, Transmission, SABnzbd, Prowlarr,
  Sonarr, Radarr, Lidarr, Jellyfin. Keep all sixteen directory requirements: six
  libraries (including podcasts), incomplete downloads and nine completed-download
  categories. A Readarr/category directory does not install or support that app.
- Derive media/download/config/backup roots and numeric ownership from the
  validated storage contract, matching the frozen compatibility projection. No
  legacy default-root fallback is invented. Valid conflicting UID/GID requests
  are ignored with `managed_identity_overrides_request`; the contract wins.
  Invalid IDs, overlapping/template/traversal roots, derived path overflow and
  unsupported VPN-off choices refuse. Preserve path case and interior spaces.
- Match explicit `(app, stack)` catalog mappings within the exact target scope.
  A managed canonical mapping is `already_declared`, without proving current
  template/configuration equivalence. Unowned/operator entries, duplicate app
  mappings, noncanonical services and unmapped same-name declarations block.
  Do not adopt a service based on a 409 message. Unrelated services remain intact.
- New/existing selection belongs to the whole proposal. An existing stack stays
  existing when VPN is already present, and an empty registered stack stays
  distinct from absence. Failed, partial, future, stale or mismatched reads yield
  unknown with no target/install/profile candidates. Original sample time is
  retained; age 90 seconds is admitted. Declared services plus only missing
  members must fit the existing catalog's sixteen-service ceiling.
- A ten-stage dependency proposal adds source/storage review, quiescent client
  configuration before stack start, startup verification before seeding, loaded
  configuration verification before profile saving, and profile verification.
  These dependencies are **declarations only**. There is no event consumer,
  durable state machine, dispatcher, receipt verifier or executable transition.
  A terminal event/return-code policy is not implemented by a stage name.
- A version-two `ProfileDraft` contains only bundle/stack, VPN choice, selected
  timezone, managed identity, roots and fixed members. It has no arbitrary
  `values` dictionary, API/VPN credential fields or completion flag. It is a
  proposed format, not a saved profile or completed-work receipt. Names, paths
  and selected settings remain private data. Existing version-one files are
  neither read nor migrated here; P06 retains actual migration.

All members already declared means zero install candidates, while preparation
and every verification gate remain necessary. Fresh unblocked proposals can
contain a profile draft before any effect; never treat that draft as completion.
Unknown or blocked proposals retain directory requirements/workflow for review
but contain no install/profile/target candidates.

Bounds: sixteen declarations and installation mappings; 64 bytes per selected
name/timezone; 512 UTF-8 bytes per root/generated path; 32 KiB aggregate selected
text; 64 KiB streamed output. Timezone admission checks bounded ASCII syntax only,
not installed tzdata validity. Root comparison is lexical and proves no actual
mount, inode, symlink, writable state, UID/GID ownership or hardlink behavior.

## Integration and remaining work

Register the leaf after this B lineage. Shared roots/manifests/production lock,
Store/schema/protocol, API/UI mounting, package activation and publication are
unchanged. Existing source/installation ownership and completeness DTOs confer no
protected acquisition or authority.

Six gates remain unresolved for every outcome: protected sources/prepared target;
verified physical storage/numeric ownership; complete catalog templates/VPN;
current authority/claims/locks; before-image/quiescence/recovery; independently
verified workflow results. Bind complete current declarations, exact source/
template equivalence, physical absence/prepared targets and current evidence to
common reviews and complete sorted claims. The full frozen VPN template is still
outside the existing catalog subset (environment/env_file, capabilities,
healthchecks and nine ports versus its eight-port limit). This proposal does not
render, validate or bypass that limitation.

Coordinate directory effects with A's registered storage path. Resolve full
numeric process/file ownership and actual binds. Compose changes use the existing
managed-file path and must preserve operator content. Configure client files only
under verified quiescence and recovery; missing initial config needs a protected
bootstrap/generation/stop path, not an empty-file assumption. Reconcile uncertain
creation/start/seed/save before retrying through durable shared state.

The runtime must require explicit successful terminal status and independently
verify the full stack/provider before seeding; the frozen missing-terminal/null
return-code acceptance is not an execution rule to retain. Recollect and review
the existing library, settings, client and indexer proposals after startup.
Check loaded results before saving an atomic private profile, then verify that
save. Persist/acknowledge durable stage outcomes through the central Store;
this library supplies no transaction schema or replacement registry. Register
authenticated review/job/status API/UI centrally; GET causes no effect.

B1 still awaits the accepted durable Store contract/registration. Complete
quickstart execution/API/UI, full source/VPN support, recovery and installed
acceptance remain open. RW-042/P04 remain incomplete; no feature is retired.

## Evidence

[Packet](rewrite-evidence/p04/2026-10-16-media-quickstart/README.md): **186 passing
domain tests**, including 23 new quickstart cases and 163 prior domain cases.
Strict all-target Clippy, Rustfmt, helper Ruff and harness byte checks pass. This
domain-only slice does not rerun or credit the 39 decoders from the preceding
202-case packet. Product bytes were committed after checks without edits.

Review reproduced a capacity defect: nine unrelated services plus eight missing
members produced an impossible seventeen-service proposal. The failing focused
test and before-fix source are retained. The fix counts existing services plus
only missing members, including the exact sixteen-service boundary. The initial
185-case pass preceded that regression and is not added to the final count.

Frozen files were read/hashed only, never imported/executed. Fixtures port bundle
order/layout plus all three existing storage profiles. No decoder/API/UI/browser,
file/process/network effect, actual workflow/receipt/recovery, profile save,
package/ARM64/Pi, performance, footprint or soak qualification ran. Main and
production hosts are unchanged; the branch is local, unmerged and unpublished.
