/** Proposed B2 wire types; shared contract registration/generation is central. */
export type RuntimeStatus = "unknown" | "no_containers" | "stopped" | "running" | "partial" | "conflict";
export type SourceStatus = { state: "fresh" | "stale" | "unavailable"; sampled_at: number | null; max_age_seconds: number };
export type Counts = { ordinary: number; running: number; one_offs: number; one_off_running: number };
export type MemberView = { container: { resource: string; image: string; started_at: string; running: boolean }; one_off: boolean };
export type ServiceView = { name: string; declared: boolean; runtime: RuntimeStatus; counts: Counts | null; members: MemberView[] };
export type FileStatus = { state: "not_inspected" | "unavailable" } |
  { state: "conflict"; candidates: string[] } | { state: "unsupported"; reason: "yaml" | "environment" | "include" | "interpolation" | "other" };
export type StackView = {
  stack: string; project: string; working_directory: string; config_files: string[]; files: FileStatus;
  runtime: RuntimeStatus; counts: Counts | null; missing_services: number | null;
  services: ServiceView[]; issues: ("identity_conflict" | "overlapping_project" | "undeclared_services")[];
};
export type OtherProject = {
  project: { name: string; working_directory: string; config_files: string[] };
  relation: "not_registered" | "catalog_unknown" | "engine_mismatch" | "identity_conflict" | "overlapping";
  services: ServiceView[];
};
export type StackInventory = {
  version: 1; generated_at: number; catalog: SourceStatus; engine: SourceStatus;
  engine_matches_catalog: boolean | null; stacks: StackView[]; other_projects: OtherProject[];
  standalone_containers: number | null;
};

