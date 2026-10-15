import type { ActivitySnapshot, Provider } from "./media-activity-model";

export class ActivityReadError extends Error {
  constructor(message: string, readonly status = 0) { super(message); }
}
const invalid = (): never => { throw new ActivityReadError("The host returned invalid media activity."); };
const check = (ok: boolean): void => { if (!ok) invalid(); };
function record(value: unknown, keys: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return invalid();
  const row = value as Record<string, unknown>; const expected = keys.split(" ");
  check(Object.keys(row).length === expected.length && expected.every(key => Object.hasOwn(row, key)));
  return row;
}
const list = (v: unknown, max: number): unknown[] => Array.isArray(v) && v.length <= max ? v : invalid();
const integer = (v: unknown, max = 253402300799): number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0 && v <= max ? v : invalid();
const metric = (v: unknown, max = Number.MAX_SAFE_INTEGER): number | null => v === null ? null : typeof v === "number" && Number.isFinite(v) && v >= 0 && v <= max ? v : invalid();
const optionalInteger = (v: unknown, max = 253402300799): number | null => v === null ? null : integer(v, max);
const bool = (v: unknown): boolean => typeof v === "boolean" ? v : invalid();
const choice = (v: unknown, options: readonly string[]): string => typeof v === "string" && options.includes(v) ? v : invalid();
function text(v: unknown, max: number, nonempty = false): string {
  if (typeof v !== "string" || (nonempty && !v.length) || /[\u0000-\u001f\u007f-\u009f]/u.test(v)) return invalid();
  for (const char of v) { const cp = char.codePointAt(0)!; if (cp >= 0xd800 && cp <= 0xdfff) invalid(); }
  check(new TextEncoder().encode(v).length <= max); return v;
}
function timestamp(value: unknown): void {
  const v = text(value, 30); const parts = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/.exec(v) ?? invalid();
  const [year, month, day, hour, minute, second] = parts.slice(1, 7).map(Number);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  check(year > 0 && month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1] && hour <= 23 && minute <= 59 && second <= 59);
}
const providers = ["jellyfin", "emby", "sabnzbd", "transmission", "sonarr", "radarr", "lidarr", "readarr"];
const states = ["ready", "partial", "stopped", "stale", "future", "identity_mismatch", "unavailable", "credential_unavailable", "timeout", "invalid_response", "unsupported"];
const family = (p: Provider): number => providers.indexOf(p) < 2 ? 0 : providers.indexOf(p) < 4 ? 1 : 2;
const resource = (v: unknown): string => /^container:[0-9a-f]{64}$/.test(text(v, 74)) ? v as string : invalid();

