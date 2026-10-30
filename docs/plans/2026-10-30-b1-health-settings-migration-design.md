# Plan frozen health settings before binding runtime policy

Implemented by `32265d4`. The
[handoff](../p04-b1-health-settings-migration-handoff.md) and
[local evidence](../rewrite-evidence/p04/2026-10-30-b1-health-settings/README.md)
record the exact parser profile, bounds and remaining integration work.

Baseline `4eea9c8` registers owned history but still folds observations under the
default alert policy. The [frozen inventory](../p04-frozen-notification-inventory.md)
contains supported settings and legacy values outside Rust's runtime bounds.
Using defaults would change their meaning; raising runtime limits would require
new admission/footprint qualification. Add a pure bounded migration planner
that either produces a complete representable policy or a typed review issue.

## Input and result

Add `automation::settings` in the domain crate. Accept only the selected daemon
environment fields (poll interval, failure threshold and required mounts), a
typed policy-read outcome and a caller-supplied trusted creation time for legacy
silences without one. Do not accept credentials, read files/environment, create
schedules or write state. A later protected migration adapter owns exact source
custody, destination references and current authority.

Retain frozen defaults, invalid-integer fallback, nonpositive clamping and
environment mount splitting. Preserve literal path case/spelling and remove
exact duplicate required mounts as the frozen policy normalizer does. A missing,
unreadable, malformed or JSON-null policy falls back to environment mounts. A
valid empty object overrides them with an empty list; a wrapped policy object
uses that object. Structurally invalid settings refuse with a bounded field and
reason, without returning raw input or exception text.

The candidate preserves category settings, exact literal silence keys, creation
and expiry metadata, and the trimmed reason. Support explicit calendar ISO-8601
timestamps with timezone offsets and whole-second expiry. Preserve creation
timestamp text, including fractional precision. An expiry that cannot map
exactly to the current whole-second runtime is a review issue, never rounded.
Other Python-accepted timestamp/integer spellings outside the explicit parser
profile also require review rather than fallback or reinterpretation.

Keep existing runtime caps: poll 30–2,678,400 seconds, threshold 1–16, at most
256 required mounts, 64 silences, 256-byte alert keys and 200-character silence
reasons. Bound policy input at 64 KiB and each selected environment value at
4 KiB before parsing. Mount paths must fit their eventual literal alert key.
Reject excess/unsupported values without truncation, case folding, path
canonicalization or a partially usable candidate. Output can be serialized for
review but grants no effect or persisted-policy authority.

## Validation and integration

Use the existing pinned 32-case frozen notification fixture. Cover the daemon,
policy, silence-expiry and policy-file precedence cases in this slice; external
destination selection/provisioning cases retain their separate inventory scope.
Add boundary regressions for oversized inputs, integer syntax/overflow, literal
paths, invalid structure, duplicate entries, timestamps/offsets/leap years,
fractional expiry, metadata preservation and exact exclusive expiry. Validate
the derived runtime policy through its real `fold_alert` behavior.

Run the affected domain suite, strict Clippy, formatting, repository checks and
actual contract drift under the existing one-job/repository-disk/3-GiB rules.
Preserve source-bound results separately from the registered-history evidence.
This is an intermediate migration prerequisite. Configured Store revision
binding, resource replacement, genuine collection, API/UI settings, production
supervision, external delivery and installed/cutover acceptance remain open.
