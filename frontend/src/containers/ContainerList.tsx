/**
 * The container list: a dense table from 1280 px (the previous app's xl
 * breakpoint), cards below. One markup is rendered at a time. Each row shows
 * name, stack, image, a status pill and a separate health cue (both with
 * words), CPU and memory bars on the Overview's thresholds, any operation in
 * flight, and for managers one contextual action, Logs and a menu with the
 * rest. Viewers get no lifecycle or log controls.
 */
import { useEffect, useId, useState, type MouseEvent } from "react";
import type { ContainerAction } from "../../../contracts/generated/types";
import { Link } from "../app/router";
import { Icon } from "../ui/icons";
import { MetricBar, StatusBadge, type Tone } from "../ui/primitives";
import { formatPercent } from "../ui/format";
import { metricBarValue, metricTone, toneWord } from "../overview/overview-model";
import { ActionMenu, type MenuItem } from "./ActionMenu";
import { actionLabel, actionStates, imageLabel, jobBadge, primaryAction, type Group, type Row } from "./container-model";
import { CIcon, type ContainerIconName } from "./icons";

export const TABLE_QUERY = "(min-width: 1280px)";

export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => typeof window !== "undefined" && window.matchMedia(query).matches);
  useEffect(() => {
    const list = window.matchMedia(query);
    const change = () => setMatches(list.matches);
    change();
    list.addEventListener("change", change);
    return () => list.removeEventListener("change", change);
  }, [query]);
  return matches;
}

export function statusCue(row: Pick<Row, "resource" | "attention">): { label: string; tone: Tone } {
  const status = row.resource.status;
  switch (status) {
    case "running": return { label: "running", tone: "success" };
    case "paused": return { label: "paused", tone: "info" };
    case "restarting": return { label: "restarting", tone: "warning" };
    case "dead":
    case "missing": return { label: status, tone: "danger" };
    case "exited": return { label: "exited", tone: row.attention ? "warning" : "neutral" };
    default: return { label: status || "unknown", tone: "neutral" };
  }
}

export function healthCue(health: string | null): { label: string; tone: Tone; icon: ContainerIconName | null } {
  if (health === "healthy") return { label: "healthy", tone: "success", icon: "check" };
  if (health === "unhealthy") return { label: "unhealthy", tone: "danger", icon: "close" };
  if (health === "starting") return { label: "health starting", tone: "info", icon: "rotate" };
  return { label: "no healthcheck", tone: "neutral", icon: null };
}

export interface RowHandlers {
  canManage: boolean;
  now: number;
  /** False when the docker source is not current: readings show as Unknown. */
  current: boolean;
  /** Rows sit under their stack's heading, so they need not repeat the stack. */
  grouped?: boolean;
  onAction: (row: Row, action: ContainerAction, opener: HTMLElement) => void;
  onLogs: (row: Row, opener: HTMLElement) => void;
  onDetail: (row: Row, opener: HTMLElement) => void;
  onShowOperation: (id: string) => void;
}

const detailHref = (row: Row) => `/containers?container=${encodeURIComponent(row.name)}`;
const actionIcon: Record<ContainerAction, ContainerIconName> = { start: "play", stop: "square", restart: "rotate" };

function NameLink({ row, handlers, id }: { row: Row; handlers: RowHandlers; id: string }) {
  const follow = (event: MouseEvent<HTMLAnchorElement>) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    handlers.onDetail(row, event.currentTarget);
  };
  return <a id={id} className="container-name" href={detailHref(row)} onClick={follow} title={`Details of ${row.name}`}>{row.name}</a>;
}

/** A healthcheck's state in words and colour; nothing when the image defines none. */
function Health({ row }: { row: Row }) {
  const cue = healthCue(row.resource.health);
  if (!cue.icon) return null;
  return (
    <span className={`health-cue tone-text-${cue.tone}`}>
      <CIcon name={cue.icon} size={13} />
      <span>{cue.label}</span>
    </span>
  );
}

/** Break opportunities after "/" and before ":" or "@", so a long reference wraps where it reads well. */
function Breakable({ text }: { text: string }) {
  const parts = text.split(/(?<=\/)|(?=[:@])/);
  return <>{parts.map((part, i) => <span key={i}>{i > 0 && <wbr />}{part}</span>)}</>;
}

