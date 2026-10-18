# Supplied egress address fixtures

`comparisons.json` preserves the frozen matching/differing host/provider address
fact while making missing reads unknown. It includes canonical IPv6 and mapped
IPv4 equality plus family mismatch. `responses.json` covers supplied terminal
status, raw plaintext and malformed/error-body cases. These synthetic examples
make no request to the addresses they contain.

Use the existing [combined Stream B harness](../../stream-b/README.md), one local
Cargo job at a time with repository disk scratch. The actual domain and adapter
module lists include `egress`, so no additional wrapper or dependency is needed.
All sample identities/statuses are ordinary supplied data, not protected owners,
receipts, endpoint evidence or a verified VPN result.

See the [handoff](../../../../docs/p04-network-egress-handoff.md) for the pending
protected collection and API/UI requirements.
