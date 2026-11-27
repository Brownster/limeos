/** Display formatting shared by every page. Unknown is always said, never zero. */
export const UNKNOWN = "Unknown";

const finite = (value: number | null | undefined): value is number =>
  typeof value === "number" && Number.isFinite(value);

export function formatPercent(value: number | null | undefined): string {
  return finite(value) ? `${value.toFixed(1)}%` : UNKNOWN;
}

export function formatCelsius(value: number | null | undefined): string {
  return finite(value) ? `${value.toFixed(1)} °C` : UNKNOWN;
}

export function formatBytes(value: number | null | undefined): string {
  if (!finite(value) || value < 0) return UNKNOWN;
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let size = value, unit = 0;
  while (size >= 1000 && unit < units.length - 1) { size /= 1000; unit++; }
  return `${unit === 0 || size >= 100 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

/** "3d 4h", "5h 12m", "42m"; seconds below a minute read "under a minute". */
export function formatDuration(seconds: number | null | undefined): string {
  if (!finite(seconds) || seconds < 0) return UNKNOWN;
  const days = Math.floor(seconds / 86400), hours = Math.floor((seconds % 86400) / 3600), minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return minutes > 0 ? `${minutes}m` : "under a minute";
}

export function formatLoad(value: number | null | undefined): string {
  return finite(value) ? value.toFixed(2) : UNKNOWN;
}

/** Wall-clock time for an epoch-seconds instant, in the viewer's locale. */
export function formatClock(at: number | null | undefined): string {
  return finite(at) ? new Date(at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : UNKNOWN;
}

/** "just now", "40 s ago", "5 min ago", "3 h ago". */
export function formatAge(at: number | null | undefined, now = Date.now() / 1000): string {
  if (!finite(at)) return "never";
  const age = Math.max(0, now - at);
  if (age < 5) return "just now";
  if (age < 60) return `${Math.round(age)} s ago`;
  if (age < 3600) return `${Math.round(age / 60)} min ago`;
  if (age < 86400) return `${Math.round(age / 3600)} h ago`;
  return `${Math.round(age / 86400)} d ago`;
}
