# Committed A1 fixture readiness, 21 October 2026

Design `03bed9a`; setup integration `3d5a15f`; final wait-deadline correction
`c6d278b`; direct guest-client import `6761d09`. These changes affect disposable
test setup only. Product binaries remain the frozen `c6932b2` candidate from
the [exact task witness](../2026-10-21-a1-task-witness/README.md).

The tracked planning fixture waits for its fixed reader socket and a successful
authenticated inventory read before its strict later API assertions. Only the
exact busy response is retried; malformed/permanent refusal, command failure,
byte/attempt/time bounds fail. A1 settles the fixture's queued udev work and
waits for its genuine daemon's workers to exit naturally before the first
planning RPC. Parent identity, proc bytes, children and time are bounded. Udev
stays running. Every runtime comparison and effect refusal remains unchanged.

The reproduced boundary regression prevents an empty worker observation at the
deadline from being accepted. A lint failure for malformed-response exception
type and its correction are retained. The initially incorrect guest-client
module reference was corrected in review; it has no installed result credited.
Eight new pure cases cover these setup boundaries. All **73 reference tests**
pass on combined source `3e494a0`; the actual fixed client imports and parses.
Pure tests do not establish kernel readiness or an approved mount result.

The exact guest was frozen from `6761d09`, with **479 non-test product input
hashes matching** the prepared build and verified binary/package hashes. Its
384-MiB runner changes only non-build guest RAM; service ceilings and assertions
are intact. Admission observed 3,820,457,984 bytes available against an additional
3.5-GiB start threshold. The unchanged **3-GiB host stop floor** then fired at
3,217,817,600 bytes, exit 143, before the suite emitted output. Inputs remained
unchanged. The runtime correction remains unqualified, and no owned test VM
remained running afterward. More starting headroom is needed for another run.

The package candidate's UI predates the new schedule panel. The later UI work
does not inherit installed qualification from this interrupted guest. The next
gates remain a real approved mount with independent verification, replay and
restart, then the broader A1 fault/resource/ARM64 matrix and A/B, P05/P06 work.
No Pi or frozen application was contacted, and no push occurred.

`inputs-3.json` and the archived guest modules bind the delivered fixture.
`checks/` preserves the passing, failed and stopped attempts separately.
Private fixture keys, binaries and disk images are excluded; existing candidate
records retain their hashes and local paths. `SHA256SUMS` covers every other file.
