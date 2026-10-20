# Registered stack inventory: local evidence

Source `e5e45caa31db2062ef118e28ce2ad419a944000c` registers the read-only stack
cohort through the dormant root reader, actual Core/API and native Stacks view.
The [handoff](../../../p04-stack-inventory-runtime-handoff.md) describes behavior,
resource admission and the outstanding installed/ARM64 gates.

`inputs.sha256` binds 667 source/configuration/contract/test inputs. `manifest.json`
binds the production UI assets and all 28 terminal guarded attempts. Each
compressed check record preserves its actual source hashes, command,
environment, exit status, sampled headroom and before/after input comparison.
Passed repeats are not added to coverage totals.

The focused native Rust checks passed 1,012 cases across six library suites and
four actual Core stack/HTTP tests. The Core run filters 27 unrelated tests; it is
not a complete Core or workspace test run. The frontend passed 222 cases, strict
TypeScript and its production build. Reference tests passed 61 cases. Strict
all-target workspace Clippy, actual Rust contract generation/drift, formatting,
repository hygiene, Python Ruff and Bash syntax passed. The supplemental B
harness passed 68 cases, overlapping root API coverage. Domain and deployment
adapter tests now run in their registered native crates and are re-exported by
the leaf harness; its zero-case domain wrapper supplies no extra coverage.

The mounted Chrome fixture passed six checks: browser timer binding, Stacks
availability despite a failed overview, replica/stopped/one-off rows, source
failure withholding counts, 403 clearing private facts and fixed GETs only.
`browser-5` preserves the final result and inspected screenshot. These are closed
browser fixtures, not an installed Engine or physical host acceptance run.

The first combined Rust attempt stopped at the sampled three-GiB host memory
floor. Browser attempt 1 lacked the pinned Playwright cache; attempts 2 and 3
reproduced `Illegal invocation` after reading valid data. Attempt 3's DOM/network
capture showed that the default clock invoked the native timer with its clock
object as receiver. Wrapping the timer calls fixed the browser failure; attempts
4 and 5 passed and separately reproduce the raw native timer rejection. Minor
test-only Clippy borrowing and new-script executable-mode failures are retained
alongside their passing reruns. No assertion, owner check, role limit or source
freshness condition was relaxed to pass.

The source registers the next B2 path; **installed acceptance remains open**.
There is no new native ARM64, package execution, complete service resource,
installed crash/recovery, Compose source-inspection, whole-stack effect or Pi
cutover acceptance. Component resource models and existing generic supervisor
tests do not replace measurement and fault testing of the new installed worker.
Production/Pi services and the frozen Python checkout were untouched.
