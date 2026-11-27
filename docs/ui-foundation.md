# UI foundation

The web UI's shell, look and Overview follow the previous app (pi-health), as
architecture decision A06 intended. This note is what you need to build the
next screens (Containers, Stacks, Storage) on top of it.

## Dependencies: none added

The UI keeps React and React DOM as its only runtime dependencies. Tailwind,
lucide-react and react-router were considered and left out:

- **Styling.** The existing feature components are styled by semantic class
  names (`.job-row`, `.container-logs`, `.schedules-table`), and tests select
  by those names. Adding Tailwind would have meant two styling systems side by
  side. Plain CSS on design tokens (below) gives the previous app's look and
  keeps one system.
- **Icons.** About twenty icons, drawn on lucide's 24 px grid as inline SVG in
  `src/ui/icons.tsx` (ISC notice kept there). lucide-react would add a
  dependency for that.
- **Routing.** Five pages and two sub-pages. `src/app/router.tsx` (history
  API, about 40 lines) and `src/app/routes.ts` (the table and path parsing,
  unit tested) cover them, plus aliases and unknown paths.
- **Result.** `npm audit` stays clean, nothing new to pin, CSP unchanged. The
  bundle grew from 265 KB JS / 8 KB CSS (79 / 2.4 KB gzip) to 307 KB / 32 KB
  (91 / 6.8 KB gzip). The previous app shipped 718 KB.

Add a dependency only for something we cannot reasonably write and test
ourselves. Pin it to an exact version and say why in the commit.

## Files

| Path | What |
| --- | --- |
| `src/ui/tokens.css` | Colour, type and spacing tokens; base element styles (buttons, inputs, focus, reduced motion) |
| `src/ui/primitives.tsx` + `.css` | Card, CardTitle, Badge, StatusBadge, MetricBar, PageHeader, RefreshControl, StateMessage, Stat, `.segmented` |
| `src/ui/icons.tsx` | `<Icon name="…" />` |
| `src/ui/copy.tsx` | `CopyValue` / `CopyButton`: an ID, digest or path as selectable code with a copy button |
| `src/ui/parts.css` | `.summary-band` (a strip of labelled figures), `button.compact` (quiet secondary action, still 44 px), `details.card-disclosure` (a quiet disclosure inside a card) |
| `src/ui/format.ts` | Percent, °C, bytes, durations, clock and age. Unknown is always the word "Unknown", never 0 |
| `src/ui/error-copy.ts` | `describeError(error, subject)` returns friendly copy and keeps the server reference |
| `src/app/routes.ts` | The route table, aliases, `parseRoute`, nav entries by role, document titles |
| `src/app/router.tsx` | `usePathname`, `navigate`, `<Link>` |
| `src/app/Shell.tsx` + `shell.css` | Sidebar, top bar, drawer, account and sign-out |
| `src/app/overview-data.tsx` | `OverviewProvider` / `useOverview()`: the one observation stream |
| `src/app/live-status.ts` | Whether the observations are live, polling, reconnecting or offline |
| `src/pages/*` | One file per page; `Pages.tsx` maps route to page |
| `src/overview/*` | Overview model (thresholds, health, counts), parts, history charts |
| `src/stacks/*` | Stacks page: `stack-model.ts` (status, member names, jobs on cards), `StackCard.tsx`, `stacks.css` |
| `src/storage/*` | Storage page: `storage-model.ts` (disk tree, summary, inventory facts), `inventory-client.ts`, `StorageView.tsx`, `storage.css`; `StorageMounts` |
| `src/style.css` | The existing feature components, on tokens |

## Tokens

Use the semantic names and never a literal colour (`grep` for `#` and `rgb(`
in what you touch: there should be none outside `tokens.css`).

- Surfaces: `--bg`, `--sidebar`, `--surface` (cards), `--surface-2` (hover,
  table heads), `--surface-3`, `--overlay`.
- Lines: `--border`, `--border-strong`, `--border-hover`, `--divider`.
- Text: `--text`, `--text-muted`, `--dim`. `--dim` is the quietest colour
  that still meets 4.5:1 on every surface. `--dim-decor` is for marks only,
  never text.
- Brand: `--accent` (lime), `--accent-hover`, `--accent-fg`, `--accent-soft`,
  `--accent-line`.
