import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { Freshness, Resource } from "../../contracts/generated/types";
import type { StackInventory } from "./stack-inventory-client";
import { StackInventoryController } from "./stack-inventory-controller";
import { StackJobs, useStackJobs } from "./StackActions";
import { ObservedStackCard, OtherProjectCard, StackCard } from "./stacks/StackCard";
import {
  containerIndex, dockerFresh, inventoryUnknown, jobForStack, observedStacks, omittedRows, readerDown,
  RECENT_JOB_SECONDS, serviceRows, sourceWord, stackSummary, unhealthyCount,
} from "./stacks/stack-model";
import { Link } from "./app/router";
import { formatClock } from "./ui/format";
import { Icon } from "./ui/icons";
import { MetricBar, StateMessage } from "./ui/primitives";

/** Whole-stack lifecycle controls are mounted only for an authenticated session. */
export interface StackOperations { csrf: string; revision: number; changed: () => void; expired: () => void; admin?: boolean }
const noop = () => {};
const containerPath = (name: string) => `/containers?search=${encodeURIComponent(name)}`;

/**
 * The Stacks page body. `value` is the strict stack inventory; `resources`
 * and `sources` are the shared overview's, used only to name containers and
 * to show the host observer's grouping when the inventory can't be read.
 */
export function StackManagement({ value, loading = false, stale = false, error = "", denied = false, refresh, operations, resources, sources, now = Date.now() / 1000 }: {
  value: StackInventory | null; loading?: boolean; stale?: boolean; error?: string; denied?: boolean; refresh: () => void; operations?: StackOperations;
  resources?: readonly Resource[]; sources?: readonly Freshness[]; now?: number;
}) {
  const index = useMemo(() => containerIndex(resources), [resources]);
  const live = dockerFresh(sources, now);
  const jobs = useStackJobs({
    csrf: operations?.csrf ?? "", revision: operations?.revision ?? 0, expired: operations?.expired ?? noop,
    admin: operations?.admin, enabled: !!operations, recentSince: now - RECENT_JOB_SECONDS,
  });
  const down = !!value && readerDown(value);
  const unknown = value ? inventoryUnknown(value, stale) : null;
  const observed = observedStacks(resources, live);
  const showObserved = observed.length > 0 && !denied && (down || (!value && !!error));
  return <section aria-labelledby="stack-management-heading" aria-busy={loading} className="stack-management">
    <header className="stack-toolbar">
      <div className="stack-toolbar-text">
        <h2 id="stack-management-heading">Stacks and services</h2>
        <p>{operations
          ? "Whole-stack actions start, stop or restart the existing service containers one at a time. They never create, remove or edit anything."
          : "Registered stacks stay visible when their containers are stopped or have not been created."}</p>
      </div>
      <button type="button" className="quiet compact" onClick={refresh} disabled={loading} aria-busy={loading}>
        <Icon name="refresh" size={14} className={loading ? "spin" : undefined} />Refresh stacks
      </button>
    </header>
    {loading && !value && !denied && <StateMessage kind="loading" title="Reading stack inventory…">Asking the host which stacks are registered and what is running.</StateMessage>}
    {denied && <StateMessage kind="unavailable" title="Stacks aren't available to your account">{error || "You no longer have access to stack inventory."}</StateMessage>}
    {!denied && error && !value && <StateMessage kind="error" title="Stacks can't be read right now" onRetry={refresh}>
      <p>{error}</p>
      <p>If this keeps happening, the host's stack reader may not be running. Containers keep running either way.</p>
    </StateMessage>}
    {!denied && error && value && <StateMessage kind="error" compact title="The last refresh failed" onRetry={refresh}>{error} Showing the previous reading.</StateMessage>}
    {!value && !loading && !error && !denied && <StateMessage kind="unavailable" title="Stack inventory is unavailable.">Choose Refresh stacks to read it again.</StateMessage>}
    {value && !denied && down && <ReaderDown />}
    {value && !denied && !down && <InventoryView value={value} unknown={unknown} index={index} live={live} now={now} jobs={jobs} operations={operations} />}
    {showObserved && <>
      <h3 className="stack-section-title">Stacks seen by the host observer</h3>
      <p className="stack-section-note">Docker's own grouping of containers into Compose projects, from the latest observation. Registration, Compose files and whole-stack actions need the stack reader.</p>
      <div className="stack-grid">{observed.map((s) => <ObservedStackCard key={s.id} stack={s} />)}</div>
    </>}
    {operations && <StackJobs data={jobs} csrf={operations.csrf} expired={operations.expired} admin={operations.admin} changed={operations.changed} names={index} />}
  </section>;
}

