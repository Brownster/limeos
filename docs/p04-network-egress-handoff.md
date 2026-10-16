# Egress address comparison handoff

Stream B continuation from main `7df3f75`, on
`engineer/p04-egress-comparison`. This implements the supplied-data comparison
boundary in the [design](plans/2026-10-16-network-egress-comparison-design.md).
The [evidence packet](rewrite-evidence/p04/2026-10-16-network-egress/README.md)
binds exact source and fixture hashes to local checks. Runtime collection and
API/UI registration remain pending.

## Delivered behavior

`domain::deployment::egress::compare` compares supplied host/provider IP samples.
`executor-container::deployment::egress::{parse_http_address,parse_exec_address}`
decodes bounded supplied plaintext after checking terminal metadata. These
functions perform no HTTP request, container exec, namespace entry or host write.

The report distinguishes same address, different addresses and unknown, with
independent source state and original sample times. An unavailable, failed,
nonterminal, timed-out, incomplete, malformed, oversized, stale, future or
wrong-scope observation supplies no comparable address pair. Both families must
match after IPv4-mapped IPv6 normalization. Both original samples must be no more
than 90 seconds old and within five seconds of each other, inclusively.

Host boot UUID, Engine identity and the complete container snapshot must match
the expected target exactly. A changed ID, image, start time or running state
invalidates the provider sample. A stopped expected provider keeps comparison
unknown even when supplied address samples match its identity. Validating these
ordinary input shapes does not establish their physical or protected provenance.

The decoder admits at most 64 bytes before whitespace trimming and returns fixed
outcomes without raw output. Only HTTP 200 and a present terminal exit code of
zero permit parsing. It accepts one IP literal, normalizes mapped IPv4, and
rejects unspecified/loopback/multicast/broadcast addresses, malformed data,
hostnames, URLs, ports, zones, multiple addresses and non-ASCII surrounding
whitespace. Other private/reserved/documentation ranges remain address literals;
the helper does not classify them as public. The serialized report is capped at
4 KiB through a streaming check.

Every report carries `vpn_evidence: not_established`. Matching addresses are a
diagnostic observation, not proof of a leak; differing addresses are not proof
of tunnel use or leak absence. There is no `vpn_leak: false` success fallback.
Missing reads in the frozen implementation must not become an all-clear result.

## Integration requirements

1. Register these B-owned modules only when accepting the corresponding shared
   collection contract. No shared root exports, manifests, lock, Store, executor
   protocol, route aggregation, frontend shell or package activation changed.
2. Collect only after an explicitly authorized bounded probe request. Use a
   fixed approved HTTPS endpoint and transport; caller-selected URLs, commands,
   shell snippets and tools are outside this interface. GET reads cached results
   and cannot trigger a probe. Recheck current session/grants and bind the actual
   host network namespace, Engine and container process lifetime before and after
   collection through the existing protected ownership path. A Core process's
   own namespace, a Docker label or a supplied target string is insufficient.
3. Enforce whole-worker time, byte, descriptor, retained-output and concurrency
   limits before passing borrowed bytes here. Per-response limits do not grant
   independent budgets per provider. Prove complete body/stream consumption and
   actual terminal status; a valid-looking truncated prefix is incomplete, and a
   missing exit code remains unknown. A supplied status cannot prove TLS,
   endpoint, namespace, current instance, process success or response completeness.
4. Preserve the trusted original sample times and host boot binding. Re-rendering
   or caching cannot refresh them. Require the same approved endpoint/probe
   profile for both samples; differing address families remain unknown. The new
   five-second pairing limit is a conservative rule pending installed collection
   qualification, not evidence that the frozen probes met that bound.
5. Mount an authenticated cache-only API/UI result with explicit unknown/error
   states, pair age and honest comparison labels. Clear retained views on
   expiry/logout/principal or grant changes and 401/403. Never style a differing
   address as a verified VPN, healthy namespace or permission to start/update a
   stack. Live namespace and route/tunnel checks remain separate evidence.

## Local acceptance

The combined B harness passes 453 Rust cases: 301 domain, 127 adapter/host-policy,
19 API and six runner. The 29 new cases comprise 20 comparison tests and nine
decoder tests; their tables include ten comparison scenarios and 16 responses.
Strict all-target Clippy and Rustfmt pass. The initial test log retains one
test-name style warning; the final run and Clippy use the corrected name.

Fixtures are synthetic ports of frozen source `80593b2`, never live endpoint or
Engine captures. They cover frozen equality/inequality, missing/failing probes,
nullable exit status, malformed/sensitive output refusal, normalization, family
differences, identity changes, stopped targets, time/size boundaries and direct
typed-input bypass attempts. No frozen Python is imported or executed.

Existing API/runner cases run as part of the combined harness; no new API or UI
implementation is supplied. Root domain/executor crates compile as dependencies,
without credit for their full unit suites. The earlier 89 frontend cases retain
their own integration-packet identity and are not rerun or credited here.
Protected collection, installed probe behavior, permissions, worker budgets,
ARM64/performance/soak and Pi qualification remain with the existing owners.
This closes no P04 phase or defect-register row and does not establish cutover
readiness. The first Stream B runtime milestone still awaits the shared
automation Store contract and registration described in the B1 handoff.
