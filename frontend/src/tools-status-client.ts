/** Proposed DTOs; shared schema generation/registration remains central. */
export type Freshness = "fresh" | "unavailable" | "stale" | "future";
export type ConfigIssue = "unavailable" | "incomplete" | "invalid_document" | "unsupported_options" | "invalid_path" | "invalid_port" | "limit";
export type ServiceIssue = "unavailable" | "incomplete" | "limit" | "invalid_properties";
export interface ToolsConfig { share_path: string; port: number }
export type ConfigState = { state: "defaults" | "stored"; config: ToolsConfig } | { state: "unknown"; reason: ConfigIssue };
export type PayloadState = "absent" | "present" | "unexpected_type" | "unavailable";
export type ServiceState = { state: "active" | "inactive" | "failed" | "activating" | "deactivating" | "reloading" | "not_found" | "masked" | "load_error" } | { state: "unknown"; reason: ServiceIssue };
export interface Component<T> { sampled_at: number | null; freshness: Freshness; value: T | null }
export interface ToolsStatus {
  version: 1; generated_at: number; max_age_seconds: 90;
  config: Component<ConfigState>; payload: Component<PayloadState>; service: Component<ServiceState>;
  applied_configuration: "not_established";
}
export class ToolsReadError extends Error {
  constructor(message: string, readonly status = 0) { super(message); }
}
const MAX_TIME = 253402300799;
const MAX_BYTES = 8192;
const encoder = new TextEncoder();
function invalid(): never { throw new ToolsReadError("The host returned an invalid tools observation."); }
const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : invalid();
const integer = (v: unknown, max = MAX_TIME): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : invalid();
function choice<T extends string>(v: unknown, choices: readonly T[]): T { return typeof v === "string" && choices.includes(v as T) ? v as T : invalid(); }
function config(value: unknown): ConfigState {
  const r = record(value); const state = choice(r.state, ["defaults", "stored", "unknown"]);
  if (state === "unknown") return { state, reason: choice(r.reason, ["unavailable", "incomplete", "invalid_document", "unsupported_options", "invalid_path", "invalid_port", "limit"]) };
  const c = record(r.config); const path = c.share_path;
  if (typeof path !== "string" || !path.startsWith("/") || encoder.encode(path).length > 512 || /[\p{Cc}\uD800-\uDFFF]/u.test(path)) invalid();
  if (path.slice(1).split("/").some(part => !part || part === "." || part === ".." || encoder.encode(part).length > 255 || /^\p{White_Space}|\p{White_Space}$/u.test(part))) invalid();
  const port = integer(c.port, 65535); if (port === 0) invalid();
  if (state === "defaults" && (path !== "/srv/copyparty" || port !== 3923)) invalid();
  return { state, config: { share_path: path, port } };
}
function service(value: unknown): ServiceState {
  const r = record(value); const state = choice(r.state, ["active", "inactive", "failed", "activating", "deactivating", "reloading", "not_found", "masked", "load_error", "unknown"]);
  return state === "unknown" ? { state, reason: choice(r.reason, ["unavailable", "incomplete", "limit", "invalid_properties"]) } : { state };
}
function component<T>(value: unknown, generated: number, decode: (v: unknown) => T): Component<T> {
  const r = record(value); const sampled = r.sampled_at === null ? null : integer(r.sampled_at);
  const freshness = choice(r.freshness, ["fresh", "unavailable", "stale", "future"]);
  if ((freshness === "unavailable") !== (sampled === null)) invalid();
  if (freshness !== "fresh" && r.value !== null) invalid();
  if (sampled !== null) {
    if (freshness === "fresh" && (sampled > generated || generated - sampled > 90)) invalid();
    if (freshness === "stale" && generated - sampled <= 90) invalid();
    if (freshness === "future" && sampled <= generated) invalid();
  }
  return { sampled_at: sampled, freshness, value: freshness === "fresh" ? decode(r.value) : null };
}
export function decodeToolsStatus(value: unknown): ToolsStatus {
  const r = record(value); if (r.version !== 1 || r.max_age_seconds !== 90 || r.applied_configuration !== "not_established") invalid();
  const generated = integer(r.generated_at);
  const result: ToolsStatus = { version: 1, generated_at: generated, max_age_seconds: 90,
    config: component(r.config, generated, config), payload: component(r.payload, generated, v => choice(v, ["absent", "present", "unexpected_type", "unavailable"])),
    service: component(r.service, generated, service), applied_configuration: "not_established" };
  if (encoder.encode(JSON.stringify(result)).length > MAX_BYTES) invalid();
  return result;
}
/** Local invalidation only. Withheld values can be restored only by another read. */
export function ageToolsStatus(value: ToolsStatus, now = Date.now() / 1000): ToolsStatus | null {
  const seconds = Math.floor(now);
  if (!Number.isFinite(now) || seconds < value.generated_at || seconds > MAX_TIME || seconds < 0) return null;
  const age = <T>(part: Component<T>): Component<T> => part.freshness === "fresh" && part.sampled_at !== null && seconds - part.sampled_at > 90
    ? { sampled_at: part.sampled_at, freshness: "stale", value: null } : part;
  const config = age(value.config), payload = age(value.payload), service = age(value.service);
  return config === value.config && payload === value.payload && service === value.service ? value : { ...value, config, payload, service };
}
export function toolsExpiryDelay(value: ToolsStatus, now: number): number | null {
  if (ageToolsStatus(value, now) === null) return null;
  const deadlines = [value.config, value.payload, value.service].filter(part => part.freshness === "fresh" && part.sampled_at !== null).map(part => part.sampled_at! + 91);
  return deadlines.length ? Math.max(1, (Math.min(...deadlines) - now) * 1000) : null;
}
async function body(response: Response, signal: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader() ?? invalid(); const bytes = new Uint8Array(MAX_BYTES); let size = 0;
  const cancel = () => { void reader.cancel().catch(() => {}); };
  const check = () => { if (signal.aborted) throw new ToolsReadError("The tools read was cancelled."); };
  signal.addEventListener("abort", cancel, { once: true }); if (signal.aborted) cancel();
  try {
    check();
    for (;;) {
      const chunk = await reader.read(); check(); if (chunk.done) break;
      if (chunk.value.length > bytes.length - size) { cancel(); invalid(); }
      bytes.set(chunk.value, size); size += chunk.value.length;
    }
  } finally { signal.removeEventListener("abort", cancel); reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, size))) as unknown; }
  catch { return invalid(); }
}
export async function readToolsStatus(signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<ToolsStatus> {
  const controller = new AbortController(); const abort = () => controller.abort();
  signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
  const timer = setTimeout(abort, 12_000);
  try {
    if (controller.signal.aborted) throw new ToolsReadError("The tools read was cancelled.");
    const response = await fetcher("/api/v1/deployment/tools/copyparty", { method: "GET", credentials: "same-origin", cache: "no-store", redirect: "error", headers: { Accept: "application/json" }, signal: controller.signal });
    if (response.status === 401 || response.status === 403) {
      void response.body?.cancel().catch(() => {});
      throw new ToolsReadError(response.status === 401 ? "Your session has expired." : "You no longer have access to tools status.", response.status);
    }
    if (!response.ok) { void response.body?.cancel().catch(() => {}); throw new ToolsReadError("Tools status is unavailable. Refresh to try again.", response.status); }
    return decodeToolsStatus(await body(response, controller.signal));
  } finally { clearTimeout(timer); signal.removeEventListener("abort", abort); }
}