export class StackReadError extends Error {
  constructor(message: string, readonly status = 0, readonly code = "unavailable", readonly auditId = "") { super(message); }
}
const invalid = (): never => { throw new StackReadError("The host returned an invalid stack inventory."); };
const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : invalid();
const list = (v: unknown, max: number): unknown[] => Array.isArray(v) && v.length <= max ? v : invalid();
const integer = (v: unknown, max = 253402300799): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : invalid();
const text = (v: unknown, max: number): string => typeof v === "string" && v.length > 0 && new TextEncoder().encode(v).length <= max && !/[\u0000-\u001f\u007f]/u.test(v) ? v : invalid();
const bool = (v: unknown): boolean => typeof v === "boolean" ? v : invalid();
const choice = (v: unknown, choices: readonly string[]): void => { if (typeof v !== "string" || !choices.includes(v)) invalid(); };
const nullableInteger = (v: unknown, max: number): void => { if (v !== null) integer(v, max); };
const runtimes = ["unknown", "no_containers", "stopped", "running", "partial", "conflict"];
function source(v: unknown): void {
  const r = record(v); choice(r.state, ["fresh", "stale", "unavailable"]);
  nullableInteger(r.sampled_at, 253402300799);
  if ((r.state === "unavailable") !== (r.sampled_at === null) || r.max_age_seconds !== 90) invalid();
}
function counts(v: unknown): void {
  const r = record(v);
  for (const field of ["ordinary", "running", "one_offs", "one_off_running"]) integer(r[field], 16);
  if ((r.running as number) > (r.ordinary as number) || (r.one_off_running as number) > (r.one_offs as number)) invalid();
}
function project(v: unknown): void {
  const r = record(v); text(r.name, 64); text(r.working_directory, 512);
  const files = list(r.config_files, 4); if (files.length === 0) invalid();
  for (const f of files) text(f, 512);
}
export function decodeStackInventory(value: unknown): StackInventory {
  const r = record(value); if (r.version !== 1) invalid(); integer(r.generated_at);
  source(r.catalog); source(r.engine);
  if (r.engine_matches_catalog !== null) bool(r.engine_matches_catalog);
  nullableInteger(r.standalone_containers, 16);
  let members = 0; const ids = new Set<string>();
  const services = (value: unknown, max: number) => {
    const names = new Set<string>();
    for (const item of list(value, max)) {
      const s = record(item); const name = text(s.name, 64); if (names.has(name)) invalid(); names.add(name);
      bool(s.declared); choice(s.runtime, runtimes);
      if (s.counts !== null) counts(s.counts);
      if ((s.runtime === "unknown" || s.runtime === "conflict") !== (s.counts === null)) invalid();
      const rows = list(s.members, 16);
      if (s.counts === null && rows.length !== 0) invalid();
      for (const value of rows) {
        const m = record(value); bool(m.one_off); const c = record(m.container);
        const id = text(c.resource, 74); if (!/^container:[0-9a-f]{64}$/.test(id) || ids.has(id)) invalid();
        ids.add(id); members++; if (members > 16) invalid();
        if (!/^sha256:[0-9a-f]{64}$/.test(text(c.image, 71))) invalid();
        text(c.started_at, 64); bool(c.running);
      }
    }
  };
  const stacks = new Set<string>();
  for (const value of list(r.stacks, 8)) {
    const s = record(value); const id = text(s.stack, 64); if (stacks.has(id)) invalid(); stacks.add(id);
    project({ name: s.project, working_directory: s.working_directory, config_files: s.config_files });
    choice(s.runtime, runtimes); if (s.counts !== null) counts(s.counts);
    if ((s.runtime === "unknown" || s.runtime === "conflict") !== (s.counts === null)) invalid();
    nullableInteger(s.missing_services, 16); if ((s.counts === null) !== (s.missing_services === null)) invalid();
    const files = record(s.files); choice(files.state, ["not_inspected", "conflict", "unsupported", "unavailable"]);
    if (files.state === "conflict") {
      const names = list(files.candidates, 4); if (names.length < 2 || new Set(names).size !== names.length) invalid();
      for (const name of names) choice(name, ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"]);
    }
    if (files.state === "unsupported") choice(files.reason, ["yaml", "environment", "include", "interpolation", "other"]);
    for (const issue of list(s.issues, 3)) choice(issue, ["identity_conflict", "overlapping_project", "undeclared_services"]);
    services(s.services, 32);
  }
  const projects = new Set<string>();
  for (const value of list(r.other_projects, 16)) {
    const p = record(value); project(p.project); const name = text(record(p.project).name, 64);
    if (projects.has(name)) invalid(); projects.add(name);
    choice(p.relation, ["not_registered", "catalog_unknown", "engine_mismatch", "identity_conflict", "overlapping"]);
    services(p.services, 16);
  }
  return value as StackInventory;
}

export function inventoryStale(value: StackInventory, now = Date.now() / 1000): boolean {
  return [value.catalog, value.engine].some(s => s.state !== "fresh" || s.sampled_at === null || now < s.sampled_at || now - s.sampled_at > s.max_age_seconds);
}

async function body(response: Response): Promise<unknown> {
  const reader = response.body?.getReader() ?? invalid();
  const buffer = new Uint8Array(65_536); let bytes = 0;
  try {
    for (;;) {
      const item = await reader.read(); if (item.done) break;
      if (item.value.length > buffer.length - bytes) { void reader.cancel().catch(() => {}); invalid(); }
      buffer.set(item.value, bytes); bytes += item.value.length;
    }
  } finally { reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, bytes))) as unknown; }
  catch { return invalid(); }
}

export async function readStackInventory(signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<StackInventory> {
  const controller = new AbortController(); const abort = () => controller.abort();
  signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
  const timer = setTimeout(abort, 12_000);
  try {
    const response = await fetcher("/api/v1/deployment/stacks", { method: "GET", credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" }, signal: controller.signal });
    if (response.status === 401 || response.status === 403) {
      void response.body?.cancel().catch(() => {});
      throw new StackReadError(response.status === 401 ? "Your session has expired." : "You no longer have access to stack inventory.", response.status);
    }
    if (!response.ok) {
      const value = await body(response).catch(() => null); const error = value && typeof value === "object" ? value as Record<string, unknown> : null;
      throw new StackReadError(typeof error?.message === "string" && error.message.length <= 512 ? error.message : "Stack inventory is unavailable. Refresh to try again.", response.status,
        typeof error?.code === "string" && error.code.length <= 64 ? error.code : "unavailable", typeof error?.audit_id === "string" && error.audit_id.length <= 128 ? error.audit_id : "");
    }
    return decodeStackInventory(await body(response));
  } finally { clearTimeout(timer); signal.removeEventListener("abort", abort); }
}