/** Admission checks display integrity, not collector ownership or effect authority. */
export function decodeMediaActivity(value: unknown): ActivitySnapshot {
  const r = record(value, "version generated_at sampled_at max_age_seconds source complete services streams downloads queues omitted_streams omitted_downloads totals");
  check(r.version === 1 && r.max_age_seconds === 8);
  const now = integer(r.generated_at); const sampled = optionalInteger(r.sampled_at);
  const source = choice(r.source, ["fresh", "stale", "future", "incomplete", "unavailable", "identity_mismatch"]);
  const complete = bool(r.complete); const active = source === "fresh" || source === "incomplete";
  check((source === "unavailable") === (sampled === null));
  if (active) check(sampled! <= now && now - sampled! <= 8);
  if (source === "future") check(sampled! > now);
  if (source === "stale") check(now - sampled! > 8);
  const services = new Map<string, { family: number; state: string }>();
  for (const value of list(r.services, 8)) {
    const s = record(value, "target sampled_at state"); const t = record(s.target, "container name provider");
    const c = record(t.container, "resource image started_at running"); const id = resource(c.resource);
    check(!services.has(id)); check(/^sha256:[0-9a-f]{64}$/.test(text(c.image, 71)));
    timestamp(c.started_at); const running = bool(c.running); text(t.name, 128, true);
    const f = family(choice(t.provider, providers) as Provider); const state = choice(s.state, states);
    const time = optionalInteger(s.sampled_at);
    check((state === "stopped") === !running);
    if (!["stopped", "unavailable"].includes(state)) check(time !== null);
    if (["ready", "partial", "credential_unavailable", "timeout", "invalid_response", "unsupported"].includes(state)) check(time! >= sampled! && time! <= now && now - time! <= 8);
    if (state === "stale") check(time! <= now && (time! < sampled! || now - time! > 8));
    if (state === "future") check(time! > now);
    if (state === "partial") check(f !== 0);
    services.set(id, { family: f, state });
  }
  check(complete === (source === "fresh" && [...services.values()].every(s => s.state === "ready")));
  const available = (id: unknown, f: number) => {
    const s = services.get(resource(id)) ?? invalid();
    check(s.family === f && ["ready", "partial"].includes(s.state)); return s;
  };
  const streamRows = list(r.streams, 12); const downloadRows = list(r.downloads, 12); const queueRows = list(r.queues, 8);
  const perSource = new Map<string, number>();
  for (const value of streamRows) {
    const row = record(value, "source data"); available(row.source, 0);
    const d = record(row.data, "title subtitle user client device play_method paused transcoding transcode_reason bitrate progress_percent position_seconds duration_seconds");
    for (const key of ["title", "subtitle", "user", "client", "device"]) text(d[key], 200);
    if (d.play_method !== null) text(d.play_method, 64);
    if (d.transcode_reason !== null) text(d.transcode_reason, 512);
    bool(d.paused); bool(d.transcoding); metric(d.progress_percent, 100);
    for (const key of ["bitrate", "position_seconds", "duration_seconds"]) metric(d[key]);
    perSource.set(row.source as string, (perSource.get(row.source as string) ?? 0) + 1);
  }
  for (const value of downloadRows) {
    const row = record(value, "source data"); available(row.source, 1);
    const d = record(row.data, "name state progress_percent size_bytes remaining_bytes speed_bytes eta");
    text(d.name, 200); text(d.state, 64); if (d.eta !== null) text(d.eta, 64);
    metric(d.progress_percent, 100); for (const key of ["size_bytes", "remaining_bytes", "speed_bytes"]) metric(d[key]);
    perSource.set(row.source as string, (perSource.get(row.source as string) ?? 0) + 1);
  }
  const queueIds = new Set<string>(); let queued = 0; let problems = 0;
  for (const value of queueRows) {
    const row = record(value, "source data"); const s = available(row.source, 2); const id = row.source as string;
    check(!queueIds.has(id)); queueIds.add(id);
    const d = record(row.data, "total observed problems complete items");
    const total = integer(d.total, 4294967295); const observed = integer(d.observed, 24); const count = integer(d.problems, 24);
    check(count <= observed && observed <= total);
    check(bool(d.complete) === (s.state === "ready")); if (d.complete) check(total === observed);
    const rows = list(d.items, 3); check(rows.length <= observed);
    for (const value of rows) { const item = record(value, "title state status"); text(item.title, 200); text(item.state, 64); text(item.status, 64); }
    queued += total; problems += count;
  }
  for (const [id, s] of services) if (s.family === 2 && ["ready", "partial"].includes(s.state)) check(queueIds.has(id));
  const capacities = [0, 0];
  for (const [id, s] of services) {
    if (s.family < 2 && ["ready", "partial"].includes(s.state)) capacities[s.family] += 24;
    check((perSource.get(id) ?? 0) <= 24);
  }
  const omittedStreams = integer(r.omitted_streams, 192); const omittedDownloads = integer(r.omitted_downloads, 192);
  check((omittedStreams === 0 || streamRows.length === 12) && streamRows.length + omittedStreams <= capacities[0]);
  check((omittedDownloads === 0 || downloadRows.length === 12) && downloadRows.length + omittedDownloads <= capacities[1]);
  const totals = record(r.totals, "streams transcodes downloads download_speed queued queue_problems");
  const streams = optionalInteger(totals.streams, 192); const transcodes = optionalInteger(totals.transcodes, 192);
  const downloads = optionalInteger(totals.downloads, 192); const speed = metric(totals.download_speed);
  const queueTotal = optionalInteger(totals.queued, 4294967295); const queueProblems = optionalInteger(totals.queue_problems, 192);
  if (!complete) check(Object.values(totals).every(v => v === null));
  else {
    check(streams === streamRows.length + omittedStreams && downloads === downloadRows.length + omittedDownloads);
    const visibleTranscodes = streamRows.filter(row => ((row as Record<string, unknown>).data as Record<string, unknown>).transcoding).length;
    check(transcodes !== null && transcodes >= visibleTranscodes && transcodes <= visibleTranscodes + omittedStreams);
    check(queueTotal === (queued > 4294967295 ? null : queued) && queueProblems === problems);
    if (capacities[1] === 0) check(speed === 0);
  }
  if (!active) check(!complete && services.size === 0 && !streamRows.length && !downloadRows.length && !queueRows.length && !omittedStreams && !omittedDownloads);
  return value as ActivitySnapshot;
}

async function body(response: Response, signal: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader() ?? invalid(); const buffer = new Uint8Array(65_536); let bytes = 0;
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener("abort", cancel, { once: true }); if (signal.aborted) cancel();
  try {
    for (;;) {
      const item = await reader.read(); if (signal.aborted) throw new ActivityReadError("Media activity read was interrupted.");
      if (item.done) break;
      if (item.value.length > buffer.length - bytes) { cancel(); invalid(); }
      buffer.set(item.value, bytes); bytes += item.value.length;
    }
  } finally { signal.removeEventListener("abort", cancel); reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, bytes))) as unknown; }
  catch { return invalid(); }
}

/** Fixed authenticated cache read. No provider URL, credential or caller body. */
export async function readMediaActivity(signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<ActivitySnapshot> {
  const controller = new AbortController(); const abort = () => controller.abort();
  signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; abort(); }, 12_000);
  let cancelRead = () => {};
  const cancelled = new Promise<never>((_, reject) => {
    cancelRead = () => reject(new ActivityReadError(timedOut ? "Media activity read timed out. Refresh to try again." : "Media activity read was interrupted."));
    controller.signal.addEventListener("abort", cancelRead, { once: true });
    if (controller.signal.aborted) cancelRead();
  });
  const read = async () => {
    if (controller.signal.aborted) return await cancelled;
    const response = await fetcher("/api/v1/deployment/activity", { method: "GET", credentials: "same-origin", cache: "no-store", redirect: "error", headers: { Accept: "application/json" }, signal: controller.signal });
    if (controller.signal.aborted || !response.ok) {
      void response.body?.cancel().catch(() => {});
      if (controller.signal.aborted) return await cancelled;
      throw new ActivityReadError(response.status === 401 ? "Your session has expired." : response.status === 403 ? "You no longer have access to media activity." : "Media activity is unavailable. Refresh to try again.", response.status);
    }
    return decodeMediaActivity(await body(response, controller.signal));
  };
  try { return await Promise.race([read(), cancelled]); }
  catch (error) {
    if (error instanceof ActivityReadError) throw error;
    throw new ActivityReadError("Media activity is unavailable. Refresh to try again.");
  } finally {
    clearTimeout(timer); signal.removeEventListener("abort", abort); controller.signal.removeEventListener("abort", cancelRead);
  }
}