function Image({ row }: { row: Row }) {
  const { image } = row;
  return (
    <span className="image-ref" title={image.raw || undefined}>
      {image.registry && <span className="image-registry">{image.registry}/</span>}
      <span className="image-repo"><Breakable text={image.kind === "reference" ? image.repository : imageLabel(image)} /></span>
      {image.tag && <span className="image-tag">:{image.tag}</span>}
    </span>
  );
}

function Metric({ label, metric, value, current, name, running }: { label: string; metric: "cpu" | "memory"; value: number | null; current: boolean; name: string; running: boolean }) {
  const tone = metricTone(metric, value);
  // A stopped container uses nothing; Docker reports no figure for it.
  const idle = value === null && !running && current;
  const text = value === null ? (idle ? "Not running" : "Unknown") : formatPercent(value);
  return (
    <div className="row-metric">
      <span className={value === null ? (idle ? "metric-text metric-idle dim" : "metric-text dim") : `metric-text tone-text-${tone}`}>
        <span className="metric-label" aria-hidden="true">{label} </span>{text}
      </span>
      <MetricBar label={`${label} of ${name}`} value={metricBarValue(metric, value)} tone={tone}
        valueText={value === null ? (idle ? "Not running" : current ? "Unknown: not reported" : "Unknown: Docker readings are out of date") : `${text}, ${toneWord[tone]}`} />
    </div>
  );
}

function Notes({ row, handlers }: { row: Row; handlers: RowHandlers }) {
  const badge = jobBadge(row.job, handlers.now);
  if (!row.attention && !badge) return null;
  return (
    <div className="row-notes">
      {row.attention && <span className="row-attention"><Icon name="warning" size={13} />{row.attention}</span>}
      {badge && row.job && (
        <a className={`row-job tone-text-${badge.tone}`} href={`#operations-heading`}
          onClick={(event) => { event.preventDefault(); handlers.onShowOperation(row.job!.id); }}>
          <span className={`job-dot tone-bg-${badge.tone}`} aria-hidden="true" />{badge.label}
          <span className="sr-only">: show in recent operations</span>
        </a>
      )}
    </div>
  );
}

export function RowActions({ row, handlers, describedBy, compact }: { row: Row; handlers: RowHandlers; describedBy?: string; compact: boolean }) {
  const states = actionStates(row);
  const primary = primaryAction(row);
  const primaryState = states.find((s) => s.action === primary);
  const items: MenuItem[] = [
    ...states.map((state) => ({
      id: state.action,
      label: state.label,
      icon: actionIcon[state.action],
      disabled: state.disabled,
      onSelect: (trigger: HTMLButtonElement) => handlers.onAction(row, state.action, trigger),
    })),
    { id: "logs", label: "Logs", icon: "logs" as const, separatorBefore: true, onSelect: (trigger: HTMLButtonElement) => handlers.onLogs(row, trigger) },
    { id: "details", label: "Details", icon: "layers" as const, onSelect: (trigger: HTMLButtonElement) => handlers.onDetail(row, trigger) },
  ];
  return (
    <div className="row-actions">
      {primary && primaryState && (
        <button type="button" className="quiet row-primary" disabled={!!primaryState.disabled} title={primaryState.disabled ?? undefined}
          aria-describedby={describedBy} onClick={(event) => handlers.onAction(row, primary, event.currentTarget)}>
          <CIcon name={actionIcon[primary]} />{actionLabel(primary)}
        </button>
      )}
      <button type="button" className={compact ? "quiet icon-button row-logs" : "quiet row-logs"} aria-label="Logs" title={`Logs of ${row.name}`}
        aria-describedby={describedBy} onClick={(event) => handlers.onLogs(row, event.currentTarget)}>
        <CIcon name="logs" />{!compact && <span aria-hidden="true">Logs</span>}
      </button>
      <ActionMenu label={`More actions for ${row.name}`} items={items} />
    </div>
  );
}

