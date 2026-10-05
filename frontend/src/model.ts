import type {
  Freshness,
  Overview,
  Resource,
  ResourceKind,
} from "../../contracts/generated/types";
export function percent(value: number | null): string {
  return value === null || !Number.isFinite(value)
    ? "Unknown"
    : `${value.toFixed(1)}%`;
}
export function sampled(at: number | null): string {
  return at === null
    ? "Waiting for first sample"
    : new Date(at * 1000).toLocaleTimeString();
}
export function stale(source: Freshness, at = Date.now() / 1000): boolean {
  return (
    source.state !== "fresh" ||
    source.sampled_at === null ||
    at - source.sampled_at > source.max_age_seconds
  );
}
export function page(
  resources: Resource[],
  kind: ResourceKind | undefined,
  offset: number,
): Resource[] {
  return resources
    .filter((r) => kind === undefined || r.kind === kind)
    .slice(offset, offset + 20);
}
export function partial(value: Overview, at = Date.now() / 1000): boolean {
  return (
    value.sources.some((s) => stale(s, at) || s.warnings.length > 0) ||
    value.resources.some((r) => r.status === "missing")
  );
}
