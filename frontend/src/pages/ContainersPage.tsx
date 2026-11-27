/**
 * Containers: every container the host reports, from the shared overview
 * (no extra stream or poll), with filters, search, grouping and sorting kept
 * in the URL, lifecycle actions through plan → approve → queue, logs, a
 * detail drawer and the principal's recent operations.
 *
 * Paging: the overview carries at most 64 containers (the Engine reader's
 * bound; beyond it the docker source says how many were left out), so the
 * whole list is drawn and the old 20-row pager is gone. MAX_ROWS caps the
 * drawing at 200 in case that bound ever grows.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ContainerAction } from "../../../contracts/generated/types";
import { useOverview } from "../app/overview-data";
import { PageHeader, StateMessage } from "../ui/primitives";
import { formatAge, formatClock } from "../ui/format";
import {
  buildRows, counts as countRows, dockerSource, FILTERS, groupRows, MAX_ROWS, parseView, SEARCH_MAX, serializeView, SORTS,
  visibleRows, type DockerSource, type Group, type Row, type Sort, type ViewState,
} from "../containers/container-model";
import { ContainerCards, ContainerTable, TABLE_QUERY, useMediaQuery, type RowHandlers } from "../containers/ContainerList";
import { ContainerDetail } from "../containers/ContainerDetail";
import { LifecycleDialog, type LifecycleTarget } from "../containers/LifecycleDialog";
import { LogsDialog, type LogsTarget } from "../containers/LogsDialog";
import { ContainerJobs, jobAnchor } from "../containers/OperationsList";
import { useContainerJobs } from "../containers/use-container-jobs";
import { CIcon } from "../containers/icons";
import "../containers/containers.css";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import { canManage, isAdmin, type PageProps } from "./Pages";

/** The view in the URL query: replaced (not pushed) so Back leaves the page. */
function useView(): [ViewState, (patch: Partial<ViewState>) => void] {
  const [view, setView] = useState<ViewState>(() => parseView(window.location.search));
  const update = useCallback((patch: Partial<ViewState>) => setView((v) => ({ ...v, ...patch })), []);
  useEffect(() => {
    if (window.location.pathname !== "/containers") return;
    const search = serializeView(view);
    if (search !== window.location.search) window.history.replaceState(window.history.state, "", `/containers${search}${window.location.hash}`);
  }, [view]);
  useEffect(() => {
    const pop = () => setView(parseView(window.location.search));
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  return [view, update];
}

function DockerNotice({ docker, now }: { docker: DockerSource; now: number }) {
  const when = docker.sampledAt === null ? null : `${formatClock(docker.sampledAt)} (${formatAge(docker.sampledAt, now)})`;
  return (
    <>
      {docker.state === "stale" && (
        <StateMessage kind="unavailable" compact title="Docker readings are out of date">
          The last reading is from {when ?? "an unknown time"}. Statuses below are from then and may have changed; CPU and memory show as Unknown until Docker reports again.
        </StateMessage>
      )}
      {(docker.state === "unavailable" || docker.state === "missing") && (
        <StateMessage kind="unavailable" compact title="Docker isn't reporting right now">
          {when ? `The last reading is from ${when}. ` : ""}Which containers exist, their status and their CPU and memory are unknown until it reports again. Lifecycle actions still inspect the container first.
        </StateMessage>
      )}
      {docker.state === "loading" && (
        <StateMessage kind="loading" compact title="Waiting for Docker's first reading">The list fills in when it arrives.</StateMessage>
      )}
      {docker.warnings.map((warning, i) => <p key={i} className="notice" role="status">{warning}</p>)}
    </>
  );
}

function Skeleton() {
  return (
    <ul className="skeleton-list" aria-hidden="true">
      {Array.from({ length: 6 }, (_, i) => <li key={i}><span /><span /><span /></li>)}
    </ul>
  );
}

let nonce = 0;

export function ContainersPage({ session, expired }: PageProps) {
  const { value, now, refresh, refreshKey } = useOverview();
  const manage = canManage(session);
  const admin = isAdmin(session);
  const [view, update] = useView();
  const [revision, setRevision] = useState(0);
  const changed = useCallback(() => setRevision((v) => v + 1), []);
  const jobs = useContainerJobs({ enabled: manage, admin, revision: revision + refreshKey, expired, settled: refresh });
  const wide = useMediaQuery(TABLE_QUERY);
  const [lifecycle, setLifecycle] = useState<LifecycleTarget | null>(null);
  const [logs, setLogs] = useState<LogsTarget | null>(null);
  const detailOpener = useRef<HTMLElement | null>(null);
  /** Set when the drawer closes to show an operation: focus goes there instead. */
  const skipRestore = useRef(false);
  const [pendingFocus, setPendingFocus] = useState<string | null>(null);

  const docker = useMemo(() => dockerSource(value?.sources ?? [], now), [value, now]);
  const current = docker.state === "fresh";
  const rows = useMemo(() => (value ? buildRows(value.resources, jobs.jobs, current) : []), [value, jobs.jobs, current]);
  const counts = useMemo(() => countRows(rows), [rows]);
  const shown = useMemo(() => visibleRows(rows, view), [rows, view]);
  const drawn = shown.slice(0, MAX_ROWS);
  const groups: Group[] = useMemo(
    () => (view.group ? groupRows(drawn, rows) : [{ stack: null, label: "", rows: drawn, running: counts.running, total: counts.all }]),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [view.group, shown, rows, counts],
  );
  const names = useMemo(() => new Map(rows.map((r) => [r.resource.id, r.name])), [rows]);
  const detailRow = view.detail === null ? null : rows.find((r) => r.name === view.detail) ?? null;
  const filtered = view.filter !== "all" || view.query.trim() !== "";

  const fallbackFor = (row: Row) => () =>
    document.querySelector<HTMLElement>(`a.container-name[href="/containers?container=${CSS.escape(encodeURIComponent(row.name))}"]`) ??
    document.querySelector<HTMLElement>("[data-page-title]");

  const handlers: RowHandlers = {
    canManage: manage,
    now,
    current,
    grouped: view.group,
    onAction: (row: Row, action: ContainerAction, opener: HTMLElement) =>
      setLifecycle({ resource: row.resource, action, opener, fallback: fallbackFor(row), nonce: ++nonce }),
    onLogs: (row: Row, opener: HTMLElement) => setLogs({ resource: row.resource, opener, fallback: fallbackFor(row), nonce: ++nonce }),
    onDetail: (row: Row, opener: HTMLElement) => {
      detailOpener.current = opener;
      update({ detail: row.name });
    },
    onShowOperation: (id: string) => {
      if (view.detail !== null) {
        skipRestore.current = true;
        detailOpener.current = null;
        update({ detail: null });
      }
      setPendingFocus(id);
    },
  };

  // Show an operation: after the drawer (if any) has closed, scroll to and focus its row.
  useEffect(() => {
    if (pendingFocus === null) return;
    const frame = window.requestAnimationFrame(() => {
      const target = document.getElementById(jobAnchor(pendingFocus)) ?? document.getElementById("operations-heading");
      target?.scrollIntoView({ block: "center" });
      target?.focus({ preventScroll: true });
      setPendingFocus(null);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [pendingFocus]);

  const closeDetail = useCallback(() => {
    const opener = detailOpener.current;
    detailOpener.current = null;
    update({ detail: null });
    if (skipRestore.current) {
      skipRestore.current = false;
      return;
    }
    window.requestAnimationFrame(() => {
      const target = opener?.isConnected ? opener : document.querySelector<HTMLElement>("[data-page-title]");
      target?.focus();
    });
  }, [update]);

  const attentionWord = counts.attention === 1 ? "needs attention" : "need attention";
  const description = value
    ? `${counts.running} running · ${counts.stopped} stopped · ${counts.attention} ${attentionWord} · ${counts.all} in total`
    : "Every container the host reports.";
  const caption = `Containers: ${drawn.length} shown of ${counts.all}${view.group ? ", grouped by stack" : ""}`;

  return (
    <>
      <PageHeader title="Containers" description={description} actions={<OverviewRefresh />} />
      <OverviewPending />
      {!value && <Skeleton />}
      {value && (
        <section className="containers-surface" aria-label="Container list">
          <DockerNotice docker={docker} now={now} />
          <div className="containers-toolbar">
            <div className="segmented filter-switch" role="group" aria-label="Show">
              {FILTERS.map((filter) => (
                <button key={filter.id} type="button" aria-pressed={view.filter === filter.id} onClick={() => update({ filter: filter.id })}>
                  {filter.label}
                  <span className={filter.id === "attention" && counts.attention > 0 ? "seg-count tone-text-warning" : "seg-count"}>{counts[filter.id]}</span>
                </button>
              ))}
            </div>
            <label className="search-field">
              <CIcon name="search" />
              <span className="sr-only">Search containers by name, image or stack</span>
              <input type="search" value={view.query} maxLength={SEARCH_MAX} placeholder="Search name, image or stack"
                onChange={(event) => update({ query: event.target.value })} />
            </label>
            <div className="toolbar-end">
              <label className="sort-field">
                <span>Sort</span>
                <select value={view.sort} onChange={(event) => update({ sort: event.target.value as Sort })}>
                  {SORTS.map((sort) => <option key={sort.id} value={sort.id}>{sort.label}</option>)}
                </select>
              </label>
              <button type="button" className="quiet group-toggle" aria-pressed={view.group} onClick={() => update({ group: !view.group })}>
                <CIcon name="layers" />Group by stack
              </button>
            </div>
          </div>
          <div className="result-line">
            <span role="status">
              {filtered ? `Showing ${shown.length} of ${counts.all} containers` : `${counts.all} ${counts.all === 1 ? "container" : "containers"}`}
              {view.group ? ", grouped by stack" : ""}
            </span>
            {filtered && <button type="button" className="ghost clear-filters" onClick={() => update({ filter: "all", query: "" })}>Clear filters</button>}
          </div>
          {shown.length > MAX_ROWS && <p className="notice">Showing the first {MAX_ROWS}. Narrow the list with a filter or search.</p>}
          {counts.all === 0 ? (
            <StateMessage kind="empty" title="No containers reported">
              {current ? "Docker reports no containers on this host." : "Docker isn't reporting current readings, so containers may exist that aren't listed."}
            </StateMessage>
          ) : shown.length === 0 ? (
            <StateMessage kind="empty" title="No containers match">
              <p>Nothing matches {view.query.trim() ? <>“{view.query.trim()}”</> : "this filter"}{view.query.trim() && view.filter !== "all" ? ` among ${FILTERS.find((f) => f.id === view.filter)!.label.toLowerCase()} containers` : ""}.</p>
              <button type="button" className="quiet" onClick={() => update({ filter: "all", query: "" })}>Clear filters</button>
            </StateMessage>
          ) : wide ? (
            <ContainerTable groups={groups} grouped={view.group} handlers={handlers} caption={caption} />
          ) : (
            <ContainerCards groups={groups} grouped={view.group} handlers={handlers} />
          )}
        </section>
      )}
      {manage && (
        <div className="panel operations-panel">
          <ContainerJobs state={jobs} names={names} csrf={session.csrf_token} expired={expired} admin={admin} changed={changed} />
        </div>
      )}
      <ContainerDetail
        name={value ? view.detail : null}
        row={detailRow}
        docker={docker}
        now={now}
        jobs={detailRow ? jobs.jobs.filter((job) => job.plan.expected.resource === detailRow.resource.id) : []}
        canManage={manage}
        canReadStorage={admin}
        onAction={(action, opener) => detailRow && handlers.onAction(detailRow, action, opener)}
        onLogs={(opener) => detailRow && handlers.onLogs(detailRow, opener)}
        onClose={closeDetail}
        onShowOperation={handlers.onShowOperation}
      />
      {manage && <LifecycleDialog target={lifecycle} csrf={session.csrf_token} changed={changed} onClosed={() => setLifecycle(null)} />}
      {manage && <LogsDialog target={logs} onClosed={() => setLogs(null)} />}
    </>
  );
}
