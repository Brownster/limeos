# Samba local regression cases

All cases use supplied configuration bytes and typed fixtures. No Samba process,
service, filesystem share, Docker engine, mount or remote host is used. The
four layout fixtures retain their original synthetic qualification scope.

| Domain test | Established behavior |
| --- | --- |
| all_four_existing_layouts_match_hand_written_dependency_goldens | Single disk, separate downloads, 14-member protected pool and ordered multi-parity layouts match independent declared-route goldens; full device and UID/GID metadata survives |
| section_preview_keeps_share_order_literal_spaces_case_and_all_flags | Ordered sections preserve literal UTF-8/space/case paths, comments, user order and flags; disabled sections are omitted; configuration round-trips |
| disabled_and_read_only_shares_keep_disabled_pool_requirements | Disabled and read-only share consumers retain all 14 members of a disabled pool |
| ancestor_and_direct_paths_are_conservative_without_prefix_or_case_aliases | `/mnt` reaches 29 direct/pool routes; a direct branch reaches one device; sibling prefixes and case variants remain unmapped; parity paths keep their role |
| multiple_branches_on_one_device_coalesce_but_keep_branch_order | Repeated device membership coalesces while its B/A branch order survives; device enumeration order does not change the preview |
| every_pool_is_propagated_with_its_own_id_and_branch_mapping | Two pools retain separate IDs and membership; pool record reordering produces the same requirements |
| empty_shares_still_validate_storage_and_pool_configuration | Empty input cannot hide invalid ownership, unknown/parity pool branches or disabled recursive pools |
| nested_device_or_pool_roots_refuse_ambiguous_lexical_ownership | Nested physical declarations and pool/device root overlap refuse the whole preview |
| names_are_bounded_reserved_and_case_collisions_refuse_even_when_disabled | Section injection, reserved names, 64/65-byte names and disabled case-colliding duplicates refuse |
| unsafe_paths_cannot_inject_sections_or_expand_substitutions | Root, traversal, controls, substitutions, config delimiters, noncanonical whitespace and 512/513-byte paths refuse; literal UTF-8/interior spaces remain supported |
| comments_and_local_user_expressions_are_checked_before_rendering | Comment limits, groups/domains/substitutions/embedded lines, case-colliding users, 32/33 users, 32/33-byte usernames and 2048/2049-byte lists are checked |
| share_count_and_expanded_requirement_limits_refuse_whole_requests | 64/65 share records and 511/512/513 direct requirements exercise exact global limits without clipping |
| pool_expansion_counts_towards_the_same_whole_request_limit | 36 pool shares produce 504 requirements; the 37th reaches 518 and refuses the whole result |
| findings_do_not_echo_paths_users_comments_or_section_names | Error field/code data cannot echo imported credential-like values |

| Observations test | Established behavior |
| --- | --- |
| frozen_sample_and_imported_share_fixtures_match_section_goldens | Frozen sample/import test output matches hand-written generated sections; source bytes, booleans and disabled settings survive |
| each_frozen_default_is_reported_and_explicit_values_do_not_default | All six optional per-share defaults and missing root list have fixed findings; explicit false/empty values retain their meaning |
| renderer_trimming_is_preserved_but_names_and_paths_are_never_repaired | Frozen comment/user-list outer trimming is retained; padded names or paths refuse |
| malformed_unknown_duplicate_and_sequence_shaped_inputs_refuse_without_values | Missing/malformed/invalid UTF-8, unknown/duplicate fields, coercions, nulls and sequence-shaped structures refuse without partial configuration or value leaks |
| every_boolean_requires_a_json_boolean_without_truthiness_or_null_defaults | All four booleans reject string/integer/null/list coercions |
| invalid_disabled_share_blocks_whole_import_without_partial_configuration | A later disabled share's unsafe path or case collision cannot return an earlier partial share |
| byte_and_record_bounds_accept_exact_limits_and_refuse_over_limit | Exact 64 KiB and 64 shares/384 default findings pass; one byte or record over refuses |
| unsupported_user_groups_domains_substitutions_and_embedded_lines_refuse | Unsupported user syntax, embedded controls/non-space whitespace and duplicate/empty users remain explicit refusals |

These 22 new cases advance RW-041 planning coverage and the declared-share input
needed by DSK-001. They do not establish actual Samba semantics under global
inheritance, account access, installed config acceptance, physical identity,
unmount refusal, runtime-loss shutdown or crash-safe share writes. UI-003 still
needs bounded API/UI partial-result handling. No defect or phase gate closes.
