/**
 * What the Stacks page shows, derived from the stack inventory and the shared
 * overview. Pure and dependency-free so node tests import it directly.
 * Rules:
 * - a stack is "Running", "Partly running" or "Stopped" only from a current
 *   inventory (both sources fresh, inside their lifetime, same Engine);
 *   otherwise it is "Unknown" and the reason says what is unknown;
 * - an incomplete inventory never reads as a clean "Running";
 * - containers are named from the overview where it knows them, else by
 *   their short ID; a name is never invented.
 */
import type {
  Freshness, Resource, StackInventory, StackJob, StackOmissionReason, StackReceipt, StackServiceView, StackView,
} from "../../../contracts/generated/types";

export type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

export type StackStateKey = "running" | "partial" | "stopped" | "empty" | "unknown" | "conflict";
export interface StackStatus {
  key: StackStateKey;
  label: string;
  tone: Tone;
  /** What is unknown or worth knowing about this status, in plain words. */
  reason: string | null;
}

/** The first 12 hex digits of a `container:<64 hex>` resource. */
export const shortId = (resource: string): string => resource.replace(/^container:/, "").slice(0, 12);

/**
 * Why the inventory's runtime facts cannot be taken as current, or null when
 * they can. `stale` is the client's own lifetime check (inventoryStale).
 */
export function inventoryUnknown(value: StackInventory, stale: boolean): string | null {
  const { catalog, engine } = value;
  if (catalog.state === "unavailable" && engine.state === "unavailable") return "The stack reader isn't answering, so nothing current is known about your stacks.";
  if (engine.state === "unavailable") return "Docker can't be read right now, so which containers are running is unknown.";
  if (catalog.state === "unavailable") return "The stack catalog can't be read right now, so which stacks are registered is unknown.";
  if (engine.state !== "fresh") return "The last Docker reading is out of date, so which containers are running is unknown.";
  if (catalog.state !== "fresh") return "The last stack catalog reading is out of date.";
  if (value.engine_matches_catalog === false) return "These stacks were registered against a different Docker Engine, so their containers can't be matched.";
  if (stale) return "This reading is more than 90 seconds old, so it may no longer be current.";
  return null;
}

/** Both sources unavailable: the reader has not delivered anything since it (or the host) started. */
export function readerDown(value: StackInventory): boolean {
  return value.catalog.state === "unavailable" && value.engine.state === "unavailable";
}

const incomplete = (stack: StackView) => stack.issues.includes("omitted_members") || stack.issues.includes("incomplete_inventory");

export function stackStatus(stack: StackView, unknown: string | null): StackStatus {
  if (unknown) return { key: "unknown", label: "Unknown", tone: "neutral", reason: unknown };
  if (stack.runtime === "conflict" || stack.issues.includes("identity_conflict") || stack.issues.includes("overlapping_project")) {
    return { key: "conflict", label: "Identity conflict", tone: "danger", reason: "Docker's view of this project doesn't match its registration, so its containers can't be counted." };
  }
  if (stack.runtime === "unknown" || !stack.counts) return { key: "unknown", label: "Unknown", tone: "neutral", reason: "Docker didn't report this stack's containers." };
  const { running, ordinary } = stack.counts;
  const partialRead = incomplete(stack) ? "Some of its containers couldn't be read, so this may be incomplete." : null;
  if (ordinary === 0) return { key: "empty", label: "No service containers", tone: "neutral", reason: partialRead ?? "None of its services has a container yet." };
  if (running === ordinary) return { key: "running", label: "Running", tone: partialRead ? "warning" : "success", reason: partialRead };
  if (running === 0) return { key: "stopped", label: "Stopped", tone: "neutral", reason: partialRead };
  return { key: "partial", label: "Partly running", tone: "warning", reason: partialRead };
}

/** "1 of 2 service containers running", or the unknown sentence the tests rely on. */
export function countsSentence(counts: StackView["counts"], unknown: boolean): string {
  return counts && !unknown ? `${counts.running} of ${counts.ordinary} service containers running` : "Current container counts are unknown.";
}

/** Bar fill (0–100) for running of total, or null when unknown or empty. */
export function runningPercent(counts: StackView["counts"], unknown: boolean): number | null {
  if (!counts || unknown || counts.ordinary === 0) return null;
  return (counts.running / counts.ordinary) * 100;
}

// ---- Member names --------------------------------------------------------

/** Containers in the overview, by resource id. */
export function containerIndex(resources: readonly Resource[] | null | undefined): Map<string, Resource> {
  const index = new Map<string, Resource>();
  for (const r of resources ?? []) if (r.kind === "container") index.set(r.id, r);
  return index;
}

export interface MemberRow {
  resource: string;
  shortId: string;
  /** The container name from the overview, or null when it isn't known. */
  name: string | null;
  /** What to show: the name, else the short ID. */
  label: string;
  service: string;
  oneOff: boolean;
  /** Running according to the stack inventory. */
  running: boolean;
  /** "running", "exited", "restarting"…, or "not running" / "last seen running". */
  state: string;
  health: string | null;
  tone: Tone;
  image: string;
  startedAt: string;
}
export interface ServiceRow {
  name: string;
  declared: boolean;
  members: MemberRow[];
  running: number | null;
  total: number | null;
}

