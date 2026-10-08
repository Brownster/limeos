# Independent foundation refusal-probe correction review

Verdict: **GO for the narrow fixture correction below**, with full corrected-head CI still required. No daemon, Rust runtime, policy, capability, service ceiling or acceptance rule is relaxed.

The settled correction is committed and pushed as `2fd74209e1238c1374837ab431ef670020726dc2`. It changes only the two fixture/test files below. Its full workflow `37746766303` is in progress at review; no success is claimed here.

| Settled file | SHA-256 |
|---|---|
| `tests/privileged_vm/guest.py` | `70c4fc420bdc6e083b8ee4a10dbb99ccb15c00604276bf1b984b2101f66fa201` |
| `tests/reference/test_executor_refusal_probe.py` | `23e468c6ca816adcfbc51ace6e860ad7078557639135f6612fa141872ed6bd35` |

## Causal evidence

Original source `0e2cf08a882035b69cb65efead5323b01d03152c` workflow `37743977472` passed both native jobs but failed installed AMD64 qualification. Raw installed log `/tmp/limeos-combined-ci-37743977472/downloads/113205330348.log` records four successful foundation checks, then the wrong-UID `runuser` probe at guest.py line 294 failing at inline Python line 4 with `BrokenPipeError: [Errno 32] Broken pipe`. No completed fresh acceptance JSON followed. Failure diagnostics show no restart/panic evidence; this report does not invent which executor loop failed.

The original inline line 4 is `sendall`, outside the existing receive-reset handling. The unchanged executor checks kernel peer UID before reading a frame, produces a bounded forbidden receipt and drops the stream. The client can encounter a closed peer while sending the frame. The existing negative helper already accepts EOF/reset as refusal, but its send path failed before that helper could examine either a buffered refusal or EOF. This explains the recorded error without changing production authentication.

## Correction and acceptance preservation

The correction catches only `BrokenPipeError`/`ConnectionResetError` around send and the initial header read, after successful connect. A send error continues to receive any buffered response. Therefore an explicit forbidden receipt remains inspected, and a buffered successful health response cannot be converted into denial. Unexpected write/header I/O and timeouts remain errors; connection errors remain outside normalization. Partial/malformed headers and body truncation still fail, and body reads have no new exception suppression.

The guest still requires an authorized health success before negative probes and now requires another authorized health success after the denied and forged probes for each executor. A daemon exit cannot satisfy the entire check. Wrong UID and forged actor outcomes still use the exact existing refusal predicate; no successful/private observation or effect reply passes that predicate.

## Regression proof inspected, not rerun

`/tmp/limeos-foundation-refusal-proof` retains six raw attempts and matching hashes. The pre-fix six-test invocation reports five errors, including deterministic EPIPE from a real closed Unix socketpair and buffered-response cases. The post-fix invocation and settled focused invocation each pass six tests. The full reference suite passes **15 tests**, comprising the prior nine plus six new probe methods; settled Ruff passes. The initial Ruff failure and corrected output remain present.

The test substitutes connect and injects reset/timeout/I/O cases privately. Its real socketpair proves the pre-send transport boundary; it does not prove kernel UID authentication or installed guest behavior. Tests also preserve buffered forbidden/health responses, authorized health, exact invalid-input semantics and partial/truncated failures. Actual installed proof requires the corrected-head workflow.

The reviewer independently matched the six raw output hashes and the two settled source hashes. At review time the attempt JSON contained tool outputs/exits/hashes; the integrator was asked to add known commands and source-stage labels before publication, marking any unretained pre-fix instrumentation hash honestly. This code GO does not claim that metadata enrichment is already complete.

## Attribution and follow-up

Keep workflow `37743977472` as a failed attempt with successful native 372-test/33-harness results and no completed fresh installed result. A successful corrected workflow belongs to its actual new head. Rust/library runtime equivalence to `0e2cf08a` must be documented separately from the modified validation harness. Re-pin new engineer briefs to the actual corrected tested source or state that distinction explicitly; never describe the failed original workflow as successful. No old wildcard VM artifact can fill its missing fresh result.

BKP-001, DSK-001, MNT-001/MNT-002, RT-001 and applicable installed composition/effect gates remain open. Current library source approval is unchanged; the historical footprint and failed inventory budget remain unchanged.

Only source/log/data reads and this `/tmp` review note were performed. The reviewer ran no tests, changed no repository/Git state, accessed no host/SSH/Pi/wybie/Docker and spawned no agent.