- Status: `--success`, `--warning`, `--danger`, `--info`, each with a
  `-soft` fill. `--warning-line`, `--danger-line` and the `-hover` variants
  are for bordered buttons.
- Type: `--font-sans` and `--font-mono` (IBM Plex when installed; no
  webfonts). The scale runs `--text-2xs` 10 px (axes), `--text-xs` 11 px
  (labels, badges), `--text-sm` 13, `--text-md` 14 (body), `--text-lg` 16
  (card titles), `--text-xl` 20 (metric values) to `--text-2xl` 22 (page
  titles). Titles, labels and numbers are mono; prose is sans.
- Shape: `--radius-sm`/`--radius`/`--radius-lg`, `--gap`, `--touch` (44 px:
  the minimum height for anything you can press), `--shadow`.

Tone classes: `tone-text-{tone}`, `tone-bg-{tone}`, and `.ui-badge.tone-{tone}`,
where tone is `neutral | accent | success | warning | danger | info`.

## Primitives

- `PageHeader title description status actions`: every page starts with one.
  Its `<h1>` takes focus after navigation (`data-page-title`).
- `Card` / `CardTitle`: a bordered surface. `.panel` is the same surface for
  feature components that bring their own headings.
- `Badge` / `StatusBadge`: the label is the meaning; the colour and dot are
  cues. Never use a badge with colour alone.
- `MetricBar value tone label valueText`: a 0–100 bar (`role=progressbar`).
  An unknown value draws a hatched empty track and announces "Unknown".
- `RefreshControl`: the live state ("Live", "Updating every 30 s",
  "Reconnecting", "Offline"), "Updated HH:MM:SS" and a refresh button. Use
  `<OverviewRefresh />` on pages backed by the overview.
- `StateMessage kind=loading|empty|error|unavailable`: one way to say
  "loading", "nothing here", "couldn't read" and "not set up". Pass `error`
  and `subject` for friendly copy, `onRetry` for a retry button. The server's
  message, code and reference stay under "Details for support". Do not show
  `message · code · reference` as a headline.
- `.segmented` with `aria-pressed` buttons: range and filter switches.

## Routing

`routes.ts` holds every route: `/`, `/containers`, `/stacks`, `/storage`,
`/alerts`, `/alerts/schedules`, `/alerts/policy`. Trailing and repeated
slashes and case are normalised. The previous app's paths redirect: `/system`,
`/disks`, `/pools`, `/mounts` and a few others. Anything else shows "Page not
found" with links. Caddy serves `index.html` for every non-file path
(`try_files {path} /index.html`), so every route survives reload and deep
links. A signed-out deep link shows the sign-in page and then that page.

### Adding a page

1. Add a `RouteDef` to `routes` in `src/app/routes.ts`. Give it `nav` if it
   belongs in the sidebar, `section` for the sidebar entry it highlights, and
   `manageOnly` if it only edits.
2. Write `src/pages/YourPage.tsx` taking `PageProps` (`session`, `expired`,
   `reauthenticate`). Start with `<PageHeader>`. Put feature components in a
   `<div className="panel">`.
3. Add it to the switch in `src/pages/Pages.tsx`.
4. Add the path to the routes test in `tests/ui-foundation.test.mjs`, and to
   `PAGES` in `tests/frontend/shell.py` if it is in the sidebar.

Link with `<Link to="/path">`, which keeps ctrl- and middle-click behaviour.
Navigate in code with `navigate("/path")`.

Linking to one container: `/containers?search=<container name>` (URL-encoded).
The Stacks page links each member this way. The Containers page is expected
to read the `search` query and filter to (or open) that container; until it
does, the link opens the Containers page unfiltered.

## Sharing the overview data

`OverviewProvider` (mounted once in `main.tsx`) owns the only
`/api/v1/observations/stream` EventSource and the 30 s `GET /overview`
fallback. A session may hold only two streams, so **never open another stream
or add a poll for overview data.** Call `useOverview()` instead:

```ts
const { value, error, live, updatedAt, refresh, refreshKey, now } = useOverview();
```

- `value` is the last good `Overview` (null until the first one arrives).
  `error` is the last failed read; it clears on the next success. When both
  are set, show the value and say it may be out of date
  (`<OverviewPending />` does this).
