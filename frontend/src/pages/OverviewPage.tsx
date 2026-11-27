import { useOverview } from "../app/overview-data";
import { SourceList } from "../ObservationView";
import { HistorySection } from "../overview/HistorySection";
import { Checks, HealthBanner, HostFacts, MetricCard, Workloads, healthLabel, healthTone } from "../overview/OverviewParts";
import { healthSummary, sourceStale, type MetricKey } from "../overview/overview-model";
import { PageHeader, StatusBadge } from "../ui/primitives";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import { canManage, type PageProps } from "./Pages";

const metrics: MetricKey[] = ["cpu", "memory", "disk", "temperature"];

export function OverviewPage({ session, expired }: PageProps) {
  const { value, error, now, refreshKey } = useOverview();
  const summary = healthSummary(value, now, !!error);
  const stale = (name: string) => {
    const source = value?.sources.find((s) => s.source === name);
    return !source || sourceStale(source, now);
  };
  const current = value ? value.sources.filter((s) => !sourceStale(s, now)).length : 0;
  return (
    <>
      <PageHeader
        title="Overview"
        status={<StatusBadge label={value ? healthLabel[summary.state] : "syncing"} tone={value ? healthTone[summary.state] : "info"} />}
        description="Your host at a glance: health, resources and what's running."
        actions={<OverviewRefresh />}
      />
      <div className="overview">
        <OverviewPending />
        {value && <HealthBanner summary={summary} />}
        {value && (
          <div className="metric-grid">
            {metrics.map((m) => <MetricCard key={m} metric={m} host={value.host} stale={!!value.host && stale("host")} />)}
            <HostFacts host={value.host} />
          </div>
        )}
        <div className="overview-pair">
          {value && <Workloads value={value} dockerStale={stale("docker")} />}
          <Checks sources={value?.sources ?? null} now={now} canManage={canManage(session)} />
        </div>
        <HistorySection refreshKey={refreshKey} expired={expired} />
        {value && (
          <details className="sources-disclosure">
            <summary>Observation sources · {current} of {value.sources.length} current</summary>
            <SourceList value={value} />
          </details>
        )}
      </div>
    </>
  );
}
