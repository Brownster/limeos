# CopyParty configuration and observation design

Stream B, based on `53ec484`. This slice covers RW-042's existing CopyParty/tools
row with pure configuration previews and supplied read adapters. The frozen
reference is `/home/marc/Documents/github/pi-health` at
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`: `tools_service.py`,
`tools_manager.py`, `pihealth_helper.py` and their tools tests.

The reference defaults are `/srv/copyparty`, port 3923 and empty extra arguments.
It merges stored keys, treats read failures as defaults, saves before applying,
tests payload existence as installation, and starts a unit without an explicit
service account. Its arbitrary arguments and unverified latest-release download
cannot become executable Rust inputs.

## Chosen boundary

Add domain `deployment/tools` and a supplied-byte adapter of the same name.
Configuration admits a literal absolute path and a nonzero 16-bit port, preserving
case and internal spaces. Missing keys in a valid legacy object use the frozen
defaults. Verified file absence is distinguishable from unreadable, malformed,
oversized or unsupported configuration. Unknown/duplicate keys, coercions and
nonempty legacy arguments produce typed diagnostics without retaining raw text.
Paths containing ambiguous components, control characters or surrounding spaces
are refused explicitly; no filesystem normalization or traversal occurs.

Project configuration, fixed legacy payload presence and fixed service state
independently from supplied observations. Each keeps its original timestamp;
future, unavailable and older-than-90-second facts withhold values. A supplied
successful, complete systemd property capture can distinguish active, inactive,
failed, changing, masked and missing services. Failed or truncated commands never
become stopped or absent. Payload presence describes a legacy regular file, not
trusted installation. Neither active service nor matching saved settings proves
which configuration is loaded. No service URL is inferred from request headers.

A configuration preview compares validated desired fields with a fresh readable
configuration, fresh present payload and usable service observation. Missing
configuration can use the explicit defaults but still needs saving. Equal stored
fields never mean an applied result. Every preview retains unresolved package
provenance, service account/access policy, storage/port checks, current authority
and locks, atomic save/apply/recovery and loaded-result verification gates.
The preview contains no command, unit file, download location or write artifact.

Implementing installation now would cross shared package/authority/recovery
ownership. Reusing legacy helper JSON would preserve ambiguous booleans and raw
errors. The selected supplied-fact boundary exposes these limits for registration.

## Delivery and checks

Provide a cached authenticated GET handler under the B-owned deployment route
leaf, defaulting to unavailable until the integrator supplies the backend. Reads
use the existing cache and current grants; they never collect or configure.
Keep production route aggregation, Store/schema, worker/protocol, root manifests,
frontend shell and activation unchanged. Supply exact mounting requirements.

Fixtures cover the default and stored layouts plus invalid ports/paths,
unsupported arguments, read/command failures, duplicates, private diagnostics,
freshness boundaries and desired-versus-applied regressions. Run the combined
local B harness, strict Clippy and formatting sequentially using disk-backed
repository caches. This yields no installed, Pi, browser, recovery, performance,
defect-closure or phase-completion credit. UI mounting remains a follow-up.
