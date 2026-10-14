import { ApiError } from "./api";

// Proposed B1 wire shapes. Integrator replaces these with generated contract imports.
export type ScheduledOperation = "observe_health" | "backup" | "protection_sync" | "protection_scrub" | "container_update" | "package_reconcile";
export type ScheduleDefinition = {
  operation: ScheduledOperation; anchor: number; interval_seconds: number;
  grace_seconds: number; missed: "coalesce_latest" | "skip";
  window: { start_minute: number; end_minute: number } | null;
};
export type ScheduleView = {
  schedule: { id: string; revision: number; paused: boolean; definition: ScheduleDefinition; settled_through: number | null };
  status: "active" | "paused" | "blocked";
  last_occurrence: { key: { schedule: string; revision: number; due: number }; outcome: { outcome: "blocked" | "skipped" } | { outcome: "job"; id: string; state: string } } | null;
};
export type AlertState = {
  key: string; kind: "container" | "smart" | "mount" | "snapraid" | "host"; last_sequence: number;
  incident: { phase: "clear"; failures: number } | { phase: "open"; generation: number; severity: "warning" | "critical"; summary: string; announced: boolean };
};
export type Notification = {
  id: string; revision: number;
  event: { key: string; generation: number; event: "incident" | "recovery"; kind: AlertState["kind"]; severity: "warning" | "critical"; summary: string; at: number };
  channel: "in_app" | "idempotent_receiver" | "external";
  delivery: { state: "pending" | "blocked" } | { state: "in_flight"; started_at: number } | { state: "delivered" | "ambiguous"; at: number } | { state: "failed"; retry_at: number | null };
  attempts: number; acknowledged_at: number | null;
};
export type Page<T> = { items: T[]; next: string | null; partial: boolean };
const MAX_RESPONSE = 64 * 1024;
function unavailable(status: number): ApiError {
  return new ApiError({ code: "unavailable", message: "Unable to read automation state. Refresh to check the outcome.", retry: true, audit_id: `http-${status}` });
}
async function boundedJSON(response: Response): Promise<unknown> {
  if (!response.body) throw unavailable(response.status);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let bytes = 0; let source = "";
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      bytes += chunk.value.byteLength;
      if (bytes > MAX_RESPONSE) throw unavailable(response.status);
      source += decoder.decode(chunk.value, { stream: true });
    }
    source += decoder.decode();
    return JSON.parse(source) as unknown;
  } finally { await reader.cancel().catch(() => undefined); reader.releaseLock(); }
}
function object(value: unknown): value is Record<string, unknown> { return !!value && typeof value === "object" && !Array.isArray(value); }
const kinds = ["container", "smart", "mount", "snapraid", "host"];
const operations = ["observe_health", "backup", "protection_sync", "protection_scrub", "container_update", "package_reconcile"];
function integer(value: unknown, min = 0, max = Number.MAX_SAFE_INTEGER): value is number { return typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max; }
function text(value: unknown, cap: number): value is string { return typeof value === "string" && value.length > 0 && new TextEncoder().encode(value).byteLength <= cap && !/[\x00-\x1f\x7f]/.test(value); }
function time(value: unknown): value is number { return integer(value, 0, 253_402_300_799); }
function schedule(value: unknown): value is ScheduleView {
  if (!object(value) || !object(value.schedule) || !object(value.schedule.definition)) return false;
  const row = value.schedule; const definition = row.definition as Record<string, unknown>;
  const window = definition.window;
  const occurrence = value.last_occurrence;
  if (occurrence !== null) {
    if (!object(occurrence) || !object(occurrence.key) || !object(occurrence.outcome) || !text(occurrence.key.schedule, 128) || !integer(occurrence.key.revision, 1) || !time(occurrence.key.due)) return false;
    if (!["blocked", "skipped", "job"].includes(String(occurrence.outcome.outcome))) return false;
    if (occurrence.outcome.outcome === "job" && (!text(occurrence.outcome.id, 128) || !["planned", "waiting_approval", "queued", "running", "verifying", "succeeded", "failed", "canceled", "precondition_changed", "outcome_unknown", "needs_intervention"].includes(String(occurrence.outcome.state)))) return false;
  }
  if (!(window === null || (object(window) && integer(window.start_minute, 0, 1439) && integer(window.end_minute, 0, 1439) && window.start_minute !== window.end_minute))) return false;
  return text(row.id, 128) && integer(row.revision, 1) && typeof row.paused === "boolean" && (row.settled_through === null || time(row.settled_through)) &&
    operations.includes(String(definition.operation)) && time(definition.anchor) && integer(definition.interval_seconds, 30, 2_678_400) && integer(definition.grace_seconds, 0, 86400) && definition.grace_seconds < definition.interval_seconds && ["coalesce_latest", "skip"].includes(String(definition.missed)) &&
    ["active", "paused", "blocked"].includes(String(value.status));
}
function alert(value: unknown): value is AlertState {
  if (!object(value) || !object(value.incident) || !text(value.key, 256) || !kinds.includes(String(value.kind)) || !integer(value.last_sequence)) return false;
  const incident = value.incident;
  return incident.phase === "clear" ? integer(incident.failures, 0, 16) : incident.phase === "open" && integer(incident.generation, 1) && ["warning", "critical"].includes(String(incident.severity)) && text(incident.summary, 512) && typeof incident.announced === "boolean";
}
function notification(value: unknown): value is Notification {
  if (!object(value) || !object(value.event) || !object(value.delivery)) return false;
  const event = value.event; const delivery = value.delivery;
  if (!text(value.id, 128) || !integer(value.revision, 1) || !integer(value.attempts, 0, 4) || !(value.acknowledged_at === null || time(value.acknowledged_at)) || !["in_app", "external", "idempotent_receiver"].includes(String(value.channel)) ||
    !text(event.key, 256) || !integer(event.generation, 1) || !["incident", "recovery"].includes(String(event.event)) || !kinds.includes(String(event.kind)) || !["warning", "critical"].includes(String(event.severity)) || !text(event.summary, 512) || !time(event.at)) return false;
  switch (delivery.state) {
    case "pending": case "blocked": return true;
    case "in_flight": return time(delivery.started_at);
    case "delivered": case "ambiguous": return time(delivery.at);
    case "failed": return delivery.retry_at === null || time(delivery.retry_at);
    default: return false;
  }
}
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/v1/automation${path}`, { ...init, credentials: "same-origin", signal: AbortSignal.timeout(12_000) }).catch(() => { throw unavailable(0); });
  const value = await boundedJSON(response).catch(() => { throw unavailable(response.status); });
  if (!response.ok) {
    if (object(value) && typeof value.code === "string" && typeof value.message === "string" && value.message.length <= 512 && typeof value.audit_id === "string" && value.audit_id.length <= 128 && typeof value.retry === "boolean") {
      throw new ApiError(value as unknown as ConstructorParameters<typeof ApiError>[0]);
    }
    throw unavailable(response.status);
  }
  if (!object(value)) throw unavailable(response.status);
  return value as T;
}
async function page<T>(path: string, guard: (value: unknown) => value is T, after?: string): Promise<Page<T>> {
  const value = await request<Page<T>>(path + (after ? `?after=${encodeURIComponent(after)}` : ""));
  if (!Array.isArray(value.items) || value.items.length > 32 || !value.items.every(guard) || typeof value.partial !== "boolean" || !(value.next === null || text(value.next, 256))) throw unavailable(200);
  return value;
}
function post<T>(path: string, body: unknown, csrf: string): Promise<T> {
  return request<T>(path, { method: "POST", headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf }, body: JSON.stringify(body) });
}
async function checked<T>(value: Promise<T>, guard: (value: unknown) => value is T): Promise<T> {
  const result = await value;
  if (!guard(result)) throw unavailable(200);
  return result;
}
export const automationClient = {
  schedules: (after?: string) => page<ScheduleView>("/schedules", schedule, after),
  alerts: (after?: string) => page<AlertState>("/alerts", alert, after),
  notifications: (after?: string) => page<Notification>("/notifications", notification, after),
  create: (definition: ScheduleDefinition, csrf: string) => checked(post<ScheduleView>("/schedules", { definition }, csrf), schedule),
  pause: (row: ScheduleView, csrf: string) => checked(post<ScheduleView>(`/schedules/${encodeURIComponent(row.schedule.id)}/pause`, { revision: row.schedule.revision, paused: !row.schedule.paused }, csrf), schedule),
  acknowledge: (row: Notification, csrf: string) => checked(post<Notification>(`/notifications/${encodeURIComponent(row.id)}/acknowledgment`, { revision: row.revision }, csrf), notification),
};

export function deliveryLabel(row: Notification): string {
  switch (row.delivery.state) {
    case "failed": return row.delivery.retry_at === null ? "Delivery failed · retries exhausted" : "Delivery failed · retry scheduled";
    case "ambiguous": return "Delivery outcome unknown · check receiver before retrying";
    case "in_flight": return "Delivering";
    case "blocked": return "Delivery blocked";
    case "pending": return "Delivery queued";
    case "delivered": return "Delivered";
  }
}
