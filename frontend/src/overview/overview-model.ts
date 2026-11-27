/**
 * Derivations for the Overview, from the overview payload alone. Pure and
 * unit tested. Rules:
 * - thresholds and colours follow the previous app's home page;
 * - "healthy" is claimed only when every source is fresh, the observation
 *   itself is current, host metrics are present and nothing needs attention;
 * - unknown is said, never rendered as zero.
 */
import type { Freshness, Overview, Resource } from "../../../contracts/generated/types";

export type MetricTone = "success" | "warning" | "danger" | "neutral";
export type MetricKey = "cpu" | "memory" | "disk" | "temperature";

/** Warning and danger thresholds (inclusive), as in pi-health's dashboard. */
export const thresholds: Record<MetricKey, { warning: number; danger: number }> = {
  cpu: { warning: 60, danger: 85 },
  memory: { warning: 70, danger: 90 },
  disk: { warning: 75, danger: 90 },
  temperature: { warning: 65, danger: 80 },
};
/** Temperature bars are drawn against 90 °C (the Pi throttles from 85 °C). */
export const TEMPERATURE_SCALE = 90;

const finite = (value: number | null | undefined): value is number => typeof value === "number" && Number.isFinite(value);

export function metricTone(key: MetricKey, value: number | null | undefined): MetricTone {
  if (!finite(value)) return "neutral";
  const t = thresholds[key];
  return value >= t.danger ? "danger" : value >= t.warning ? "warning" : "success";
}

/** The bar fill (0–100) for a metric; temperature is scaled to 90 °C. */
export function metricBarValue(key: MetricKey, value: number | null | undefined): number | null {
  if (!finite(value)) return null;
  const percent = key === "temperature" ? (value / TEMPERATURE_SCALE) * 100 : value;
  return Math.min(100, Math.max(0, percent));
}

export const toneWord: Record<MetricTone, string> = { success: "normal", warning: "high", danger: "very high", neutral: "unknown" };

/** Seconds after which the overview itself (not just a source) counts as old. */
export const OBSERVATION_MAX_AGE = 120;

export function sourceStale(source: Freshness, now: number): boolean {
  return source.state !== "fresh" || source.sampled_at === null || now - source.sampled_at > source.max_age_seconds;
}

export interface WorkloadCounts {
  containers: { total: number; running: number; stopped: number; unhealthy: number };
  stacks: { total: number; running: number; partial: number; stopped: number };
}

/** Restarting or dead containers, or a failing healthcheck, need attention. */
export function containerUnhealthy(r: Resource): boolean {
  return r.health === "unhealthy" || r.status === "restarting" || r.status === "dead";
}

export function workloadCounts(resources: Resource[]): WorkloadCounts {
  const containers = resources.filter((r) => r.kind === "container");
  const stacks = resources.filter((r) => r.kind === "stack");
  const unhealthy = containers.filter(containerUnhealthy).length;
  const running = containers.filter((r) => r.status === "running").length;
  return {
    containers: {
      total: containers.length,
      running,
      unhealthy,
      stopped: containers.filter((r) => r.status !== "running" && !containerUnhealthy(r)).length,
    },
    stacks: {
      total: stacks.length,
      running: stacks.filter((r) => r.status === "running").length,
      partial: stacks.filter((r) => r.status !== "running" && r.status !== "stopped").length,
      stopped: stacks.filter((r) => r.status === "stopped").length,
    },
  };
}

export type HealthState = "healthy" | "attention" | "critical" | "unknown";

export interface Issue {
  id: string;
  label: string;
  tone: "warning" | "danger" | "neutral";
  /** Where to look: a route path. */
  path: string;
}

export interface HealthSummary {
  state: HealthState;
  title: string;
  detail: string;
  issues: Issue[];
}

const metricLabels: Record<MetricKey, string> = { cpu: "CPU", memory: "Memory", disk: "Root disk", temperature: "Temperature" };

