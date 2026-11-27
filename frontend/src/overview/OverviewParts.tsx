/** The Overview's building blocks: health banner, metric cards, workloads, checks. */
import type { Freshness, HostMetrics, Overview } from "../../../contracts/generated/types";
import { Link } from "../app/router";
import { Icon, type IconName } from "../ui/icons";
import { Badge, Card, CardTitle, MetricBar, Stat, StatusBadge, type Tone } from "../ui/primitives";
import { formatCelsius, formatDuration, formatLoad, formatPercent } from "../ui/format";
import { containerUnhealthy, metricBarValue, metricTone, sourceStale, toneWord, workloadCounts, type HealthSummary, type MetricKey, type MetricTone } from "./overview-model";

const bannerIcon: Record<HealthSummary["state"], IconName> = { healthy: "ok", attention: "warning", critical: "critical", unknown: "unknown" };
const bannerTone: Record<HealthSummary["state"], Tone> = { healthy: "success", attention: "warning", critical: "danger", unknown: "neutral" };
export const healthLabel: Record<HealthSummary["state"], string> = { healthy: "healthy", attention: "attention", critical: "critical", unknown: "unknown" };
export const healthTone = bannerTone;

export function HealthBanner({ summary }: { summary: HealthSummary }) {
  const shown = summary.issues.slice(0, 6);
  const more = summary.issues.length - shown.length;
  const problems = summary.issues.filter((i) => i.tone !== "neutral").length;
  return (
    <section className={`health-banner tone-${bannerTone[summary.state]}`} aria-labelledby="health-title">
      <div className="health-head">
        <Icon name={bannerIcon[summary.state]} size={22} />
        <div className="health-text">
          <h2 id="health-title">{summary.title}</h2>
          <p>{summary.detail}</p>
        </div>
        <Badge tone={bannerTone[summary.state]}>{problems} {problems === 1 ? "issue" : "issues"}</Badge>
      </div>
      {shown.length > 0 && (
        <ul className="health-issues">
          {shown.map((issue) => (
            <li key={issue.id} className={`issue-${issue.tone}`}>
              <Link to={issue.path}>
                <span className="issue-mark" aria-hidden="true" />
                <span className="issue-label">{issue.label}</span>
                <span className="issue-kind">{issue.tone === "neutral" ? "unknown" : issue.tone === "danger" ? "act" : "review"}</span>
              </Link>
            </li>
          ))}
          {more > 0 && <li className="issue-more">and {more} more</li>}
        </ul>
      )}
    </section>
  );
}

const metricIcon: Record<MetricKey, IconName> = { cpu: "cpu", memory: "memory", disk: "disk", temperature: "temperature" };
const metricName: Record<MetricKey, string> = { cpu: "CPU", memory: "Memory", disk: "Root disk", temperature: "Temperature" };
const textTone = (tone: MetricTone) => `tone-text-${tone}`;

export function MetricCard({ metric, host, stale }: { metric: MetricKey; host: HostMetrics | null; stale: boolean }) {
  const raw = host ? { cpu: host.cpu_percent, memory: host.memory_percent, disk: host.disk_percent, temperature: host.temperature_celsius }[metric] : null;
  const tone = metricTone(metric, raw);
  const value = metric === "temperature" ? formatCelsius(raw) : formatPercent(raw);
  return (
    <Card className="metric-card">
      <div className="metric-head">
        <h3>{metricName[metric]}</h3>
        <Icon name={metricIcon[metric]} size={15} />
      </div>
      <p className={`metric-value ${textTone(tone)}`}>{value}</p>
      <p className="metric-note">
        {stale ? <span className="tone-text-warning">out of date</span> : tone === "neutral" ? "not reported" : toneWord[tone]}
      </p>
      <MetricBar label={metricName[metric]} value={metricBarValue(metric, raw)} tone={tone === "neutral" ? "neutral" : tone} valueText={`${value}${tone === "neutral" ? "" : `, ${toneWord[tone]}`}`} />
    </Card>
  );
}

export function FactCard({ label, value, note, icon }: { label: string; value: string; note: string; icon: IconName }) {
  return (
    <Card className="metric-card fact-card">
      <div className="metric-head"><h3>{label}</h3><Icon name={icon} size={15} /></div>
      <p className="metric-value">{value}</p>
      <p className="metric-note">{note}</p>
    </Card>
  );
}