function ReaderDown() {
  return <StateMessage kind="unavailable" title="The stack reader isn't running">
    <p>LimeOS reads your stacks through its stack reader. It doesn't start by itself after the host restarts, so until it runs, stack status and whole-stack actions are unavailable. Your containers keep running.</p>
    <p>An administrator can start it on the host with <code>systemctl start limeos-storage-reader</code>, then choose Refresh stacks.</p>
  </StateMessage>;
}

function NoStacks({ fresh }: { fresh: boolean }) {
  if (!fresh) return <StateMessage kind="unavailable" title="Registered stacks could not be confirmed.">The stack catalog reading is out of date. Choose Refresh stacks to try again.</StateMessage>;
  return <StateMessage kind="empty" title="No stacks are registered.">
    <p>Stacks come from LimeOS's management catalog, <code>/etc/limeos/management-catalog.json</code> on the host. It lists the Compose projects under <code>/opt/stacks</code> that LimeOS may show and manage.</p>
    <p>An administrator creates it on the host with <code>limeosctl migrate propose-catalog</code>, reviews what it proposes and installs it (the cutover runbook has the exact steps). Containers that aren't in a registered stack still appear on the <Link to="/containers">Containers</Link> page.</p>
  </StateMessage>;
}

function InventoryView({ value, unknown, index, live, now, jobs, operations }: {
  value: StackInventory; unknown: string | null; index: Map<string, Resource>; live: boolean; now: number;
  jobs: ReturnType<typeof useStackJobs>; operations?: StackOperations;
}) {
  const previous = unknown !== null;
  const omitted = omittedRows(value, index);
  const projects = new Set(value.stacks.map((s) => s.project));
  const unattributed = omitted.filter((o) => o.project === null || !projects.has(o.project));
  const summary = stackSummary(value, unknown);
  const unhealthy = value.stacks.reduce((sum, s) => sum + unhealthyCount(serviceRows(s.services, index, { live, previous })), 0);
  const standalone = live ? [...index.values()].filter((r) => r.parent === null).map((r) => r.name).sort() : [];
  return <>
    {value.stacks.length > 0 && <SummaryBand value={value} summary={summary} unhealthy={unhealthy} current={!previous} />}
    {unknown && <p className="stack-banner" role="status"><Icon name="info" size={16} />{unknown} Current runtime counts are unavailable, so the stacks below show as Unknown.</p>}
    {value.stacks.length === 0 && <NoStacks fresh={value.catalog.state === "fresh"} />}
    {value.stacks.length > 0 && <div className="stack-grid">
      {value.stacks.map((stack) => {
        const job = operations ? jobForStack(jobs.jobs, stack, now) : null;
        return <StackCard key={stack.stack} stack={stack} unknown={unknown} index={index} live={live} names={index}
          omitted={omitted.filter((o) => o.project === stack.project)}
          job={job} receipt={job ? jobs.receipts[job.id] : undefined}
          operations={operations ? { csrf: operations.csrf, changed: operations.changed } : undefined} />;
      })}
    </div>}
    {value.other_projects.length > 0 && <>
      <h3 className="stack-section-title">Other observed projects</h3>
      <p className="stack-section-note">Engine labels identify these projects. They have not been adopted as managed stacks.</p>
      <div className="stack-grid">{value.other_projects.map((row) => <OtherProjectCard key={row.project.name} row={row} index={index} live={live} previous={previous} />)}</div>
    </>}
    <div className="stack-footnotes">
      <p>{previous || value.standalone_containers === null ? "Current standalone-container count is unavailable." : `${value.standalone_containers} standalone containers observed outside Compose projects.`}
        {!previous && standalone.length > 0 && value.standalone_containers === standalone.length && <> {standalone.map((name, i) => <span key={name}>{i > 0 && ", "}<Link to={containerPath(name)}>{name}</Link></span>)}.</>}
      </p>
      {!previous && value.truncated_containers ? <p className="notice">{value.truncated_containers} more Engine containers are not shown: the inventory keeps the first 64, running ones first, and counts only those.</p> : null}
      {!previous && value.unattributed_containers ? <p className="notice">{value.unattributed_containers} Engine {value.unattributed_containers === 1 ? "container's" : "containers'"} Compose project could not be identified.</p> : null}
      {!previous && unattributed.length > 0 && <p className="notice">Left out of every stack while they can't be read: {unattributed.map((o, i) => <span key={o.resource}>{i > 0 && ", "}{o.name ? <Link to={containerPath(o.name)}>{o.name}</Link> : <code>{o.shortId}</code>}{o.project ? ` (${o.project})` : ""}</span>)}.</p>}
    </div>
  </>;
}

function SummaryBand({ value, summary, unhealthy, current }: { value: StackInventory; summary: ReturnType<typeof stackSummary>; unhealthy: number; current: boolean }) {
  const services = summary.services;
  const notRunning = services ? services.total - services.running : null;
  return <div className="summary-band stack-summary" role="group" aria-label="Stacks summary">
    <div className="summary-cell">
      <p className="summary-label">Stacks</p>
      <p className="summary-value">{summary.stacks}</p>
      <p className="summary-note">{summary.unknown === summary.stacks ? "state unknown" : [`${summary.running} running`, summary.partial ? `${summary.partial} partly running` : "", `${summary.stopped} stopped`, summary.unknown ? `${summary.unknown} unknown` : ""].filter(Boolean).join(" · ")}</p>
    </div>
    <div className="summary-cell">
      <p className="summary-label">Service containers</p>
      <p className="summary-value">{services ? `${services.running} / ${services.total}` : "Unknown"}</p>
      <MetricBar value={services && services.total > 0 ? (services.running / services.total) * 100 : null} tone={services && services.running === services.total ? "success" : "warning"}
        label="Service containers running" valueText={services ? `${services.running} of ${services.total} running` : "Unknown"} />
    </div>
    <div className="summary-cell">
      <p className="summary-label">Needs a look</p>
      <p className={`summary-value ${unhealthy > 0 ? "tone-text-danger" : notRunning ? "tone-text-warning" : ""}`}>{services === null ? "Unknown" : unhealthy + (notRunning ?? 0) === 0 ? "Nothing" : unhealthy + (notRunning ?? 0)}</p>
      <p className="summary-note">{services === null ? "counts are unknown" : [unhealthy ? `${unhealthy} unhealthy` : "", notRunning ? `${notRunning} not running` : ""].filter(Boolean).join(" · ") || "every service container is up"}</p>
    </div>
    <div className="summary-cell">
      <p className="summary-label">Readings</p>
      <p className={`summary-value ${current ? "" : "tone-text-warning"}`}>{current ? "Current" : "Not current"}</p>
      <p className="summary-note">catalog {sourceWord(value.catalog)} · Docker {sourceWord(value.engine)} · read {formatClock(value.generated_at)}</p>
    </div>
  </div>;
}

/** Mount with the shared authenticated principal/grant identity. `manage` false hides lifecycle controls a viewer cannot use. */
export function StackPanel({ principal, grantRevision, csrf, expired, admin, manage = true, resources, sources, now }: {
  principal: string; grantRevision: number; csrf: string; expired: () => void; admin?: boolean; manage?: boolean;
  resources?: readonly Resource[]; sources?: readonly Freshness[]; now?: number;
}) {
  return <OwnedStackPanel key={JSON.stringify([principal, grantRevision, csrf])} csrf={csrf} expired={expired} admin={admin} manage={manage} resources={resources} sources={sources} now={now} />;
}
const useOwnerEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;
function OwnedStackPanel({ csrf, expired, admin, manage, resources, sources, now }: {
  csrf: string; expired: () => void; admin?: boolean; manage: boolean; resources?: readonly Resource[]; sources?: readonly Freshness[]; now?: number;
}) {
  const [revision, setRevision] = useState(0);
  const expiry = useRef(expired); expiry.current = expired;
  const stableExpired = useCallback(() => expiry.current(), []);
  const controller = useMemo(() => new StackInventoryController(() => expiry.current()), []);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useOwnerEffect(() => {
    void controller.start(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  const changed = () => { setRevision(value => value + 1); void controller.refresh(); };
  return <StackManagement value={state.value} loading={state.loading} stale={state.stale} error={state.error} denied={state.denied} refresh={() => { void controller.refresh(); }}
    operations={manage ? { csrf, revision, changed, expired: stableExpired, admin } : undefined} resources={resources} sources={sources} now={now} />;
}