- `refresh()` re-reads now and bumps `refreshKey`. Use `refreshKey` as an
  effect dependency to re-read your page's own data on a manual refresh.
- `now` ticks every 5 s with no network, for ages and freshness checks.
- The stream ends every 25 s by design. `live-status.ts` keeps "Live" for
  20 s after the last snapshot, so the rotation never shows "Reconnecting".
  The fallback poll is skipped while the stream delivers. A stream the
  browser gave up on is retried at most once per 30 s.

Derive page facts from `value` with pure functions in a `*-model.ts` that
node tests import directly (see `src/overview/overview-model.ts`). Rules
already there: a stale or unavailable source, an old observation or missing
host metrics never yield "healthy"; a stopped container is a count, not an
issue; unknown values stay "Unknown".

Data the overview does not carry (stacks inventory, jobs, alerts, mounts) is
read by its feature controller only while its page is mounted. Keep it that
way: Alerts sub-pages mount on demand, as the old toggle buttons did.

## Roles

`canManage` (operator, administrator) shows container and stack lifecycle
controls, container jobs, schedules and policy. `isAdmin` shows
StorageMounts (storage_manage) and resolution. Viewers get no editing
controls and no reads for them. The server still enforces every grant; the
UI only hides what would be refused.

## Conventions

- **CSP** (`packaging/Caddyfile.cutover.example`): `default-src 'self';
  script-src 'self'; style-src 'self'`. No inline `<script>` or `<style>`, no
  CDN, no `data:` images. JSX inline SVG and React `style={{}}` are fine.
  `tests/frontend/shell.py` serves the build with this header and fails on
  console errors.
- **Mutations** stay plan → approve → queue in their controllers; pages only
  mount them.
- **Strict decoders**: every feature client decodes its responses. The
  overview and stream snapshot are still cast, not decoded (see follow-ups).
- **Accessibility**: one `<h1>` per page; native `<dialog>` with
  `containFocus` for modals (the drawer uses the same); `aria-pressed` on
  toggles; `aria-current="page"` on the current link; captions on tables;
  `role=alert` only for errors that need attention now (`StateMessage
  kind=error`), `role=status` otherwise; 44 px targets; colour never the only
  signal; `prefers-reduced-motion` respected globally.
- **Small screens**: no horizontal page scroll at 390 px. Wide tables scroll
  inside `.table-wrap` (give them `tabIndex=0`, `role=region` and a label),
  or stack as cards like `.resource-table`.

## Test selectors

- Select by role and accessible name first (`get_by_role("link",
  name="Containers")`), then by visible text, then by the stable classes
  that tests already use: `.account`, `.job-row`, `.container-logs`,
  `.schedules-table`, `section[aria-labelledby="stack-management-heading"]`.
- Labels other suites rely on: "Approve and restart", "Restart", "Refresh
  stacks", "Manage alert policy" (now a link on the Overview), "Save health
  settings", "Sign in", "Sign out"; headings "Stacks and services", "Recent
  container operations", "Welcome home." (sign-in) and each page's `<h1>`.
- If you change a label, role or class a test uses, update the test in the
  same commit and say so in the message.
- The node tests transpile some components and rewrite their relative
  imports by exact string (`tests/helpers/*.mjs`). Adding an import to
  `StorageMounts.tsx`, `HealthHistory.tsx`, `ContainerActions.tsx` or
  `ObservationView.tsx` needs the matching helper updated. Prefer importing
  primitives from new page components. `tests/helpers/modules.mjs` instead
  follows relative imports itself (with named stubs for the network and
  dialogs); the stack helper uses it, so `StackManagement.tsx`,
  `StackActions.tsx` and `src/stacks/*` can import freely. Pure `*-model.ts`
  files with only type imports are imported by node tests directly.
- Browser fixtures must serve `index.html` for non-file paths, as Caddy does.
  `tests/frontend/household_fixture.py` is a ready household host (CPU,
  memory, temperature, disks, containers, stacks, alerts) for new browser
  tests and screenshots. Its stack inventory names the same containers as
  its overview, it answers whole-stack plan/approve/queue and job progress,
  and `/storage/inventory`; `stacks_mode`, `storage_inventory`,
  `job_state` and `job_receipt` switch the states the tests need.

