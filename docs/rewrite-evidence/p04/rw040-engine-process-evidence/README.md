# Authenticated Engine and PID library proof

Implementation is `a216c101414056db203db5bae2752329808ebfa0`, after approved design `e18c47b`, on assignment base `b742f24`. Only four executor-container source/test paths changed. External archive custody and executor-storage process-module ownership remain untouched. [The interface](../../../p04-container-engine-process-evidence.md) defines the trust boundary and precise bounds. [Independent review](independent-review.md) gives GO for the exact four source hashes and interface document; it inspected implementer results without rerunning tests or contacting a host.

The library retains a no-follow socket descriptor, authenticates kernel UID before HTTP, double-inspects complete declarations/PIDs and freshly reacquires those facts within the original wall/monotonic lifetime. Production peer policy is root-only. Future protected integration must select canonical `/run/docker.sock`; today's advisory `/var/run/docker.sock` adapter and wire API are unchanged. Socket activation's peer PID is not independently proved daemon identity. Reports cannot construct owners or authorize operations.

## Local validation

| Check | Actual result |
|---|---|
| Focused executor-container | 45 passed, zero failed/ignored; 24 new tests |
| Rust workspace | 315 passed, zero failed/ignored |
| Formatting / strict all-target Clippy | Passed |
| Generated contracts / repository boundaries / diff | Passed; wire contracts unchanged |
| Cached offline cargo-deny | Passed, existing duplicate-version warnings retained |
| Cached cargo-audit, no fetch | Passed; 1,290 cached advisories and 227 locked packages |

[Checks and source binding](checks.json) records commands, outcomes and log hashes. [The source manifest](source-manifest.json) binds runtime, fixture and build inputs to this commit's Git blobs and working bytes. No dependency/version/lock change occurs. `logs/*-tool-output.txt` preserves complete captured UTF-8 test tool output, including terminal blank lines; the independently reviewed `focused-tests.txt` and `workspace-tests.txt` retain their earlier normalized terminal whitespace and original review hashes. Both forms carry the same result. Empty fmt/diff logs accompany successful tool exits. No unchanged frontend/harness/native tests were redundantly run locally; full exact-source CI owns those qualification gates.

The first production-only crate check passed. A test tool attempt was aborted after a **677.5-second approval wait**, with no returned test output. The next real test compile failed because `unwrap_err()` unnecessarily required Debug on an opaque evidence owner; a test-only pattern match fixes it. [The original diagnostic](logs/test-compile-attempt-1.txt) and [attempt metadata](attempts.json) remain. An intermediate 41-test suite passed before the four reviewer-requested boundary cases were added; its raw output was not separately saved and is not represented as a source-bound final acceptance log. Final source, 45-test log and 315-test workspace log are the accepted proof.

## Acceptance scope

The 24 new discovered test cases cover these independent behaviors:

| Behavior | Test names / proof |
|---|---|
| Complete stopped/running/mountless membership and empty Engine | `complete_gets_include_stopped_and_mountless_consumers_without_private_fields`, `genuinely_empty_double_enumeration_is_the_only_empty_success` |
| Exact Linux PID forms/states and uniqueness | `running_pid_requires_a_non_system_linux_integer_and_stopped_requires_zero`, `duplicate_running_pids_refuse_the_complete_result` |
| Second-pass and fresh-reacquisition changes | `second_inspection_pid_snapshot_mount_membership_or_engine_change_refuses`, `reacquisition_binds_pid_full_snapshots_mounts_membership_and_engine` |
| Actual kernel credentials before HTTP | `mismatched_kernel_peer_uid_sends_no_http_bytes`, `production_root_peer_rule_rejects_actual_unprivileged_credentials_before_bytes`, `root_policy_never_relaxes_to_an_unprivileged_fixture_owner` |
| Socket replacement / symlink / wrong mode | `replaced_socket_object_invalidates_the_original_owner_before_more_http`, `socket_symlink_and_world_writable_socket_refuse` |
| Counts, amplification, complete maximum membership | `count_mount_and_report_limits_refuse_without_truncation`, `all_sixty_four_running_consumers_are_admitted_without_mounts`, `aggregate_body_budget_spans_both_inspection_passes` |
| Partial response / cancellation / request deadline / collection deadline | `partial_body_produces_no_owner_or_empty_success`, `cancellation_aborts_the_connection_task_and_peer_observes_eof`, `request_timeout_also_aborts_the_connection_task`, `complete_collection_deadline_refuses_responses_each_below_request_timeout` |
| Original issue time, revalidation expiry, wall rollback, monotonic expiry | `declarations_preserve_issue_time_before_socket_acquisition`, `fresh_revalidation_timestamp_does_not_renew_original_lifetime`, `final_reacquisition_response_cannot_cross_the_original_wall_expiry`, `initial_collection_checks_wall_rollback_and_age_at_its_final_response`, `unchanged_wall_clock_cannot_extend_monotonic_expiry` |
| Advisory compatibility | `advisory_inventory_keeps_its_missing_pid_behavior`; all existing storage/action/log/receipt cases remain passing |

Fixtures use private scratch Unix listeners and actual local credentials. Successful selected-fact parsing uses private test policy, not relaxed production UID rules. The root-specific refusal cases require an unprivileged account; the local account is UID 1000. Clock jumps and socket-acquisition timing are private deterministic injection; the four-second deadline and cancellation/EOF cases use real local async transport. No real Docker daemon, host process owner or installed combined collector runs here.

## Remaining gates and effort

No HTTP/RPC/CLI/service registration, root ceiling, mount, data write or process signal was added. The separately assigned process owner still needs review and kernel liveness/root/namespace binding. Integration must compose fresh Engine with retained namespace destinations, sources, UUIDs/backing and pool/protection/share propagation. Single-flight independently killable workers, protected host context/ancestors/capabilities, installed acceptance, complete shared claims, approval and effect-time verification remain pending. Synchronous filesystem/JSON work is not preempted by async timeout. Mount/fstab effects, unmount/runtime-loss shutdown and live restore remain gated. No defect-register or cutover row closes.

No SSH, Pi/wybie test, expired-window reuse, production package/workload action or frozen Python development ran. Historical proof and the **61.401 ms / 20 ms failed inventory row** remain unchanged. Native fixtures/CI below do not replace frozen `8acac40` footprint measurements or qualify a matched Python comparison, bare-metal pages, soaks, eight disks, media or assistant workloads.

The provisional human estimate is **12 hours**, review at **18 hours (150%)**, within P04's unchanged 320-hour/480-hour thresholds. Active agent time and human engineering time were not independently instrumented; no measured human hours are claimed. The recorded approval suspension is separate from compiler/test timing and CI infrastructure. Local validation collection finished around 20:10Z on 2026-10-07; later documentation/CI time is separate.

## Full CI

[Exact-source workflow 37679994369](https://github.com/Brownster/limeos/actions/runs/37679994369) tests `a216c101414056db203db5bae2752329808ebfa0`. Completion evidence will separately identify native AMD64/ARM64 and fresh installed AMD64 artifacts. Committed historical VM results in the workflow upload cannot qualify this source. Native unit/package CI and the existing installed paths do not exercise this unregistered library against root Docker.
