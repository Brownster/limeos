# Configured storage graph handoff

Branch `engineer/storage-dependency-graph`, based on pushed main
`af94f19ac21f9da14b207439808ffb33c87e8e10`. Qualified code
`c62ecf0f7e78bb3009673684df9418da56542564`; design `a8799f8` precedes code.
Worktree `.worktrees/storage-dependency-graph` and builds/caches/logs are on disk.
The branch stays local and unmerged; the Pi bundle frozen at `016ba49` is unchanged.

Delivered an additive pure graph composition of the existing container/pool/
protection and Samba planners. Full existing device rows are reused, share
requirements join by complete metadata, and desired share settings are stored
once with exact-name references. One 512-match bound applies across expanded
share and container routes. Original Docker time, literal case, ordered
branches/protection paths/share configuration, numeric ownership, disabled,
stopped and read-only consumers survive. Unmapped shares remain explicit.

Local acceptance passes **533 Rust cases**: 525 unit/integration and eight
compile-fail doctests. The integrated 521-case baseline gains 12 graph cases.
Workspace excluding executor-storage passes 432; executor-storage separately
passes 101 with owned small auto-cleaned tmpfs scratch. Strict workspace/all-target
Clippy, fmt, generated contracts and repository checks pass. Local byte/source
hashes and commands appear in `checks.json`/`source-manifest.json`; evidence
hashes cover packet files except the hash index itself. Raw terminal blank lines
are retained through lossless `.txt.gz` copies for `storage-fixtures` and
`test-rest-final`; [compression bindings](raw-transcript-compression.json) retain
their original hashes and Git snapshot. Compiler concurrency is one, test concurrency two. Historical
preliminary failures remain in ignored cache and are excluded from totals.

| New regression | Established behavior |
| --- | --- |
| four_existing_layouts_match_independent_combined_goldens | All four frozen layouts have independent device/share/combined-count goldens |
| existing_storage_rows_are_reused_without_format_or_identity_changes | Existing full pool/protection/container rows, engine, time and UID/GID remain equal |
| disabled_stopped_read_only_consumers_and_protection_remain_visible | All disabled/read-only/stopped consumer relationships remain; stopped identity time stays valid |
| ancestor_routes_include_both_direct_and_pool_consumers_without_deduplicating_routes | `/mnt` expands to 58 share/container routes on the 14-data-plus-parity layout |
| ordered_branches_complete_device_metadata_and_numeric_ownership_survive | Serial/full device metadata, UID 1234/GID 4321 and B/A branches remain intact |
| original_timestamp_and_named_volume_identity_are_not_renewed_or_rewritten | Original age 98, named volume/driver and destination survive; stale/future times refuse |
| share_flags_users_and_comments_are_stored_once_with_exact_name_references | Desired settings are retained once despite 14 pool members; every name resolves uniquely |
| literal_case_component_siblings_and_unmapped_paths_remain_explicit | Case variants, sibling prefixes and outside paths remain explicitly unmapped |
| enumeration_reordering_keeps_device_graph_stable_while_share_order_is_preserved | Device/container enumeration does not alter graph rows; desired share order stays meaningful |
| every_invalid_input_refuses_the_composition_without_value_leaks | Invalid contract/pool/protection/share/Docker facts refuse without echoing values |
| mixed_511_512_and_513_matches_apply_one_global_limit_to_individually_valid_previews | Mixed exact limits pass; individually admissible previews totaling 513 refuse the whole graph |
| empty_share_input_keeps_prior_rows_but_never_skips_validation_or_mutates_inputs | Explicit empty share input preserves old relationships and still validates all configuration |

The four layouts and frozen source provenance come from unchanged
`tests/fixtures/pool-protection/`; the new golden is hand-written from their
configured identities plus synthetic Docker/share declarations. No Python source
was changed or executed. No root/mount/namespace/Samba/remote operation was run.
Existing kernel library tests use private scratch/owned children only.

The [public-interface mapping](../../../p04-storage-dependency-graph.md) and
[design](../../../plans/2026-10-12-storage-dependency-graph-design.md) record the
integration seam. Only one additive domain export touches a shared file. Core,
backup recovery, executors, schema, services, APIs, UI and generated wire contracts
stay unchanged. Protected source/namespace composition, actual share completeness,
complete claims, approvals, locks, effect-time checks and recovery remain open.
Empty/unmapped rows never authorize unmount or runtime-loss shutdown. No defect,
P04 or cutover gate closes; the 61.401 ms/20 ms inventory failure is unchanged.

Estimate eight human engineering hours, review at 12; actual human/agent effort
is unmeasured. P04 remains 320 hours with a separate 480-hour review.