## Running

```sh
cd frontend && npm ci --ignore-scripts && npm test && npm run build && npm audit --audit-level=high
python3 tests/frontend/restart.py
python3 tests/frontend/stacks.py --output /tmp/stacks
python3 tests/frontend/shell.py --output /tmp/shell --screens /tmp/shell-screens
python3 tests/frontend/stack_cards.py --output /tmp/stack-cards --screens /tmp/stack-screens
python3 tests/frontend/storage_page.py --output /tmp/storage --screens /tmp/storage-screens
```

`health_history.py`, `health_policy.py` and `schedules.py` also need a
`.cache/b1-*-qualification/frontend-build.json` manifest for the build under
test.

## Known follow-ups (frontend)

- The privileged-VM browser guests (`tests/privileged_vm/p04_*_browser_guest.py`)
  still wait for the `<h1>` "At home." and click a "Stacks" *button*. They
  need `h1` "Overview" (or the page's title) and a click on the "Stacks"
  *link* (`/stacks`). They could not be run here.
- `/overview` and the stream snapshot have no strict decoder yet. Add one in
  `api.ts` that every overview read passes, as the feature clients do.
- `MediaActivityPanel`, `NetworkGroups`, `ToolsStatus` and `ResolutionPanel`'s
  wider views exist but are not mounted on any page.
- The internals of the moved Containers components are unchanged (the
  pager, big buttons). Stacks and Storage are redesigned (see below).
- IBM Plex is used only when installed. Self-hosted WOFF2 files would be
  allowed by the CSP (`font-src` falls back to `'self'`), at roughly 100–200 KB.

## Needs backend work

1. **Alerts on the Overview.** The overview carries no alert state, so the
   home page cannot list active alerts without a second read of
   `/automation/alerts`. An open-incident summary in `Overview` (count, worst
   severity, newest few) would allow it at no extra load.
2. **Host capacity.** `HostMetrics` has percentages only. The previous app
   showed memory and disk used/total, swap, and a load figure people could
   judge against the core count. Needed: memory used/total bytes, root disk
   used/total bytes, swap used/total, CPU count, and hostname, model and OS
   for the header.
3. **Application launcher.** The previous home page's main feature was the
   grid of apps with "open" links (Jellyfin, Sonarr…). Containers carry no
   published ports or web URLs, so it cannot be rebuilt. Published ports, or
   a configured URL per app, are needed.
4. **Storage helper state.** The mount catalog answers a generic 503
   `unavailable` both when the storage helper is not installed or enabled and
   when it is briefly down. The UI explains rather than alarms, but cannot
   tell the two apart. A distinct code or a capability flag is needed.
5. **Capabilities in the session.** The UI infers what to show from the
   role, but grants can be narrower (scheduling needs a separate enablement).
   A list of permitted operations in `SessionView` would let pages hide
   exactly what will be refused.
6. **Storage fullness.** `Resource.size_bytes` is capacity only. Used or free
   bytes per partition and pool are needed for storage bars; today only the
   root filesystem has one (from `HostMetrics.disk_percent`). Pools carry no
   size at all, and no branch list. Disk model, bus, temperature and SMART
   health (the previous app's disk cards) are not observed.
7. **Container detail.** Start time or uptime and restart count are not in
   `Resource`. The Containers redesign will want them, and the previous app
   showed them.
8. **Stream rotation.** An explicit end event (or `retry:` and a final
   comment) before the planned 25 s close would let clients tell rotation
   from failure without a timing heuristic.
9. **Stack status.** `degraded` covers both "some members stopped" and "a
   member unhealthy". Per-stack running/total counts in the overview would
   let the Overview say which without reading the stacks inventory. (The
   Stacks page now derives it from the inventory, and from the overview's
   containers when the inventory can't be read.)
10. **Stack reader state.** When the stack reader is not running (after a
    reboot it does not start by itself) the inventory answers both sources
    `unavailable`, the same as a reader that is running but not yet
    permitted to read. The Stacks page says "isn't running" and how to start
    it; a reader status field would let it be exact.
11. **Stack logs and edits.** The previous stack cards had logs, pull,
    edit Compose, backups and delete. None has an API here (by design so
    far); only whole-stack start, stop and restart are offered.
