import { ApiError } from "./api";
import type { AlertKind, HealthPolicyDefinition, HealthPolicyView, ReplaceHealthPolicy } from "../../contracts/generated/types";
export type { HealthPolicyDefinition, HealthPolicyView, HealthPolicySilence } from "../../contracts/generated/types";

const LIMIT = 64 * 1024;
export const healthKinds: AlertKind[] = ["container", "smart", "mount", "snapraid", "host"];
const encoder = new TextEncoder();
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const keys = (value: Record<string, unknown>, allowed: string[]) => Object.keys(value).every(key => allowed.includes(key));
const integer = (value: unknown, min: number, max = Number.MAX_SAFE_INTEGER): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max;
const noControls = (value: string) => !/[\x00-\x1f\x7f-\x9f]/.test(value);
function timestamp(value: unknown, expiry = false): boolean {
  if (typeof value !== "string") return false;
  const match = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,6}))?(Z|([+-])(\d{2}):(\d{2}))$/.exec(value);
  if (!match) return false;
  const [year, month, day, hour, minute, second] = match.slice(1, 7).map(Number);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  if (year < 1 || month < 1 || month > 12 || day < 1 || day > [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] || hour > 23 || minute > 59 || second > 59 || (expiry && Number(match[7] ?? "0") !== 0)) return false;
  const offsetHour = Number(match[10] ?? 0), offsetMinute = Number(match[11] ?? 0);
  if (offsetHour > 23 || offsetMinute > 59) return false;
  const date = new Date(0); date.setUTCFullYear(year, month - 1, day); date.setUTCHours(hour, minute, second, 0);
  const instant = date.getTime() / 1000 - (offsetHour * 3600 + offsetMinute * 60) * (match[9] === "-" ? -1 : 1);
  return instant >= 0 && instant <= 253_402_300_799;
}
export function validHealthPolicy(value: unknown): value is HealthPolicyDefinition {
  if (!object(value) || !keys(value, ["fail_threshold", "disabled", "required_mounts", "silences"]) || !integer(value.fail_threshold, 1, 16) || !Array.isArray(value.disabled) || value.disabled.length > 5 || new Set(value.disabled).size !== value.disabled.length || !value.disabled.every(kind => healthKinds.includes(kind)) || !Array.isArray(value.required_mounts) || value.required_mounts.length > 256 || new Set(value.required_mounts).size !== value.required_mounts.length || !Array.isArray(value.silences) || value.silences.length > 64) return false;
  if (!value.required_mounts.every(path => typeof path === "string" && path.startsWith("/") && noControls(path) && encoder.encode(path).byteLength <= 250)) return false;
  if (!value.silences.every(silence => object(silence) && keys(silence, ["kind", "key", "created_at", "expires_at", "reason"]) && healthKinds.includes(silence.kind as AlertKind) && typeof silence.key === "string" && silence.key.startsWith(`${silence.kind}:`) && silence.key.length > String(silence.kind).length + 1 && silence.key.trim() === silence.key && noControls(silence.key) && encoder.encode(silence.key).byteLength <= 256 && timestamp(silence.created_at) && (silence.expires_at === null || timestamp(silence.expires_at, true)) && typeof silence.reason === "string" && silence.reason.trim() === silence.reason && Array.from(silence.reason).length <= 200)) return false;
  return encoder.encode(JSON.stringify(value)).byteLength <= LIMIT;
}
function view(value: unknown): value is HealthPolicyView {
  return object(value) && keys(value, ["revision", "updated", "definition"]) && integer(value.revision, 0) && validHealthPolicy(value.definition) && (value.revision === 0 ? value.updated === null && value.definition.fail_threshold === 2 && value.definition.disabled.length === 0 && value.definition.required_mounts.length === 0 && value.definition.silences.length === 0 : integer(value.updated, 0, 253_402_300_799));
}
function unavailable(): ApiError {
  return new ApiError({ code: "unavailable", message: "Unable to confirm health settings. Refresh saved settings before trying again.", retry: true, audit_id: "health-policy-unavailable" });
}
function invalid(): ApiError {
  return new ApiError({ code: "invalid_input", message: "Check the threshold, literal absolute paths and silence dates. Settings exceed a supported value or limit.", retry: false, audit_id: "health-policy-input" });
}
function forbidden(): ApiError {
  return new ApiError({ code: "forbidden", message: "Health scheduling permission is required to view or change these settings.", retry: false, audit_id: "health-policy-403" });
}
async function request(init: RequestInit): Promise<HealthPolicyView> {
  const signal = init.signal ? AbortSignal.any([init.signal, AbortSignal.timeout(12_000)]) : AbortSignal.timeout(12_000);
  const response = await fetch("/api/v1/automation/health-policy", { ...init, signal, credentials: "same-origin", cache: "no-store" }).catch(() => { throw unavailable(); });
  if (response.status === 401) { await response.body?.cancel().catch(() => {}); throw new ApiError({ code: "unauthenticated", message: "Your session has expired. Sign in again.", retry: false, audit_id: "health-policy-401" }); }
  if (!response.body) throw response.status === 403 ? forbidden() : unavailable();
  const reader = response.body.getReader(); const decoder = new TextDecoder("utf-8", { fatal: true }); let bytes = 0, source = ""; let value: unknown;
  try {
    while (true) { const chunk = await reader.read(); if (chunk.done) break; bytes += chunk.value.byteLength; if (bytes > LIMIT + 4096) throw unavailable(); source += decoder.decode(chunk.value, { stream: true }); }
    value = JSON.parse(source + decoder.decode()) as unknown;
  } catch { throw response.status === 403 ? forbidden() : unavailable(); } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  if (!response.ok) {
    if (response.status === 403) {
      if (object(value) && keys(value, ["code", "message", "retry", "audit_id"]) && value.code === "expired" && typeof value.message === "string" && value.message.length <= 512 && typeof value.retry === "boolean" && typeof value.audit_id === "string" && value.audit_id.length <= 128) throw new ApiError({ code: "expired", message: "The reply arrived after its deadline. Refresh to check saved health settings.", retry: false, audit_id: "health-policy-expired" });
      throw forbidden();
    }
    if (object(value) && typeof value.code === "string" && value.code.length <= 64 && typeof value.message === "string" && value.message.length <= 512 && typeof value.retry === "boolean" && typeof value.audit_id === "string" && value.audit_id.length <= 128) throw new ApiError(value as unknown as ConstructorParameters<typeof ApiError>[0]);
    throw unavailable();
  }
  if (!view(value)) throw unavailable();
  return value;
}
function same(left: HealthPolicyDefinition, right: HealthPolicyDefinition): boolean {
  return left.fail_threshold === right.fail_threshold && JSON.stringify(left.disabled) === JSON.stringify(right.disabled) && JSON.stringify(left.required_mounts) === JSON.stringify(right.required_mounts) && left.silences.length === right.silences.length && left.silences.every((value, i) => {
    const other = right.silences[i]; return value.kind === other.kind && value.key === other.key && value.created_at === other.created_at && value.expires_at === other.expires_at && value.reason === other.reason;
  });
}
export const healthPolicyClient = {
  read: (signal?: AbortSignal) => request({ signal }),
  replace: async (input: ReplaceHealthPolicy, csrf: string, signal?: AbortSignal) => {
    if (!integer(input.revision, 0) || !validHealthPolicy(input.definition)) throw invalid();
    const value = await request({ method: "POST", headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf }, body: JSON.stringify(input), signal });
    if (value.revision === 0 || (value.revision !== input.revision && value.revision !== input.revision + 1) || !same(value.definition, input.definition)) throw unavailable();
    return value;
  },
};