export function healthSummary(overview: Overview | null, now: number, readFailed = false): HealthSummary {
  if (!overview) {
    return {
      state: "unknown",
      title: readFailed ? "Can't reach the host" : "Checking your host…",
      detail: readFailed ? "LimeOS couldn't read the host's status. It will keep trying." : "Waiting for the first observation.",
      issues: [],
    };
  }
  const issues: Issue[] = [];
  const host = overview.host;
  const metricValues: Record<MetricKey, number | null | undefined> = {
    cpu: host?.cpu_percent, memory: host?.memory_percent, disk: host?.disk_percent, temperature: host?.temperature_celsius,
  };
  for (const key of ["temperature", "disk", "memory", "cpu"] as MetricKey[]) {
    const tone = metricTone(key, metricValues[key]);
    if (tone === "warning" || tone === "danger") {
      const value = metricValues[key] as number;
      issues.push({
        id: `metric:${key}`, tone, path: "/",
        label: `${metricLabels[key]} is ${toneWord[tone]}: ${key === "temperature" ? `${value.toFixed(1)} °C` : `${value.toFixed(1)}%`}`,
      });
    }
  }
  for (const r of overview.resources) {
    if (r.status === "missing") {
      const storage = r.kind === "disk" || r.kind === "partition" || r.kind === "pool";
      issues.push({ id: `missing:${r.id}`, tone: storage ? "danger" : "warning", path: storage ? "/storage" : r.kind === "stack" ? "/stacks" : "/containers", label: `${r.name} (${r.kind}) is missing` });
    } else if (r.kind === "container" && containerUnhealthy(r)) {
      issues.push({ id: `container:${r.id}`, tone: "warning", path: "/containers", label: `${r.name} is ${r.health === "unhealthy" ? "unhealthy" : r.status}` });
    } else if (r.kind === "stack" && r.status === "degraded") {
      issues.push({ id: `stack:${r.id}`, tone: "warning", path: "/stacks", label: `${r.name} stack is only partly running` });
    } else if (r.status === "ambiguous") {
      issues.push({ id: `ambiguous:${r.id}`, tone: "warning", path: "/storage", label: `${r.name} can't be identified uniquely` });
    }
  }
  const stale = overview.sources.filter((s) => sourceStale(s, now));
  for (const s of stale) {
    issues.push({
      id: `source:${s.source}`, tone: "neutral", path: "/",
      label: s.state === "unavailable" ? `${s.source} readings are unavailable` : s.state === "loading" ? `${s.source} readings haven't arrived yet` : `${s.source} readings are out of date`,
    });
  }
  const old = !Number.isFinite(overview.generated_at) || now - overview.generated_at > OBSERVATION_MAX_AGE;
  if (old) issues.push({ id: "observation:old", tone: "neutral", path: "/", label: "This observation is more than two minutes old" });
  if (!host) issues.push({ id: "host:none", tone: "neutral", path: "/", label: "Host metrics are not available" });
  if (overview.sources.length === 0) issues.push({ id: "sources:none", tone: "neutral", path: "/", label: "No observation sources have reported yet" });

  const danger = issues.some((i) => i.tone === "danger");
  const warning = issues.some((i) => i.tone === "warning");
  const unknown = issues.some((i) => i.tone === "neutral") || readFailed;
  const count = issues.filter((i) => i.tone !== "neutral").length;
  const gaps = unknown ? " Some readings are missing or out of date, so there may be more." : "";
  if (danger) return { state: "critical", title: "Needs attention now", detail: `${count} ${count === 1 ? "problem" : "problems"} found.${gaps}`, issues };
  if (warning) return { state: "attention", title: "Something needs a look", detail: `${count} ${count === 1 ? "thing" : "things"} to review.${gaps}`, issues };
  if (unknown) return { state: "unknown", title: "Status partly unknown", detail: "Nothing looks wrong in what LimeOS can see, but some readings are missing or out of date.", issues };
  return { state: "healthy", title: "All systems normal", detail: "Every source is current; metrics, containers and storage look fine.", issues };
}
