/**
 * Geometry for the dependency-free SVG history charts (ported from the
 * previous app's performance history). Pure and unit tested.
 */
import type { MetricPoint } from "../../../contracts/generated/types";

export type SeriesKey = "cpu_percent" | "memory_percent" | "temperature_celsius" | "disk_percent";

export const CHART_WIDTH = 1000;
export const CHART_HEIGHT = 200;

const finite = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

export interface TimeSpan { from: number; to: number }

/** The time span to draw: the API's own range when given, else the points'. */
export function timeSpan(points: MetricPoint[], from?: number | null, to?: number | null): TimeSpan {
  const first = points[0]?.at ?? 0, last = points.at(-1)?.at ?? first;
  const start = finite(from) ? Math.min(from, first) : first;
  const end = finite(to) ? Math.max(to, last) : last;
  return end > start ? { from: start, to: end } : { from: start - 1, to: start + 1 };
}

export function xFor(at: number, span: TimeSpan, width = CHART_WIDTH): number {
  return ((at - span.from) / (span.to - span.from)) * width;
}

export function yFor(value: number, domain: [number, number], height = CHART_HEIGHT): number {
  const [lo, hi] = domain;
  const bounded = Math.min(hi, Math.max(lo, value));
  return height - ((bounded - lo) / Math.max(hi - lo, 1)) * height;
}

/**
 * SVG path data for one series. A missing value, or a gap of more than two
 * buckets between samples, breaks the line: no line is drawn across
 * unknown time.
 */
export function seriesPaths(points: MetricPoint[], key: SeriesKey, domain: [number, number], span: TimeSpan, bucketSeconds: number, width = CHART_WIDTH, height = CHART_HEIGHT): string[] {
  const paths: string[] = [];
  let commands: string[] = [];
  let previousAt: number | null = null;
  const flush = () => {
    if (commands.length === 1) commands.push(commands[0].replace(/^M/, "L")); // a lone sample still shows as a dot
    if (commands.length) paths.push(commands.join(" "));
    commands = [];
  };
  for (const point of points) {
    const value = point[key];
    if (!finite(value)) { flush(); previousAt = null; continue; }
    if (previousAt !== null && bucketSeconds > 0 && point.at - previousAt > bucketSeconds * 2) flush();
    commands.push(`${commands.length ? "L" : "M"} ${xFor(point.at, span, width).toFixed(1)} ${yFor(value, domain, height).toFixed(1)}`);
    previousAt = point.at;
  }
  flush();
  return paths;
}

export interface SeriesSummary { current: number | null; min: number | null; average: number | null; max: number | null }

export function summarize(points: MetricPoint[], key: SeriesKey): SeriesSummary {
  const values = points.map((p) => p[key]).filter(finite);
  if (values.length === 0) return { current: null, min: null, average: null, max: null };
  const latest = [...points].reverse().find((p) => finite(p[key]));
  return {
    current: latest ? (latest[key] as number) : null,
    min: Math.min(...values),
    max: Math.max(...values),
    average: values.reduce((a, b) => a + b, 0) / values.length,
  };
}

/** Percentages use 0–100; temperature fits its values in 10-degree steps, at least 30 wide. */
export function domainFor(key: SeriesKey, points: MetricPoint[]): [number, number] {
  if (key !== "temperature_celsius") return [0, 100];
  const values = points.map((p) => p.temperature_celsius).filter(finite);
  if (values.length === 0) return [20, 80];
  const lo = Math.max(0, Math.floor((Math.min(...values) - 5) / 10) * 10);
  const hi = Math.max(lo + 30, Math.ceil((Math.max(...values) + 5) / 10) * 10);
  return [lo, hi];
}

/** The sample nearest in time to a horizontal position (0–1) across the chart. */
export function nearestIndex(points: MetricPoint[], ratio: number, span: TimeSpan): number | null {
  if (points.length === 0) return null;
  const at = span.from + Math.min(1, Math.max(0, ratio)) * (span.to - span.from);
  let best = 0;
  for (let i = 1; i < points.length; i++) if (Math.abs(points[i].at - at) < Math.abs(points[best].at - at)) best = i;
  return best;
}

/** Three time labels: start, middle, end. */
export function axisTimes(span: TimeSpan): number[] {
  return [span.from, (span.from + span.to) / 2, span.to];
}
