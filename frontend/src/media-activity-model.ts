/** Proposed leaf DTOs. Replace with registered generated contracts at integration. */
export type Provider = "jellyfin" | "emby" | "sabnzbd" | "transmission" | "sonarr" | "radarr" | "lidarr" | "readarr";
export type ServiceState = "ready" | "partial" | "stopped" | "stale" | "future" | "identity_mismatch" | "unavailable" | "credential_unavailable" | "timeout" | "invalid_response" | "unsupported";
export interface Target { container: { resource: string; image: string; started_at: string; running: boolean }; name: string; provider: Provider }
export interface Stream { title: string; subtitle: string; user: string; client: string; device: string; play_method: string | null; paused: boolean; transcoding: boolean; transcode_reason: string | null; bitrate: number | null; progress_percent: number | null; position_seconds: number | null; duration_seconds: number | null }
export interface Download { name: string; state: string; progress_percent: number | null; size_bytes: number | null; remaining_bytes: number | null; speed_bytes: number | null; eta: string | null }
export interface Queue { total: number; observed: number; problems: number; complete: boolean; items: { title: string; state: string; status: string }[] }
export interface Sourced<T> { source: string; data: T }
export interface ActivitySnapshot {
  version: 1; generated_at: number; sampled_at: number | null; max_age_seconds: 8;
  source: "fresh" | "stale" | "future" | "incomplete" | "unavailable" | "identity_mismatch";
  complete: boolean; services: { target: Target; sampled_at: number | null; state: ServiceState }[];
  streams: Sourced<Stream>[]; downloads: Sourced<Download>[]; queues: Sourced<Queue>[];
  omitted_streams: number; omitted_downloads: number;
  totals: { streams: number | null; transcodes: number | null; downloads: number | null; download_speed: number | null; queued: number | null; queue_problems: number | null };
}
export function activityExpired(value: ActivitySnapshot, now: number): boolean {
  return !Number.isSafeInteger(now) || value.sampled_at === null || now < value.sampled_at || now < value.generated_at || now - value.sampled_at > value.max_age_seconds || !["fresh", "incomplete"].includes(value.source);
}
