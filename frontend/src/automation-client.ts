import { ApiError } from "./api";
import type { ObserveHealthEnabled, ObserveHealthScheduleInput, ScheduleView } from "../../contracts/generated/types";
export type { ObserveHealthScheduleInput, ScheduleDefinition, ScheduleView } from "../../contracts/generated/types";

export type { AlertState, Notification } from "../../contracts/generated/types";
import type { AlertState, Notification, AutomationPage } from "../../contracts/generated/types";
export type Page<T> = AutomationPage<T>;
const MAX_RESPONSE = 64 * 1024;
function unavailable(status: number): ApiError {
  return new ApiError({ code: "unavailable", message: "Unable to read automation state. Refresh to check the outcome.", retry: true, audit_id: `http-${status}` });
}
function forbidden(): ApiError {
  return new ApiError({ code: "forbidden", message: "The request is outside the permitted scope.", retry: false, audit_id: "http-403" });
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
  const runtime = value.runtime;
  if (value.status === "scheduled") {
    if (definition.operation !== "observe_health" || row.paused !== false || !object(runtime) ||
      Object.keys(runtime).some(key => !["phase", "last_attempt"].includes(key)) ||
      !["waiting", "checking", "partial", "unavailable"].includes(String(runtime.phase)) ||
      !(runtime.last_attempt === null || time(runtime.last_attempt)) ||
      (runtime.phase === "waiting") !== (runtime.last_attempt === null)) return false;
  } else if (runtime !== undefined && runtime !== null) return false;
  if (occurrence !== null) {
    if (!object(occurrence) || !object(occurrence.key) || !object(occurrence.outcome) || !text(occurrence.key.schedule, 128) || !integer(occurrence.key.revision, 1) || !time(occurrence.key.due)) return false;
    if (!["blocked", "skipped", "job"].includes(String(occurrence.outcome.outcome))) return false;
    if (occurrence.outcome.outcome === "job" && (!text(occurrence.outcome.id, 128) || !["planned", "waiting_approval", "queued", "running", "verifying", "succeeded", "failed", "canceled", "precondition_changed", "outcome_unknown", "needs_intervention"].includes(String(occurrence.outcome.state)))) return false;
  }
  if (!(window === null || (object(window) && integer(window.start_minute, 0, 1439) && integer(window.end_minute, 0, 1439) && window.start_minute !== window.end_minute))) return false;
  return text(row.id, 128) && integer(row.revision, 1) && typeof row.paused === "boolean" && (row.settled_through === null || time(row.settled_through)) &&
    operations.includes(String(definition.operation)) && time(definition.anchor) && integer(definition.interval_seconds, 30, 2_678_400) && integer(definition.grace_seconds, 0, 86400) && definition.grace_seconds < definition.interval_seconds && ["coalesce_latest", "skip"].includes(String(definition.missed)) &&
    ["awaiting_producer", "scheduled", "authority_suspended", "paused", "blocked"].includes(String(value.status));
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
async function request<T>(path: string, init?: RequestInit, base = "/api/v1/automation"): Promise<T> {
  const timeout = AbortSignal.timeout(12_000);
  const signal = init?.signal ? AbortSignal.any([init.signal, timeout]) : timeout;
  const response = await fetch(`${base}${path}`, { ...init, credentials: "same-origin", cache: "no-store", signal }).catch(() => { throw unavailable(0); });
  if (response.status === 401) {
    await response.body?.cancel().catch(() => undefined);
    throw new ApiError({ code: "unauthenticated", message: "Your session has expired. Sign in again.", retry: false, audit_id: "http-401" });
  }
  const value = await boundedJSON(response).catch(() => { throw response.status === 403 ? forbidden() : unavailable(response.status); });
  if (!response.ok) {
    if (object(value) && typeof value.code === "string" && typeof value.message === "string" && value.message.length <= 512 && typeof value.audit_id === "string" && value.audit_id.length <= 128 && typeof value.retry === "boolean") {
      if (response.status === 403 && value.code !== "forbidden" && value.code !== "expired") throw forbidden();
      throw new ApiError(value as unknown as ConstructorParameters<typeof ApiError>[0]);
    }
    throw response.status === 403 ? forbidden() : unavailable(response.status);
  }
  if (!object(value)) throw unavailable(response.status);
  return value as T;
}
async function page<T>(path: string, guard: (value: unknown) => value is T, after?: string, signal?: AbortSignal): Promise<Page<T>> {
  const value = await request<Page<T>>(path + (after ? `?after=${encodeURIComponent(after)}` : ""), { signal });
  if (!Array.isArray(value.items) || value.items.length > 32 || !value.items.every(guard) || typeof value.partial !== "boolean" || !(value.next === null || text(value.next, 256))) throw unavailable(200);
  return value;
}
function post<T>(path: string, body: unknown, csrf: string, signal?: AbortSignal): Promise<T> {
  return request<T>(path, { method: "POST", headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf }, body: JSON.stringify(body), signal });
}
async function checked<T>(value: Promise<T>, guard: (value: unknown) => value is T): Promise<T> {
  const result = await value;
  if (!guard(result)) throw unavailable(200);
  return result;
}
export const automationClient = {
  enableHealth: (password: string, csrf: string, signal?: AbortSignal): Promise<ObserveHealthEnabled> => {
    if (!password || new TextEncoder().encode(password).byteLength > 1024) return Promise.reject(new ApiError({ code: "invalid_input", message: "Enter your current password, up to 1,024 bytes.", retry: false, audit_id: "client-password" }));
    return checked(request<ObserveHealthEnabled>("/auth/capabilities/observe-health/enable", {
      method: "POST", headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf }, body: JSON.stringify({ password }), signal,
    }, "/api/v1"), (value): value is ObserveHealthEnabled => object(value) && value.enabled === true && typeof value.reauthenticate === "boolean" && Object.keys(value).every(key => key === "enabled" || key === "reauthenticate"));
  },
  schedules: (after?: string, signal?: AbortSignal) => page<ScheduleView>("/schedules", schedule, after, signal),
  alerts: (after?: string, signal?: AbortSignal) => page<AlertState>("/alerts", alert, after, signal),
  notifications: (after?: string, signal?: AbortSignal) => page<Notification>("/notifications", notification, after, signal),
  create: (input: ObserveHealthScheduleInput, csrf: string, signal?: AbortSignal) => checked(post<ScheduleView>("/schedules", input, csrf, signal), schedule),
  pause: (row: ScheduleView, csrf: string, signal?: AbortSignal) => checked(post<ScheduleView>(`/schedules/${encodeURIComponent(row.schedule.id)}/pause`, { revision: row.schedule.revision, paused: !row.schedule.paused }, csrf, signal), schedule),
  acknowledge: (row: Notification, csrf: string, signal?: AbortSignal) => checked(post<Notification>(`/notifications/${encodeURIComponent(row.id)}/acknowledgment`, { revision: row.revision }, csrf, signal), notification),
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
