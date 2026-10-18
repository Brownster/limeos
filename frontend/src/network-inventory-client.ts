/** Proposed leaf DTOs; shared contract registration/generation remains central. */
export type NetworkHealth = "healthy" | "unhealthy" | "starting" | "not_configured" | "unknown";
export type NetworkSource = "fresh" | "stale" | "future" | "incomplete" | "engine_mismatch" | "unavailable";
export type LinkState = "resolved" | "missing_provider" | "ambiguous_provider" | "unsupported_container_prefix" | "missing_project" | "self_reference" | "cyclic_provider" | "chained_provider";
export type GroupState = "observed_running" | "provider_stopped" | "provider_unhealthy" | "provider_starting" | "health_unknown" | "provider_network_unknown" | "unsupported_topology" | "orphan_hints";
export interface NetworkContainer {
  id: string; name: string; running: boolean; health: NetworkHealth;
  compose: { project: { name: string; working_directory: string; config_files: string[] }; service: string; one_off: boolean } | null;
}
export interface NetworkMember {
  container: NetworkContainer; reference_kind: "container" | "service"; target: string;
  state: LinkState; provider_id: string | null; hinted_provider_id: string | null; dependency_hint: string | null;
}
export interface NetworkGroup { provider: NetworkContainer; state: GroupState; member_ids: string[]; hinted_orphan_ids: string[] }
export interface NetworkInventory {
  version: 1; generated_at: number; source: NetworkSource; sampled_at: number | null; max_age_seconds: 90;
  observed_containers: number | null; independent_containers: number | null; unknown_mode_containers: number | null;
  groups: NetworkGroup[]; members: NetworkMember[];
}
export class NetworkReadError extends Error {
  constructor(message: string, readonly status = 0) { super(message); }
}
const MAX_TIME = 253402300799;
const MAX_BYTES = 65_536;
const encoder = new TextEncoder();
function invalid(): never { throw new NetworkReadError("The host returned an invalid network inventory."); }
const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : invalid();
const list = (v: unknown, max = 16): unknown[] => Array.isArray(v) && v.length <= max ? v : invalid();
const integer = (v: unknown, max = MAX_TIME): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : invalid();
const text = (v: unknown, max: number): string => typeof v === "string" && v.length > 0 && encoder.encode(v).length <= max && !/[\u0000-\u001f\u007f]/u.test(v) ? v : invalid();
const bool = (v: unknown): boolean => typeof v === "boolean" ? v : invalid();
function choice<T extends string>(v: unknown, values: readonly T[]): T { return typeof v === "string" && values.includes(v as T) ? v as T : invalid(); }
function name(v: unknown, max: number, project = false): string {
  const value = text(v, max);
  if (!(project ? /^[a-z0-9][a-z0-9_-]*$/ : /^[A-Za-z0-9][A-Za-z0-9_.-]*$/).test(value)) invalid();
  return value;
}
function path(v: unknown): string {
  const value = text(v, 512); const parts = value.slice(1).split("/");
  if (!value.startsWith("/") || value.includes(",") || parts.length > 16 || parts.some(p => !p || p === "." || p === "..")) invalid();
  return value;
}
function id(v: unknown): string { const value = text(v, 64); return /^[0-9a-f]{64}$/.test(value) ? value : invalid(); }
const nullableId = (v: unknown): string | null => v === null ? null : id(v);
function ids(v: unknown): string[] {
  const values = list(v).map(id);
  if (values.some((value, i) => i > 0 && value <= values[i - 1])) invalid();
  return values;
}
function container(v: unknown): NetworkContainer {
  const r = record(v); let compose: NetworkContainer["compose"] = null;
  if (r.compose !== null) {
    const c = record(r.compose); const p = record(c.project); const files = list(p.config_files, 4).map(path);
    if (files.length === 0 || new Set(files).size !== files.length) invalid();
    compose = { project: { name: name(p.name, 64, true), working_directory: path(p.working_directory), config_files: files }, service: name(c.service, 64), one_off: bool(c.one_off) };
  }
  return { id: id(r.id), name: name(r.name, 128), running: bool(r.running), health: choice(r.health, ["healthy", "unhealthy", "starting", "not_configured", "unknown"]), compose };
}
export function decodeNetworkInventory(value: unknown): NetworkInventory {
  const r = record(value); if (r.version !== 1 || r.max_age_seconds !== 90) invalid();
  const generated = integer(r.generated_at);
  const source = choice(r.source, ["fresh", "stale", "future", "incomplete", "engine_mismatch", "unavailable"]);
  const sampled = r.sampled_at === null ? null : integer(r.sampled_at);
  if ((source === "unavailable") !== (sampled === null)) invalid();
  if (sampled !== null) {
    if (["fresh", "incomplete"].includes(source) && (sampled > generated || generated - sampled > 90)) invalid();
    if (source === "future" && sampled <= generated) invalid();
    if (source === "stale" && generated - sampled <= 90) invalid();
  }
  const groups = list(r.groups).map(value => {
    const g = record(value);
    return { provider: container(g.provider), state: choice(g.state, ["observed_running", "provider_stopped", "provider_unhealthy", "provider_starting", "health_unknown", "provider_network_unknown", "unsupported_topology", "orphan_hints"]), member_ids: ids(g.member_ids), hinted_orphan_ids: ids(g.hinted_orphan_ids) };
  });
  const members = list(r.members).map(value => {
    const m = record(value); const kind = choice(m.reference_kind, ["container", "service"]);
    return { container: container(m.container), reference_kind: kind, target: name(m.target, kind === "service" ? 64 : 128),
      state: choice(m.state, ["resolved", "missing_provider", "ambiguous_provider", "unsupported_container_prefix", "missing_project", "self_reference", "cyclic_provider", "chained_provider"]),
      provider_id: nullableId(m.provider_id), hinted_provider_id: nullableId(m.hinted_provider_id), dependency_hint: m.dependency_hint === null ? null : name(m.dependency_hint, 64) };
  });
  const count = (v: unknown) => v === null ? null : integer(v, 16);
  const observed = count(r.observed_containers), independent = count(r.independent_containers), unknown = count(r.unknown_mode_containers);
  if (source !== "fresh") {
    if (observed !== null || independent !== null || unknown !== null || groups.length || members.length) invalid();
  } else {
    if (observed === null || independent === null || unknown === null || independent + unknown + members.length !== observed) invalid();
    const containers = new Map<string, NetworkContainer>(); const names = new Map<string, string>();
    const admit = (c: NetworkContainer) => {
      const existing = containers.get(c.id);
      if (existing && JSON.stringify(existing) !== JSON.stringify(c)) invalid();
      if (names.has(c.name) && names.get(c.name) !== c.id) invalid();
      containers.set(c.id, c); names.set(c.name, c.id);
    };
    for (const g of groups) admit(g.provider);
    for (const m of members) admit(m.container);
    if (containers.size > observed) invalid();
    if (groups.some((g, i) => i > 0 && g.provider.id <= groups[i - 1].provider.id) || members.some((m, i) => i > 0 && m.container.id <= members[i - 1].container.id)) invalid();
    const groupById = new Map(groups.map(g => [g.provider.id, g]));
    for (const m of members) {
      const hasProvider = ["resolved", "self_reference", "cyclic_provider", "chained_provider"].includes(m.state);
      if (hasProvider !== (m.provider_id !== null)) invalid();
      if (m.state === "resolved" && m.provider_id === m.container.id) invalid();
      if (m.state === "self_reference" && m.provider_id !== m.container.id) invalid();
      if (m.provider_id !== null && !groupById.get(m.provider_id)?.member_ids.includes(m.container.id)) invalid();
      if (m.hinted_provider_id !== null && (m.state !== "missing_provider" || m.reference_kind !== "container" || m.dependency_hint === null || !groupById.get(m.hinted_provider_id)?.hinted_orphan_ids.includes(m.container.id))) invalid();
    }
    for (const g of groups) {
      const actual = members.filter(m => m.provider_id === g.provider.id).map(m => m.container.id);
      const hints = members.filter(m => m.hinted_provider_id === g.provider.id).map(m => m.container.id);
      if (!actual.length && !hints.length || JSON.stringify(actual) !== JSON.stringify(g.member_ids) || JSON.stringify(hints) !== JSON.stringify(g.hinted_orphan_ids)) invalid();
      const unsupported = members.some(m => m.provider_id === g.provider.id && m.state !== "resolved");
      if (unsupported ? g.state !== "unsupported_topology" : hints.length ? g.state !== "orphan_hints" : ["unsupported_topology", "orphan_hints"].includes(g.state)) invalid();
      if (g.state === "observed_running" && (!g.provider.running || !["healthy", "not_configured"].includes(g.provider.health))) invalid();
      if (g.state === "provider_stopped" && g.provider.running) invalid();
      if (g.state === "provider_unhealthy" && (!g.provider.running || g.provider.health !== "unhealthy")) invalid();
      if (g.state === "provider_starting" && (!g.provider.running || g.provider.health !== "starting")) invalid();
      if (g.state === "health_unknown" && (!g.provider.running || g.provider.health !== "unknown")) invalid();
    }
  }
  const admitted: NetworkInventory = { version: 1, generated_at: generated, source, sampled_at: sampled, max_age_seconds: 90,
    observed_containers: observed, independent_containers: independent, unknown_mode_containers: unknown, groups, members };
  if (encoder.encode(JSON.stringify(admitted)).length > MAX_BYTES) invalid();
  return admitted;
}
export function networkInventoryExpired(value: NetworkInventory, now = Date.now() / 1000): boolean {
  const seconds = Math.floor(now);
  return !Number.isFinite(now) || seconds < 0 || seconds > MAX_TIME || value.source !== "fresh" || value.sampled_at === null || seconds < value.generated_at || seconds < value.sampled_at || seconds - value.sampled_at > value.max_age_seconds;
}
async function body(response: Response, signal: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader() ?? invalid(); const buffer = new Uint8Array(MAX_BYTES); let bytes = 0;
  const cancel = () => { void reader.cancel().catch(() => {}); };
  const checkAbort = () => { if (signal.aborted) throw new NetworkReadError("The network inventory read was cancelled."); };
  signal.addEventListener("abort", cancel, { once: true }); if (signal.aborted) cancel();
  try {
    checkAbort();
    for (;;) {
      const chunk = await reader.read(); checkAbort(); if (chunk.done) break;
      if (chunk.value.length > buffer.length - bytes) { void reader.cancel().catch(() => {}); invalid(); }
      buffer.set(chunk.value, bytes); bytes += chunk.value.length;
    }
  } finally { signal.removeEventListener("abort", cancel); reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, bytes))) as unknown; }
  catch { return invalid(); }
}
export async function readNetworkInventory(signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<NetworkInventory> {
  const controller = new AbortController(); const abort = () => controller.abort();
  signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
  const timer = setTimeout(abort, 12_000);
  try {
    if (controller.signal.aborted) throw new NetworkReadError("The network inventory read was cancelled.");
    const response = await fetcher("/api/v1/deployment/networks", { method: "GET", credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" }, signal: controller.signal });
    if (response.status === 401 || response.status === 403) {
      void response.body?.cancel().catch(() => {});
      throw new NetworkReadError(response.status === 401 ? "Your session has expired." : "You no longer have access to network groups.", response.status);
    }
    if (!response.ok) { void response.body?.cancel().catch(() => {}); throw new NetworkReadError("Network groups are unavailable. Refresh to try again.", response.status); }
    return decodeNetworkInventory(await body(response, controller.signal));
  } finally { clearTimeout(timer); signal.removeEventListener("abort", abort); }
}