function TableRow({ row, handlers }: { row: Row; handlers: RowHandlers }) {
  const id = useId();
  const status = statusCue(row);
  return (
    <tr className={row.attention ? "attention-row" : undefined}>
      <th scope="row">
        <NameLink row={row} handlers={handlers} id={`${id}-name`} />
        {!handlers.grouped && <span className="row-stack">{row.stack ? <><CIcon name="layers" size={12} />{row.stack}</> : <span className="dim">not in a stack</span>}</span>}
        <Notes row={row} handlers={handlers} />
      </th>
      <td className="cell-image"><Image row={row} /></td>
      <td className="cell-status">
        <StatusBadge tone={status.tone} label={status.label} />
        <Health row={row} />
      </td>
      <td className="cell-metric"><Metric label="CPU" metric="cpu" value={row.cpu} current={handlers.current} name={row.name} running={row.running} /></td>
      <td className="cell-metric"><Metric label="Memory" metric="memory" value={row.memory} current={handlers.current} name={row.name} running={row.running} /></td>
      {handlers.canManage && <td className="cell-actions"><RowActions row={row} handlers={handlers} describedBy={`${id}-name`} compact /></td>}
    </tr>
  );
}

function groupSummary(group: Group) {
  return `${group.running} of ${group.total} running`;
}

function GroupHeading({ group, as: Tag }: { group: Group; as: "span" | "h3" }) {
  return (
    <Tag className="group-heading">
      <span className="group-name">
        <CIcon name="layers" size={14} />
        {group.stack ? <Link to="/stacks" title={`Stack ${group.stack} on the Stacks page`}>{group.label}</Link> : group.label}
      </span>
      <span className={group.running === group.total ? "group-count tone-text-success" : group.running === 0 ? "group-count dim" : "group-count tone-text-warning"}>
        {groupSummary(group)}
      </span>
      {group.rows.length !== group.total && <span className="group-shown">{group.rows.length} shown</span>}
    </Tag>
  );
}

export function ContainerTable({ groups, grouped, handlers, caption }: { groups: Group[]; grouped: boolean; handlers: RowHandlers; caption: string }) {
  const columns = handlers.canManage ? 6 : 5;
  return (
    <div className="containers-table-wrap">
      <table className="containers-table">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            <th scope="col">Container</th>
            <th scope="col">Image</th>
            <th scope="col">Status</th>
            <th scope="col">CPU</th>
            <th scope="col">Memory</th>
            {handlers.canManage && <th scope="col"><span className="sr-only">Actions</span></th>}
          </tr>
        </thead>
        {groups.map((group) => (
          <tbody key={group.stack ?? "\u0000none"}>
            {grouped && (
              <tr className="group-row">
                <th scope="colgroup" colSpan={columns}><GroupHeading group={group} as="span" /></th>
              </tr>
            )}
            {group.rows.map((row) => <TableRow key={row.resource.id} row={row} handlers={handlers} />)}
          </tbody>
        ))}
      </table>
    </div>
  );
}

function Card({ row, handlers }: { row: Row; handlers: RowHandlers }) {
  const id = useId();
  const status = statusCue(row);
  return (
    <li className={row.attention ? "container-card attention-row" : "container-card"}>
      <div className="card-top">
        <div className="card-ident">
          <NameLink row={row} handlers={handlers} id={`${id}-name`} />
          {!handlers.grouped && <span className="row-stack">{row.stack ? <><CIcon name="layers" size={12} />{row.stack}</> : <span className="dim">not in a stack</span>}</span>}
        </div>
        <div className="card-status">
          <StatusBadge tone={status.tone} label={status.label} />
          <Health row={row} />
        </div>
      </div>
      <div className="card-image"><Image row={row} /></div>
      <Notes row={row} handlers={handlers} />
      <div className="card-metrics">
        <Metric label="CPU" metric="cpu" value={row.cpu} current={handlers.current} name={row.name} running={row.running} />
        <Metric label="Memory" metric="memory" value={row.memory} current={handlers.current} name={row.name} running={row.running} />
      </div>
      {handlers.canManage && <RowActions row={row} handlers={handlers} describedBy={`${id}-name`} compact={false} />}
    </li>
  );
}

export function ContainerCards({ groups, grouped, handlers }: { groups: Group[]; grouped: boolean; handlers: RowHandlers }) {
  return (
    <div className="container-groups">
      {groups.map((group) => (
        <div key={group.stack ?? "\u0000none"} className="card-group">
          {grouped && <GroupHeading group={group} as="h3" />}
          <ul className="container-cards" aria-label={grouped ? `${group.label}: ${groupSummary(group)}` : "Containers"}>
            {group.rows.map((row) => <Card key={row.resource.id} row={row} handlers={handlers} />)}
          </ul>
        </div>
      ))}
    </div>
  );
}
