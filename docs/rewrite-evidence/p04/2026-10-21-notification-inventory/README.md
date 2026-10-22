# Frozen notification configuration inventory

Implementation `476e2d5` supplies a selected-definition reference harness and
[32 configuration fixtures](../../../../tests/fixtures/automation/frozen-notification-settings.json)
from `pi-health@80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. The
[inventory](../../../p04-frozen-notification-inventory.md) maps preserved flows
and remaining Rust/API/UI/migration requirements. `REFERENCE_INPUTS.sha256`
identifies all 25 source blobs. No deployed configuration or credential was read.

| Check | Actual result |
| --- | --- |
| Frozen-rule capture | 32 cases captured with in-memory dependencies; source modules/app/helper were not imported |
| Focused reference tests | 11 passed before Ruff formatting; fixture bytes match that test input |
| Final lint | Ruff 0.17.0 E4/E7/E9/F/I/UP/B passed on the committed capture/test inputs |
| Formatting | Applied successfully; the intentional input changes are not a test result |
| Full reference suite | Refused before execution on 22 October by the unchanged 3-GiB floor; no pass counted |
| Frozen checkout | Pre-existing untracked document preserved; no checkout changes from capture |

`source-status.json` separates pre-format test input hashes from final lint
hashes. No post-format replay, full reference-suite pass or complete source-match
test claim is made. Raw records/output include the initial import-order lint
failure and its correction. The full-suite refusal occurred before its guard
created a test record. Checksums cover every other file in this packet.

Credential values are projected to presence/slot descriptions. Actual selected
definitions run against supplied memory paths/repositories and fixture values;
the capture never starts a daemon, probes a host, sends a webhook or loads a
model provider. Python execution of selected trusted source is a reference
harness, not a general-purpose security sandbox.

This packet establishes source rules and remaining obligations. It does not
qualify the unmerged Rust settlement draft, external delivery, installed
API/UI/restart behavior, ARM64/Pi operation or cutover. No channel is retired.
