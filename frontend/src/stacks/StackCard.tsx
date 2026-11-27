/**
 * One registered stack as a card: status, running-of-total bar, services by
 * name with each container's state and health, the job in flight, the
 * whole-stack actions, and the technical details (IDs, image digests, files)
 * behind a disclosure. Styling lives in stacks.css.
 */
import type { Resource, StackJob, StackReceipt } from "../../../contracts/generated/types";
import type { FileStatus, OtherProject, StackView } from "../stack-inventory-client";
import { StackActions, StackJobStrip } from "../StackActions";
import { Link } from "../app/router";
import { Badge, MetricBar, StatusBadge, type Tone } from "../ui/primitives";
import { CopyValue } from "../ui/copy";
import { Icon } from "../ui/icons";
import {
  countsSentence, omissionWords, runningPercent, serviceRows, stackStatus, unhealthyCount,
  type MemberRow, type ObservedStack, type OmittedRow, type ServiceRow,
} from "./stack-model";

export interface CardOperations { csrf: string; changed: () => void }

const barTone = (tone: Tone): Tone => (tone === "success" || tone === "warning" || tone === "danger" ? tone : "neutral");
const containerPath = (name: string) => `/containers?search=${encodeURIComponent(name)}`;
/** "24 Nov 2026, 06:12" in the viewer's locale; the raw value when it isn't a date. */
const startedWords = (value: string) => {
  const at = Date.parse(value);
  return Number.isFinite(at) ? new Date(at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : value;
};

const healthTone = (health: string): Tone => (health === "unhealthy" ? "danger" : health === "healthy" ? "success" : "info");

/**
 * A container row: a link to it on the Containers page when its name is
 * known. The service is named too when the container's name doesn't say it
 * (Compose names containers `project-service-1`, so usually it does).
 */
function MemberLine({ member, service }: { member: MemberRow; service: ServiceRow }) {
  const showService = !member.name || !member.name.toLowerCase().includes(service.name.toLowerCase());
  const body = (
    <>
      <span className={`member-dot tone-bg-${member.tone}`} aria-hidden="true" />
      <span className={member.name ? "member-name" : "member-name member-id"}>{member.label}</span>
      {showService && <span className="member-service"><span className="sr-only">service </span>{service.name}</span>}
      {!service.declared && <Badge tone="warning">not in registration</Badge>}
      {member.oneOff && <Badge>one-off</Badge>}
      {member.health && <Badge tone={healthTone(member.health)}>{member.health}</Badge>}
      <span className="member-state">{member.state}</span>
    </>
  );
  return member.name
    ? <Link to={containerPath(member.name)} className="member-row" title={`Open ${member.name} on the Containers page`}>{body}</Link>
    : <span className="member-row" title="This container's name isn't in the latest observation">{body}</span>;
}

/** Every service's containers as one list; a service without one says so. */
export function ServiceList({ rows, label, previous }: { rows: ServiceRow[]; label: string; previous: boolean }) {
  if (rows.length === 0) return <p className="stack-service-empty">No services are recorded for this stack.</p>;
  return (
    <ul className="stack-members" aria-label={label}>
      {rows.flatMap((service) => service.members.length > 0
        ? service.members.map((m) => <li key={m.resource}><MemberLine member={m} service={service} /></li>)
        : [<li key={`service:${service.name}`}>
          <span className="member-row member-empty">
            <span className="member-dot tone-bg-neutral" aria-hidden="true" />
            <span className="member-name">{service.name}</span>
            {!service.declared && <Badge tone="warning">not in registration</Badge>}
            <span className="member-state">{previous ? "no current reading" : "no container"}</span>
          </span>
        </li>])}
    </ul>
  );
}

export function FileDiagnostic({ value }: { value: FileStatus }) {
  if (value.state === "conflict") return <p className="error" role="status">Multiple Compose files: {value.candidates.join(", ")}. Resolve the conflict before edits or actions.</p>;
  if (value.state === "unsupported") return <p>Source interpretation is unsupported ({value.reason}). The source is preserved for review.</p>;
  if (value.state === "unavailable") return <p>Compose source inspection is unavailable.</p>;
  return <p>Compose sources have not been inspected.</p>;
}

/** The disclosure: project, files and every member with full ID and image digest. */
function StackDetails({ name, project, directory, files, fileStatus, rows, previous }: {
  name: string; project: string; directory: string; files: string[]; fileStatus?: FileStatus; rows: ServiceRow[]; previous: boolean;
}) {
  const members = rows.flatMap((s) => s.members);
  return (
    <details className="stack-details card-disclosure">
      <summary><Icon name="chevron" size={14} className="disclosure-chevron" /><span>Services and members · {rows.length}<span className="stack-details-hint"> · IDs, images and files</span></span></summary>
      <dl className="stack-facts">
        <dt>Compose project</dt><dd>{project}</dd>
        <dt>Directory</dt><dd><CopyValue value={directory} label={`${name} directory`} /></dd>
        <dt>{files.length === 1 ? "Compose file" : "Compose files"}</dt>
        <dd>{files.map((file) => <CopyValue key={file} value={file} label={`${name} Compose file`} />)}</dd>
        {fileStatus && <><dt>Compose source</dt><dd><FileDiagnostic value={fileStatus} /></dd></>}
      </dl>
      {members.length > 0 && (
        <div className="table-wrap" role="region" tabIndex={0} aria-label={`Members of ${name}`}>
          <table className="stack-member-table">
            <caption>{name} · {previous ? "previously observed members" : "observed members"}</caption>
            <thead><tr><th scope="col">Container</th><th scope="col">Role</th><th scope="col">Observed state</th><th scope="col">Container ID and image</th></tr></thead>
            <tbody>
              {members.map((m) => (
                <tr key={m.resource}>
                  <th scope="row"><span className="member-table-name">{m.label}</span><small>service {m.service}</small></th>
                  <td>{m.oneOff ? "One-off" : "Service / replica"}</td>
                  <td>{previous ? "Previously " : ""}{m.running ? "running" : "not running"}<small title={m.startedAt}>since {startedWords(m.startedAt)}</small></td>
                  <td className="member-ids">
                    <CopyValue value={m.resource.slice(10)} label={`${m.label} container ID`} />
                    <CopyValue value={m.image} label={`${m.label} image digest`} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </details>
  );
}

function OmittedList({ rows }: { rows: OmittedRow[] }) {
  if (rows.length === 0) return null;
  return (
    <div className="stack-omitted">
      <p>Also in this stack, not counted while the inventory can't read {rows.length === 1 ? "it" : "them"}:</p>
      <ul>
        {rows.map((o) => (
          <li key={o.resource}>
            {o.name ? <Link to={containerPath(o.name)} className="member-row"><span className="member-name">{o.name}</span><span className="member-state">{omissionWords[o.reason]}</span></Link>
              : <span className="member-row"><span className="member-name member-id">{o.shortId}</span><span className="member-state">{omissionWords[o.reason]}</span></span>}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function StackCard({ stack, unknown, index, live, omitted, job, receipt, operations, names }: {
  stack: StackView;
  /** Why the inventory can't be taken as current (page-level), or null. */
  unknown: string | null;
  index: Map<string, Resource>;
  /** The overview's docker source is current. */
  live: boolean;
  omitted: OmittedRow[];
  job: StackJob | null;
  receipt?: StackReceipt | null;
  operations?: CardOperations;
  names: Map<string, Resource>;
}) {
  const previous = unknown !== null;
  const status = stackStatus(stack, unknown);
  const rows = serviceRows(stack.services, index, { live, previous });
  const unhealthy = unhealthyCount(rows);
  const counts = stack.counts;
  const known = !!counts && !previous;
  const percent = runningPercent(counts, previous);
  const sentence = countsSentence(counts, previous);
  const busy = !!job && ["queued", "running", "verifying", "needs_intervention"].includes(job.state);
  return (
    <article className={`stack-card tone-${unhealthy > 0 && status.tone === "success" ? "warning" : status.tone}`} aria-label={`Stack ${stack.stack}`}>
      <header className="stack-card-head">
        <div className="stack-card-name">
          <h3><Icon name="stacks" size={16} className="tone-accent" />{stack.stack}</h3>
          <p className="stack-card-path" title={stack.working_directory}>{stack.working_directory}</p>
        </div>
        <div className="stack-card-badges">
          <StatusBadge label={status.label} tone={status.tone} />
          {unhealthy > 0 && <Badge tone="danger">{unhealthy} unhealthy</Badge>}
        </div>
      </header>
      <div className="stack-card-meter">
        <p className="stack-card-counts"><span>{sentence}</span>
          {known && counts.one_offs > 0 && <span className="stack-card-oneoffs">{counts.one_offs} one-off{counts.one_offs === 1 ? "" : "s"} ({counts.one_off_running} running)</span>}
        </p>
        <MetricBar value={percent} tone={barTone(status.tone)} label={`${stack.stack}: service containers running`}
          valueText={known ? `${counts.running} of ${counts.ordinary} running` : "Unknown"} />
      </div>
      {/* A page-level unknown is explained once, above the cards. */}
      {status.reason && !previous && <p className="stack-card-note">{status.reason}</p>}
      {!previous && stack.missing_services !== null && stack.missing_services > 0 && <p className="notice">{stack.missing_services} declared {stack.missing_services === 1 ? "service has" : "services have"} no ordinary container.</p>}
      {stack.files.state === "conflict" && <FileDiagnostic value={stack.files} />}
      {stack.issues.includes("identity_conflict") && <p className="error">Engine project labels differ from the registered directory or ordered Compose files.</p>}
      {stack.issues.includes("overlapping_project") && <p className="error">Another Engine project overlaps this stack. Membership counts are withheld.</p>}
      {stack.issues.includes("undeclared_services") && <p className="notice">Additional services are absent from the registration and are listed below.</p>}
      {stack.issues.includes("omitted_members") && <p className="notice">A container of this stack is changing, unsupported or beyond the first 64 and is left out; stack actions wait until it is read whole.</p>}
      {stack.issues.includes("incomplete_inventory") && <p className="notice">An Engine container's Compose labels could not be read, so it may belong to or overlap this stack; stack actions wait until every container is identified.</p>}
      <ServiceList rows={rows} label={`Services in ${stack.stack}`} previous={previous} />
      <OmittedList rows={omitted} />
      {job && <StackJobStrip job={job} receipt={receipt} names={names} />}
      {operations && <StackActions stack={stack} stale={previous} csrf={operations.csrf} changed={operations.changed} names={names} busy={busy} />}
      <StackDetails name={stack.stack} project={stack.project} directory={stack.working_directory} files={stack.config_files} fileStatus={stack.files} rows={rows} previous={previous} />
    </article>
  );
}

const relationLabels: Record<OtherProject["relation"], string> = {
  not_registered: "Not registered", catalog_unknown: "Registration unknown", engine_mismatch: "Different Engine",
  identity_conflict: "Registration identity conflict", overlapping: "Overlaps a registered stack",
};

/** A Compose project Docker reports that is not a managed stack: read-only. */
export function OtherProjectCard({ row, index, live, previous }: { row: OtherProject; index: Map<string, Resource>; live: boolean; previous: boolean }) {
  const rows = serviceRows(row.services, index, { live, previous });
  return (
    <article className="stack-card other-project" aria-label={`Compose project ${row.project.name}`}>
      <header className="stack-card-head">
        <div className="stack-card-name">
          <h4>{row.project.name}</h4>
          <p className="stack-card-path" title={row.project.working_directory}>{row.project.working_directory}</p>
        </div>
        <div className="stack-card-badges"><Badge tone={row.relation === "not_registered" ? "neutral" : "warning"}>{relationLabels[row.relation]}</Badge></div>
      </header>
      <ServiceList rows={rows} label={`Services in ${row.project.name}`} previous={previous} />
      <StackDetails name={row.project.name} project={row.project.name} directory={row.project.working_directory} files={row.project.config_files} rows={rows} previous={previous} />
    </article>
  );
}

/** A stack as the host observer groups it, shown when the inventory can't be read. */
export function ObservedStackCard({ stack }: { stack: ObservedStack }) {
  const percent = stack.total > 0 && stack.status.key !== "unknown" ? (stack.running / stack.total) * 100 : null;
  return (
    <article className={`stack-card observed tone-${stack.unhealthy > 0 && stack.status.tone === "success" ? "warning" : stack.status.tone}`} aria-label={`Observed stack ${stack.name}`}>
      <header className="stack-card-head">
        <div className="stack-card-name">
          <h4><Icon name="stacks" size={16} className="tone-accent" />{stack.name}</h4>
          <p className="stack-card-path">Seen by the host observer</p>
        </div>
        <div className="stack-card-badges">
          <StatusBadge label={stack.status.label} tone={stack.status.tone} />
          {stack.unhealthy > 0 && <Badge tone="danger">{stack.unhealthy} unhealthy</Badge>}
        </div>
      </header>
      <div className="stack-card-meter">
        <p className="stack-card-counts"><span>{stack.status.key === "unknown" ? "Current container counts are unknown." : `${stack.running} of ${stack.total} containers running`}</span></p>
        <MetricBar value={percent} tone={barTone(stack.status.tone)} label={`${stack.name}: containers running`} valueText={percent === null ? "Unknown" : `${stack.running} of ${stack.total} running`} />
      </div>
      {stack.containers.length > 0 && (
        <ul className="stack-members" aria-label={`Containers in ${stack.name}`}>
          {stack.containers.map((c) => {
            const unhealthy = c.health === "unhealthy" || c.status === "restarting" || c.status === "dead";
            const tone: Tone = unhealthy ? "danger" : c.status === "running" ? "success" : "neutral";
            return (
              <li key={c.id}>
                <Link to={containerPath(c.name)} className="member-row" title={`Open ${c.name} on the Containers page`}>
                  <span className={`member-dot tone-bg-${tone}`} aria-hidden="true" />
                  <span className="member-name">{c.name}</span>
                  {c.health && <Badge tone={c.health === "unhealthy" ? "danger" : c.health === "healthy" ? "success" : "info"}>{c.health}</Badge>}
                  <span className="member-state">{c.status}</span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </article>
  );
}
