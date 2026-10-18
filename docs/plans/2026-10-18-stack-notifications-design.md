# Provider-neutral stack notifications

Stream B continuation from main `7df3f75`. Preserve the frozen *arr event
normalization in `stack_notifications_service.py` at pi-health
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, without executing that repository.

Add a pure deployment notification planner plus a selective supplied-body
decoder. This fits B's existing leaf boundary while central schedule registration
continues separately. Extending the current alert state machine would confuse
external media events with observed host incidents. Rebuilding Mattermost setup
would make an optional transport a P04 prerequisite. Keep normalization independent
of transport, persistence and authenticated ingestion.

Preserve quiet/verbose policy, every frozen event alias, upgrade flags, severity,
movie/year and first-episode labels, file/release quality precedence, source
fallback, connection-test suppression and 500-character detail truncation. Carry
unknown/test/policy suppression as explicit outcomes. Treat titles and details
as untrusted text. Output carries no source identity, occurrence identity,
authorization or delivery receipt.

Admit at most 256 KiB before JSON parsing, 4 KiB per selected string and 128
episodes while retaining only the first, and 32 JSON container levels including
unknown fields. Reject invalid selected field types,
duplicate selected keys, invalid UTF-8, extra JSON documents and unsupported
numbers. Ignore unknown fields without retaining their data. Bound output source
to 128 characters and detail to 500 Unicode scalar values. Preserve printable
Unicode and multiline details; reject other control characters. Malformed input
is an error, distinct from an intentionally suppressed event.

Use fixtures derived from the frozen code and tests, including all aliases,
quiet/verbose differences, malformed/nested input and boundary cases. Run the
combined B leaf harness sequentially with disk caches and one Cargo job; no
installed host suite or frontend change is needed.

The handoff must retain pending token/credential migration, current-principal
configuration, central outbox admission, immutable event identities, retry and
ambiguous-delivery contracts. Never deduplicate by title/detail. External sends,
HTTP routes, Store/schema, package activation and UI registration stay with the
existing integration sequence. No notification channel is retired by this leaf.
