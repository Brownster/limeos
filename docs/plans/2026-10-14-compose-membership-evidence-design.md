# Read-only authenticated Compose membership

Base: local main `4119b3c`. Declared storage impact explicitly lacks stack
ownership. Add a retained Engine metadata prerequisite before any stack-aware
claim or stop planning. No stack operation or file authority is included.

## Choice and trust boundary

Extend the genuine double-inspection Engine collector with an internal optional
Compose projection. Existing advisory and retained storage/PID interfaces keep
their reports and parsing behavior. A new unregistered fixed-endpoint factory
requires the real UID-0 caller, `/run/docker.sock`, existing root socket/peer
policy and closed CombinedV1 admission. No endpoint, UID, clock or numeric limit
selector is public. Reusing cached dashboard project labels or caller-imported
JSON would not establish authenticated, complete current membership.

Project/service/one-off/working-directory/ordered-config-file metadata comes from
the selected inspect Config.Labels fields. Preserve complete original container
identity, including stopped and mountless consumers. Explicit null/empty labels
or only unrelated string labels are non-Compose; any Compose namespace marker
requires every supported mandatory field. Unknown labels/environment are never
reported. Bound project/service names to 64 bytes, canonical absolute paths to
512 bytes/16 components, and ordered config files to four distinct paths. Refuse
partial/malformed/ambiguous metadata instead of treating it as unmanaged.

One project name must have exactly one working directory and ordered config-file
selection within complete Engine membership. Multiple replicas, services and
one-off containers remain distinct members. Do not infer a trusted filesystem
stack, stack resource key, executable plan or safe shutdown from Engine labels.
Actual protected Compose files and management/stack identity remain prerequisites.

## Retention and bounds

Use the original authenticated endpoint descriptor, selected storage/PID facts,
complete membership, original wall/monotonic five-second lifetime and shared
spent HTTP/live-payload ledger. Parse and compare Compose metadata in both initial
inspection passes and every fresh revalidation. Changes in membership, Engine,
PIDs, mounts, selected labels, working directory or config order refuse. Complete
combined storage/Compose compact JSON must fit the existing 48 KiB report ceiling.
Charge strings/vectors and temporary comparison overlap under the existing 2 MiB
live ledger; keep 16 complete/8 running/32 selected-mount, response/shape and 2 MiB
lifetime HTTP ceilings unchanged. This is no peak RSS/cgroup headroom proof.

The Compose path additionally rejects duplicate JSON object keys, including
escaped equivalents, before decoding the repeated value. Preserve existing
standard/CombinedV1 JSON semantics for callers that did not request Compose.
An owner-local validation guard permanently invalidates the new owner after
failure, cancellation or reentrancy; original base access cannot bypass it.
Snapshots are serialize-only and cannot reconstruct owners. Collection/validation
still need independent supervision for synchronous kernel work.

## Local proof and remaining work

Use owned Unix listeners with explicitly private unit credential/clock fixtures,
not workstation Docker. Test complete running/stopped/mountless/unmanaged
membership, replicas/one-off, ambiguity, malformed/partial fields, literal names,
path/file limits, secret omission, duplicate keys, changes across both inspection
passes and reacquisition, original-age refusal, cancellation/poisoning, complete
counts, report and lifetime byte limits, and unchanged old interfaces. A public
nonroot refusal must occur before the canonical endpoint is touched. Root/peer
negative fixtures must send no HTTP before authentication.

Run exact-source focused/full Rust, strict feature all-target Clippy, fmt,
contracts/repository and local ARM64 harness checks; retain lossless failed and
accepted transcripts with source/artifact hashes. Work/build/cache stays on disk;
only precompiled existing storage fixtures use small owned cleaned tmpfs scratch.
Reserved backup and external qualification paths are untouched. No Pi, SSH,
guest, real Docker, block probe, frozen Python, RPC/route/schema/service/package
activation or cap change. Independent review, installed/root/namespace behavior,
physical consumers, actual stack files/claims/locks/approvals/effects/recovery,
P04/defect/performance and cutover gates remain open.
