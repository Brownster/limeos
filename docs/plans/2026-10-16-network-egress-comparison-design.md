# Network egress address comparison

Continue Stream B from `7df3f75` with the explicit host/provider address comparison
left open by the network-group handoff. The frozen `network_group_service.py`
compares the two strings only during an explicit probe; absent/failed reads make
its `vpn_leak` boolean false. Preserve equality as an observable fact, while
keeping failed reads unknown and avoiding a VPN safety claim for differing IPs.

Prefer a separate `deployment::egress` domain/decoder leaf over expanding the
topology model with active probing. The existing network inventory describes
configured references. It cannot establish the provenance or outcome of an IP
probe. Live probing would require the integrator's bounded worker, fixed endpoint,
current authentication and protected namespace/instance binding; no collector,
command template, HTTP client or execution registration is added here.

Bind an expected Linux host boot ID, Engine identity and full container snapshot
(ID, image, start timestamp, running state). Host and provider observations retain
their own original sample times and outcome. Require exact identity, a running
expected provider, complete successful address results, the existing inclusive
90-second age bound, no future sample and at most five seconds between samples.
The five-second pairing bound is a conservative new diagnostic rule, not a claim
about the frozen implementation. All missing/failed/stale/mismatched/incomplete
or differently addressed-family pairs remain unknown, with independent source
states. No addresses enter the report unless both observations are usable.

Parse at most 64 raw bytes of supplied plaintext address output before trimming
ASCII whitespace. Accept one IPv4/IPv6 literal, normalize IPv4-mapped IPv6 to IPv4,
and reject unspecified, loopback, multicast and IPv4 broadcast literals. This is
address shape admission, not a complete public-routability classification.
Private, reserved and documentation ranges are not asserted to be public.
Reject JSON, HTML, hostnames, URLs, ports, zone identifiers, multiple addresses,
non-UTF8 and trailing text. Never retain raw output or an error body in results,
Debug or errors. HTTP decoding requires exactly status 200; container-exec
decoding requires a present terminal exit code equal to zero. A nullable exit
code, error status, timeout/truncated collection or parse failure supplies no IP.
Timeout/truncation are explicit domain outcomes for the eventual collector.

Report `same_address`, `different_addresses` or `unknown`, original sample times,
bounded target identity and individual source states. Always report VPN evidence
as `not_established`. Equality may warrant investigation; inequality does not
establish tunnel use, live namespace equality, leak absence or network health.
Canonicalize addresses again in the domain so hand-built typed inputs cannot
bypass mapped-address equality or invalid-address refusal. Bound the serialized
report to 4 KiB using a streaming size check.

Fixtures port the frozen same/different address behavior and regress missing
reads, nonzero/nullable exits, failed HTTP with valid-looking output, address
normalization, family mismatch, freshness/pairing boundaries, host/Engine/instance
changes and stopped providers. Run the combined B leaf suite and strict Clippy
with one local Cargo job and repository disk scratch. Shared roots, Store,
protocol, API/UI mounting, package activation and production Pis stay unchanged.
