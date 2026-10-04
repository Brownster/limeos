# P00 execution tracker

Started: 2026-10-04. One implementer; phases run in sequence. The frozen source reference is `/home/marc/Documents/github/pi-health` at `80593b2`; wybie remains deployed at `618ce92`. No development or fixes happen in the Python project.

## Reference-host availability

The operator confirmed on 2026-10-04 that wybie is currently streaming a television show. Active testing on wybie is paused during playback. The completed footprint probe and measurement harness have exited; a read-only process check found neither running. The uploaded bundle remains under `/tmp` as an inactive artifact.

Continue development, compilation, load tests, provider spikes, and failure injection locally or on the separate test host. Limit production-host access during playback to light passive reads. Agree a quiet window with the operator before future active Pi measurements, shadow installation, or cutover. The existing footprint result is sufficient for its gate and needs no repeat now.

## Estimates and review point

P00 remaining effort estimate: 5 working days (40 implementer hours), excluding time waiting for test hardware or provider-account access. Allow one day for inventory and baseline gaps, one for threat model and ADRs, two for Linux/package/recovery mechanics, and one for footprint/provider spikes. This is a planning estimate, not evidence that any gate has passed.

Review the cutover scope before continuing if P00 exceeds 60 hours (150%). Record actual effort and any revised scope when P00 ends. P01's estimate is recorded before P01 starts.

The RW-005 footprint subtask had a 2-hour estimate and a 3-hour review point. It completed on 2026-10-04 in approximately one working hour, including compiler-container setup and the ten-minute Pi measurement. Its [passing result](2026-10-04-rw005-footprint-result.md) does not close the rest of RW-005 or P00.

## Work

| Package | Current evidence and remaining work |
|---|---|
| RW-001 — Inventory | Source reference and deployed revision identified. Checked feature/state/client inventory still required. |
| RW-002 — Baseline | [Pi 5 baseline](2026-10-04-reference-pi5-baseline.md) exists. Its unmeasured items remain open. |
| RW-003 — Threat model and ADRs | Proposed architecture and 50-row register exist. Threat model, accepted ADRs, and executable regression evidence remain open. |
| RW-004 — Linux mechanics | VM, mount, executor, recovery, and package spike evidence remains open. |
| RW-005 — Toolchain, footprint, providers | [Footprint gate passed on wybie](2026-10-04-rw005-footprint-result.md): 4.55 MiB idle PSS, no swap, all ten starts ready within 23.5 ms. Both target builds work. Provider/isolation/continuation spikes remain open. |

No phase or production defect is closed by the experiment's behavior tests. Closure requires the corresponding production regression and recorded passing evidence.
