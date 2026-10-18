# A1 browser owner cleanup correction

Source `d7b9efbfe609aab26c138b2f19b6f8a1938d3ef1` changes only the mount
component's browser owner effect. It uses layout cleanup at browser commit;
static rendering keeps the passive hook. This closes the passive-cleanup window
between DOM removal and controller disposal. The previous controller's original
race regressions remain in the [initial packet](../2026-10-17-a1-ui-lifetime/README.md).

All **129 frontend cases** pass again, including the same 40 mount cases and
static rendering. The production UI build/type-check passes. These are repeated
checks at a new source, not additional cases or a browser lifecycle test.

Review of pinned React DOM 18.3.1 shows layout destruction in deletion/mutation
commit, while passive destruction is processed through `flushPassiveEffectsImpl`.
The dependency hashes bind the inspected source; it is source evidence rather
than a new browser reproduction. Actual mounted navigation, key replacement,
focus and Strict Mode qualification remains with the existing test owner.

`inputs.sha256` rebinds the initial packet's 213 inputs. Only
`frontend/src/storage/StorageMounts.tsx` differs. Compressed native per-file logs,
summary and production-build output retain the exact run. `production-ui/`
contains that production bundle; no size/timing performance credit is claimed.
`artifacts.sha256` covers this packet except itself. The shared runtime, root
shell/contracts, executor/Store, guest and package sources remain unchanged.
No installed/Pi, actual mount, recovery, A1/Stream-A/P04 or defect gate closes.