export function HostFacts({ host }: { host: HostMetrics | null }) {
  const uptime = host?.uptime_seconds ?? null;
  const since = typeof uptime === "number" && Number.isFinite(uptime)
    ? `since ${new Date(Date.now() - uptime * 1000).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}`
    : "not reported";
  return (
    <>
      <FactCard label="Load" icon="activity" value={formatLoad(host?.load_one)} note={host?.load_one === null || host?.load_one === undefined ? "not reported" : "1-minute average"} />
      <FactCard label="Uptime" icon="clock" value={formatDuration(uptime)} note={since} />
    </>
  );
}

export function Workloads({ value, dockerStale }: { value: Overview; dockerStale: boolean }) {
  const { containers, stacks } = workloadCounts(value.resources);
  const attention = value.resources
    .filter((r) => r.kind === "container" && (r.status !== "running" || containerUnhealthy(r)))
    .sort((a, b) => Number(containerUnhealthy(b)) - Number(containerUnhealthy(a)) || a.name.localeCompare(b.name));
  return (
    <Card as="section" aria-labelledby="workloads-title">
      <CardTitle icon="activity" id="workloads-title" actions={<Badge tone={dockerStale ? "warning" : "neutral"}>{dockerStale ? "out of date" : "current"}</Badge>}>Workloads</CardTitle>
      <div className="workloads">
        <Link to="/containers" className="stat-link" aria-label={`Containers: ${containers.total} in total, ${containers.running} running, ${containers.unhealthy} unhealthy, ${containers.stopped} stopped`}>
          <Stat label="Containers" icon="containers" value={containers.total}>
            <Badge tone="success">{containers.running} running</Badge>
            <Badge tone={containers.unhealthy ? "danger" : "neutral"}>{containers.unhealthy} unhealthy</Badge>
            <Badge tone={containers.stopped ? "warning" : "neutral"}>{containers.stopped} stopped</Badge>
          </Stat>
        </Link>
        <Link to="/stacks" className="stat-link" aria-label={`Stacks: ${stacks.total} in total, ${stacks.running} running, ${stacks.partial} partly running, ${stacks.stopped} stopped`}>
          <Stat label="Stacks" icon="stacks" value={stacks.total}>
            <Badge tone="success">{stacks.running} running</Badge>
            <Badge tone={stacks.partial ? "warning" : "neutral"}>{stacks.partial} partly running</Badge>
            <Badge tone={stacks.stopped ? "warning" : "neutral"}>{stacks.stopped} stopped</Badge>
          </Stat>
        </Link>
      </div>
      {attention.length > 0 && (
        <div className="attention-list">
          <h3>Not running as expected</h3>
          <ul>
            {attention.slice(0, 5).map((r) => (
              <li key={r.id}>
                <span className="attention-name">{r.name}</span>
                {r.parent?.startsWith("stack:") && <span className="attention-stack">{r.parent.slice(6)}</span>}
                <StatusBadge tone={containerUnhealthy(r) ? "danger" : "warning"} label={r.health === "unhealthy" ? "unhealthy" : r.status} />
              </li>
            ))}
          </ul>
          {attention.length > 5 && <p className="card-note">and {attention.length - 5} more on the Containers page.</p>}
        </div>
      )}
      {dockerStale && <p className="card-note">Docker readings are out of date, so these counts may have changed.</p>}
    </Card>
  );
}

export function Checks({ sources, now, canManage }: { sources: Freshness[] | null; now: number; canManage: boolean }) {
  const current = sources ? sources.filter((s) => !sourceStale(s, now)).length : 0;
  const all = sources?.length ?? 0;
  return (
    <Card as="section" aria-labelledby="checks-title">
      <CardTitle icon="alerts" id="checks-title">Alerts and checks</CardTitle>
      <div className="checks">
        <div className="check-row">
          <span>Observation sources</span>
          {sources ? <StatusBadge tone={all > 0 && current === all ? "success" : current === 0 ? "danger" : "warning"} label={`${current} of ${all} current`} />
            : <StatusBadge tone="neutral" label="unknown" />}
        </div>
        <p className="card-note">Recorded alerts and notifications from the health checks are on the Alerts page.</p>
        <ul className="check-links">
          <li><Link to="/alerts" className="card-link">Alert history and notifications <Icon name="chevron" size={14} /></Link></li>
          {canManage && <li><Link to="/alerts/policy" className="card-link">Manage alert policy <Icon name="chevron" size={14} /></Link></li>}
          {canManage && <li><Link to="/alerts/schedules" className="card-link">Health schedules <Icon name="chevron" size={14} /></Link></li>}
        </ul>
      </div>
    </Card>
  );
}