/** A container's tone: unhealthy, restarting or dead is danger; running is success. */
export function memberTone(status: string, health: string | null, running: boolean): Tone {
  if (health === "unhealthy" || status === "restarting" || status === "dead") return "danger";
  if (health === "starting") return "info";
  if (status === "running" || (status === "" && running)) return "success";
  return "neutral";
}

/**
 * Inventory members joined to overview containers. `live` says the overview's
 * docker source is current: only then are its status and health shown. With
 * `previous`, the inventory's own state is a past observation.
 */
export function serviceRows(services: readonly StackServiceView[], index: Map<string, Resource>, { live, previous }: { live: boolean; previous: boolean }): ServiceRow[] {
  return services.map((service) => ({
    name: service.name,
    declared: service.declared,
    running: service.counts && !previous ? service.counts.running : null,
    total: service.counts && !previous ? service.counts.ordinary : null,
    members: service.members.map((member) => {
      const resource = member.container.resource;
      const seen = index.get(resource);
      const current = live && seen !== undefined;
      const state = current ? seen.status : previous ? (member.container.running ? "last seen running" : "last seen stopped") : member.container.running ? "running" : "not running";
      const health = current ? seen.health : null;
      return {
        resource,
        shortId: shortId(resource),
        name: seen?.name ?? null,
        label: seen?.name ?? shortId(resource),
        service: service.name,
        oneOff: member.one_off,
        running: member.container.running,
        state,
        health,
        tone: previous && !current ? "neutral" : memberTone(current ? seen.status : "", health, member.container.running),
        image: member.container.image,
        startedAt: member.container.started_at,
      };
    }),
  }));
}

/** Members whose container reports a failing health check or is restarting. */
export function unhealthyCount(rows: readonly ServiceRow[]): number {
  return rows.reduce((sum, s) => sum + s.members.filter((m) => m.tone === "danger").length, 0);
}

export interface OmittedRow {
  resource: string;
  shortId: string;
  name: string | null;
  label: string;
  project: string | null;
  reason: StackOmissionReason;
}
export const omissionWords: Record<StackOmissionReason, string> = { changing: "changing state", unsupported: "not readable" };

/** Containers the inventory left out, named where the overview knows them. */
export function omittedRows(value: StackInventory, index: Map<string, Resource>): OmittedRow[] {
  return value.omitted_containers.map((o) => {
    const name = index.get(o.resource)?.name ?? null;
    return { resource: o.resource, shortId: shortId(o.resource), name, label: name ?? shortId(o.resource), project: o.project, reason: o.reason };
  });
}

// ---- Summary -------------------------------------------------------------

export interface StackSummary {
  stacks: number;
  running: number;
  partial: number;
  stopped: number;
  unknown: number;
  /** Service containers running of total, or null when any stack is unknown. */
  services: { running: number; total: number } | null;
}

export function stackSummary(value: StackInventory, unknown: string | null): StackSummary {
  const statuses = value.stacks.map((s) => stackStatus(s, unknown));
  const known = value.stacks.every((s, i) => s.counts && statuses[i].key !== "unknown" && statuses[i].key !== "conflict");
  return {
    stacks: value.stacks.length,
    running: statuses.filter((s) => s.key === "running").length,
    partial: statuses.filter((s) => s.key === "partial").length,
    stopped: statuses.filter((s) => s.key === "stopped" || s.key === "empty").length,
    unknown: statuses.filter((s) => s.key === "unknown" || s.key === "conflict").length,
    services: known
      ? value.stacks.reduce((t, s) => ({ running: t.running + s.counts!.running, total: t.total + s.counts!.ordinary }), { running: 0, total: 0 })
      : null,
  };
}

/** "Current" / "out of date" / "unavailable" for a source, as people say it. */
export function sourceWord(source: StackInventory["catalog"]): string {
  return source.state === "fresh" ? "current" : source.state === "stale" ? "out of date" : "unavailable";
}

// ---- The host observer's view (when the inventory is not available) -------

export interface ObservedStack {
  id: string;
  name: string;
  containers: Resource[];
  running: number;
  total: number;
  unhealthy: number;
  status: StackStatus;
}

/**
 * Compose projects as the host observer groups them (by the Compose label).
 * Read-only: no registration, no actions. `fresh` is the docker source.
 */
export function observedStacks(resources: readonly Resource[] | null | undefined, fresh: boolean): ObservedStack[] {
  const all = resources ?? [];
  return all.filter((r) => r.kind === "stack").map((stack) => {
    const containers = all.filter((r) => r.kind === "container" && r.parent === stack.id).sort((a, b) => a.name.localeCompare(b.name));
    const running = containers.filter((c) => c.status === "running").length;
    const unhealthy = containers.filter((c) => memberTone(c.status, c.health, false) === "danger").length;
    const status: StackStatus = !fresh
      ? { key: "unknown", label: "Unknown", tone: "neutral", reason: "The host observer's Docker reading is out of date." }
      : containers.length === 0
        ? { key: "empty", label: "No containers", tone: "neutral", reason: null }
        : running === containers.length
          ? { key: "running", label: "Running", tone: "success", reason: null }
          : running === 0
            ? { key: "stopped", label: "Stopped", tone: "neutral", reason: null }
            : { key: "partial", label: "Partly running", tone: "warning", reason: null };
    return { id: stack.id, name: stack.name, containers, running, total: containers.length, unhealthy, status };
  }).sort((a, b) => a.name.localeCompare(b.name));
}

