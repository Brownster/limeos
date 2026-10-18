# Cached tools status UI local evidence

Tested product source: `1dbcdda81a2d6d119d5b59ba762a389e17c0fc1b` on
`engineer/p04-tools-read-ui`, based on `f8aec4a`. See the
[handoff](../../../p04-tools-read-ui-handoff.md) for the exact mounting and server
requirements. Shared main remains unchanged at `7df3f75`.

**161 frontend cases pass**, including 38 new tools cases, across nine files.
Pinned TypeScript checking, the standalone Vite/React preview build, evidence
script lint and diff checks pass. Each test file runs sequentially through the
native Node case reporter; these are actual test cases rather than one count per
file. No Rust tests were rerun or credited: the prior tools Rust/API source and
wire fixture remain unchanged.

Coverage includes selected-field privacy, exact Rust wire compatibility,
configuration/default/port/path admission, independent freshness and age 90/91,
unavailable/error/transition states, owner eligibility/keys, unique accessible
heading relationships, fixed transport and refused redirects, streamed UTF-8 and
8-KiB bounds, aborted/stalled bodies, 12-second deadlines, coalescing, clock
rollback, early timer rearming, access loss and abandoned late results. Static
markup and key checks do not exercise the mounted React lifecycle or a browser.

`inputs.sha256` binds the source/contract/fixture superset and unchanged Rust wire
references. `metadata.json` records source identity, pinned dependency versions,
exact counts/bounds, checked untouched paths and browser availability. Compressed
logs/scripts retain the initial test-loader failure and final results. The first
loader omitted `fileName`, allowing a `.ts` generic arrow to be parsed as JSX;
passing each source filename fixes that test boundary. `artifacts.sha256` covers
the packet, excluding itself.

`preview/` contains synthetic current, partially expired and unavailable examples
with pinned Vite configuration, existing CSS and built assets. Generated preview
paths are local to this worktree; adjust them for another checkout. No browser
surface was available (`cua.getState` returned empty app/browser inventories), so
no visual/responsive review or screenshot was produced. No preview server was
started. Bundle size/build duration are not performance qualification.

Reproduce using the pinned repository frontend dependencies and disk caches:

```text
npm run check                       # frontend directory
uv run --offline --no-project .cache/tools-ui/check-frontend.py
node /home/marc/Documents/github/lime-os/frontend/node_modules/vite/bin/vite.js build --config .cache/tools-ui/vite.config.mjs
```

The latter two commands use scripts/configuration archived here; restore those
scratch paths or adjust them for the checkout. Point `TMPDIR`, `UV_CACHE_DIR` and
`npm_config_cache` to the repository caches as recorded in metadata. Run suites
sequentially. The temporary `frontend/node_modules` link was removed afterward.

Production cache/authority/host lifetime, contracts/routes/shell mounting, actual
browser lifecycle/visual and installed acceptance remain pending. This packet
adds no service mutation, recovery, ARM64/Pi, performance/soak, durable scheduling,
defect closure, P04 completion or cutover acceptance. No production host was
contacted.
