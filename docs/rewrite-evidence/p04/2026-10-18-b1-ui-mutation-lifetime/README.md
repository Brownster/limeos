# B1 UI mutation lifetime — local evidence

Tested source `e9cb979442a03c6ba978654bfa3193f78a348804`, branch
`engineer/p04-automation-ui-lifetime`, base main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`.
See the [handoff](../../../p04-b1-ui-mutation-lifetime-handoff.md) for behavior,
integration and remaining acceptance. This packet qualifies frontend source
and local rendering/bundling only.

| Check | Result |
| --- | --- |
| Existing API assertions, pinned TypeScript transpilation | 4 passed |
| Existing automation controller cases | 14 passed |
| Existing automation client/view cases | 10 passed |
| New mutation lifetime cases | 15 passed |
| Existing media activity client cases | 29 passed |
| Existing media activity view cases | 10 passed |
| Existing observation cases | 4 passed |
| Existing stack discovery cases | 18 passed |
| Total | 104 passed, zero failed |
| `npm run check` | Passed |
| `npm run build`, existing main entry | Passed, 21 modules |
| Isolated production build importing actual `AutomationPanel` | Passed, 19 modules |
| `git diff --check` | Passed |
| Rust, installed API/UI, browser, ARM64, crash/restart, resource/performance | Unrun |

The baseline callback reproduction is separate from the 104 cases. It extracts
the real old `mutate` callback from `baseline-AutomationView.tsx`, which is copied
from the specified base, and executes two callbacks with the same render's
`blocked=false`. The log records two action dispatches and two abandoned
completion callbacks. New cases cover synchronous admission and reentrancy,
dispose before dispatch, ignored abort, generation restart, expiry and peer
cancellation, authority refusal, persistent conflict/unconfirmed feedback,
pending-section read fences, actual client abort forwarding, owner keys and
rendered pending states. These are deterministic source/controller checks,
not browser commit or double-click qualification.

The first subprocess helper produced no API TAP cases and failed its count
assertion. The preserved explicit `--test` attempt counted the API file as one
case. Neither attempt is credited. The final direct API execution uses the
unchanged test assertions with a pinned-compiler data URL for the actual
`frontend/src/api.ts`, and reports all four cases. Other test files were run
individually through the command tool, sequentially; do not reuse the failed
subprocess helper as a passing runner. Its script and logs are retained for
audit. There was no product failure in these count-check attempts.

Pinned Node, React/React DOM, TypeScript and Vite identities are in the manifest.
The React DOM development source hash records the inspected lifecycle input;
it does not constitute browser execution. The separate panel bundle is an
unregistered scratch entry, containing the real panel, controller and client.
It was built without serving it or contacting an API. The main production
bundle remains the existing registered UI. No route or shell registration was
applied just to obtain a passing build.

`inputs.sha256` binds all tracked frontend inputs and generated contract types.
The changed-path manifest contains seven frontend paths plus the design only;
all central roots, Store/schema, protocol, generated contracts, workspace lock,
package and service files are unchanged from the base. The active central
schedule registration is outside this source and this packet's acceptance.
`artifacts.sha256` binds packet files, including compressed raw logs, baseline
source, scripts, manifest and both bundles. Build size values describe these
local artifacts, with no installed footprint or performance claim.

All scratch/build/cache paths used repository disk, with dependencies reused
from the existing pinned frontend installation. No dependency install, Pi,
host mutation or second qualification suite was run. B1, P04, defect closure,
release and cutover acceptance remain open.

To check this packet from its directory, run `sha256sum -c artifacts.sha256`.
From the worktree root, verify `inputs.sha256` using its full packet path.