/** The overview's docker source is present and fresh at `now`. */
export function dockerFresh(sources: readonly Freshness[] | null | undefined, now: number): boolean {
  const s = (sources ?? []).find((x) => x.source === "docker");
  return !!s && s.state === "fresh" && s.sampled_at !== null && now - s.sampled_at <= s.max_age_seconds;
}

// ---- Jobs on cards ---------------------------------------------------------

export const activeJobStates: readonly string[] = ["queued", "running", "verifying"];
/** How long a finished job stays on its stack's card. */
export const RECENT_JOB_SECONDS = 15 * 60;

/**
 * The job to show on a stack's card: one in flight or needing attention,
 * else the newest finished in the last 15 minutes. Jobs arrive newest first.
 */
export function jobForStack(jobs: readonly StackJob[], stack: Pick<StackView, "stack" | "project">, now: number): StackJob | null {
  const mine = jobs.filter((j) => j.plan.catalog_stack === stack.stack || j.plan.expected.resource === `stack:${stack.project}`);
  const open = mine.find((j) => activeJobStates.includes(j.state) || j.state === "needs_intervention");
  if (open) return open;
  const latest = mine[0];
  return latest && now - latest.plan.created_at <= RECENT_JOB_SECONDS ? latest : null;
}

export interface OutcomeRow { resource: string; label: string; shortId: string; outcome: string; tone: Tone }
const outcomeWords: Record<string, [string, Tone]> = {
  prepared: ["prepared", "info"],
  effect_accepted: ["accepted, checking", "info"],
  verified: ["done", "success"],
  precondition_changed: ["not attempted", "neutral"],
  outcome_unknown: ["outcome unknown", "danger"],
  partly_done: ["partly done", "warning"],
};

/**
 * One row per container the job acts on: start acts on the stopped members,
 * stop and restart on the running ones. With a receipt, each row carries the
 * executor's recorded outcome; without one, "waiting".
 */
export function jobOutcomes(job: StackJob, receipt: StackReceipt | null | undefined, index: Map<string, Resource>): OutcomeRow[] {
  const recorded = new Map((receipt?.members ?? []).map((m) => [m.resource, m]));
  const targets = job.plan.expected.members.filter((m) => m.running !== (job.plan.operation === "start")).map((m) => m.resource);
  const resources = [...targets, ...[...recorded.keys()].filter((r) => !targets.includes(r))];
  return resources.map((resource) => {
    const m = recorded.get(resource);
    const [outcome, tone] = m ? outcomeWords[m.state] ?? [m.state.replaceAll("_", " "), "neutral"] : activeJobStates.includes(job.state) ? ["waiting", "neutral" as Tone] : ["no record", "neutral" as Tone];
    const name = index.get(resource)?.name;
    return { resource, label: name ?? shortId(resource), shortId: shortId(resource), outcome: m?.error ? `${outcome} (${m.error.replaceAll("_", " ")})` : outcome, tone };
  });
}

const operationWords: Record<string, [string, string]> = { start: ["Starting", "Started"], stop: ["Stopping", "Stopped"], restart: ["Restarting", "Restarted"] };
/** "Restarting stack · running", "Restarted stack · done", in words people use. */
export function jobHeadline(job: StackJob, receipt?: StackReceipt | null): { text: string; tone: Tone } {
  const [doing, done] = operationWords[job.plan.operation] ?? ["Changing", "Changed"];
  if (job.state === "failed" && receipt?.state === "partly_done") {
    const verified = receipt.members.filter((m) => m.state === "verified").length;
    return { text: `${doing} stack · partly done, ${verified} of ${receipt.members.length}`, tone: "warning" };
  }
  switch (job.state) {
    case "queued": return { text: `${doing} stack · waiting to start`, tone: "info" };
    case "running": case "verifying": return { text: `${doing} stack · in progress`, tone: "info" };
    case "succeeded": return { text: `${done} stack`, tone: "success" };
    case "canceled": return { text: `${doing} stack · canceled`, tone: "neutral" };
    case "needs_intervention": return { text: `${doing} stack · needs an administrator`, tone: "danger" };
    case "outcome_unknown": return { text: `${doing} stack · outcome unknown`, tone: "danger" };
    case "precondition_changed": return { text: `${doing} stack · not done, the stack changed first`, tone: "warning" };
    case "failed": return { text: `${doing} stack · failed`, tone: "danger" };
    default: return { text: `${doing} stack · ${job.state.replaceAll("_", " ")}`, tone: "neutral" };
  }
}
