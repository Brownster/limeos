# Local gated mount refusal recovery qualification

Qualified source: `5c10736c7784b6d460cac6600f88931087494d0b` on local main.
Design `1775aaa`, clarified at `417e22f`, precedes implementation. The
[handoff](../../../p04-a1-gated-refusal-recovery-handoff.md) describes authenticated
never-started completion and fresh no-effect reconciliation. The
[older installed Core FD refusal](../2026-10-30-a1-core-fd-refusal/README.md)
retains its original pending state; no historical missing proof is reconstructed.

## Final acceptance

| Check | Result |
| --- | --- |
| Full workspace with `limeos-executor/read-worker-probe` | 1,736 runtime passes; 55 compile-fail doctests |
| Final native guardian protocol focus | Six passes; private child entry actually invoked |
| Focused Core mount path | Four passes, including real Store/owned RPC recovery |
| Focused Store mount path | Thirteen passes, including actual reopen and retained claims |
| Focused Root journal | Fifteen passes; pre-existing descriptor child actually invoked |
| Strict workspace/all-target Clippy with the same feature | Pass, warnings denied |
| Actual generated-contract drift | Pass; two intended generated files checked in |
| Frontend tests and checked production build | 244 passes; TypeScript/Vite pass; UI assets unchanged |
| Rust formatting, repository boundaries and diff hygiene | Pass |

Nine new runtime regressions cover five native process parents and one case each
in domain, Root journal, Store and Core. One new compile-fail doctest refuses
deserialization of the opaque supervision token. Four outer ignored child entries
are invoked by passing parents; child summaries and focused checks are not added
again to the workspace total.

Six final guards (`rust-3`, `focused-5`, `clippy-3`, `contracts-1`, `frontend-1`,
`sanity-1`) share identical before/after maps covering 740 committed inputs.
Current worktree bytes match the qualified commit. All eighteen actual guard
attempts remain preserved in [source-match metadata](EVIDENCE-BUNDLE.json "bundled: source-match.json"), with
commands, Cargo groups and differences in earlier maps.

Three failed checks remain visible. `focused-2` found an undeclared tempfile test
dependency; the fixture now owns an exclusive mode-0700 repository-cache directory
without adding dependencies. `core-1` found unsupported SQLite `usize` decoding;
the Core/journal fixture counts use `i64`. `clippy-1` found a 4,920-byte running
variant; a single boxed running owner keeps the release result small while
retaining ownership. No production predicate or lint was suppressed.

`rust-1` and `rust-2` passed earlier source, before the final protected authorization
boundary; neither supplies final-source acceptance. `focused-3/4`, `core-2`,
`store-1`, `journal-1` and `schema-1` retain their earlier maps. The final full run
covers their actual final test bytes. `clippy-2` predates the final authorization
change and `clippy-3` qualifies it.

An earlier `focused-1` admission refused at 3,740,418,048 available bytes, below
the four-GiB start threshold. It launched no guard or compiler. Memory recovered
without stopping the unrelated Android emulator or any other application.
Every actual run uses repository-disk scratch, one Cargo job/test thread and the
unchanged sampled three-GiB stop floor. No guard fired. Each phase started with
no other compiler observed; this work ran one phase at a time. The lowest final
sample is 5,107,527,680 available bytes. Earlier `rust-1` reached 3,455,168,512,
still above the stop floor. All attempts' guarded inputs stayed unchanged.
The final native observation finds all 81 recorded guard/command/child PIDs absent.

## Observed behavior

The native private child fixtures use actual pipes, pidfds and the production
GO parser, ACK encoder, final-publication function and owned-child wait path.
Their effect is an owned marker; helper identity fields are synthetic unit facts,
without installed root-image or mount-admission trust.

Eighteen final native child instances cover closed, partial, expired, wrong-bound
and oversized GO; absent, malformed, oversized, wrong-action/digest, extra-byte
and unsuccessful ACK; a complete ACK whose child remains alive; and a full GO
that can complete its marker but cannot be accepted as gated abort. Protocol
failure itself is not termination: failed fixture children are explicitly killed
and waited by their owner. A token is accepted only after a complete bound ACK,
successful actual child exit and reaping.

The final-publication regression authorizes earlier, awaits the owned child,
then revokes authorization or expires freshness. The current check observes the
refusal before writing GO; the child acknowledges never-started and leaves no
marker. The still-authorized case completes its marker. Production composition
supplies a current protected write-ceiling check after awaited admission/identity
revalidation, then rechecks cancellation and both original deadlines without
another await before the first write. From that write attempt, failure remains
potentially delivered and cannot take the parent's gated-abort shortcut.

Domain checks round-trip the new outcome, refuse it as mount success and keep
pending helpers outside no-effect reconciliation. Journal tests retain exact
helper binding, durable reaped facts and all claims through reopen and expired
original replay. Replay mints no launch permit. Only trusted orchestration's
separate fresh unmounted proof can make the existing terminal journal transition.
Journal facts in these tests are explicitly synthetic; native supervision and
protected installed composition retain separate trust requirements.

Store tests genuinely reopen both after saving never-started uncertainty and
after saving a terminal Root receipt without Core's independent verification.
Both retain Core claims. Missing, stale or changed current evidence refuses;
the matching fresh unmounted proof then records the same original action as
precondition-changed and atomically releases its claims. Pending/restarted and
failed-audit regressions remain passing.

Core uses a real isolated Store and owned Unix listener with private task-scoped
test credentials, absent in production. Bound quiescent recovery requests the
existing read-only refinement and a separate current inspection; it never sends
execute or replaces the action. Missing current evidence, refused reconciliation,
pending quiescence and wrong digest retain the barrier. The existing default
root-peer refusal test still passes before request frames are sent. These tests
do not qualify genuine installed policy/guardian/physical mount behavior.

## Reproduction and remaining scope

`checks/` preserves all actual gzip-compressed guard records/logs.
`launch-and-local/` includes launch/admission identities, the refused admission,
frontend asset hashes, final source comparison and native process absence.
`source.patch.gz` contains the qualified commit. `reproduction-sources.tar.gz`
contains changed source, updated design, helpers and the unchanged guard.
`SHA256SUMS` covers every other packet file. Reproduction assumes this repository
and its locked cached dependencies/toolchain, using repository-disk scratch.

The change is registered in the actual Root/Core mount path. The additive
`not_started` outcome changes only executor-receipt schema and generated types.
SQL schema 14, Root journal schema 1, dependencies, resource/service ceilings,
all complete-consumer comparisons and protected paths are unchanged. Older
closed readers reject the new outcome. Package return must restore matching
binaries/contracts and Core/Root journal before-images even when SQL versions
match, alongside existing policy/pre-schema-14 database/package requirements.

Fresh installed packages and a disposable VM must exercise genuine root-image
trust, actual policy revocation, no-effect refinement, whole-service cleanup and
resources, accepted mount/API/UI/replay/restart and failure paths. Normal
complete-consumer reliability under host task/FD churn remains unresolved.
This packet repairs no old pending action and establishes no successful installed
mount. A1/B1, wider A/B/P04, current ARM64/Pi/return/footprint/soak and P05/P06
cutover remain open; the historical inventory-performance failure is unchanged.
No Pi, VM, real host mount, package/service activation, external send or push ran.
