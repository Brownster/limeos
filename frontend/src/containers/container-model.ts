/**
 * The Containers page's facts, derived from the shared overview and the
 * principal's own container jobs. Pure (type imports only) so node tests can
 * import it directly. Rules:
 * - "needs attention" is defined once here (see `attentionReason`);
 * - a value the host did not report, or reported through a stale source, is
 *   unknown, never zero;
 * - the URL query reproduces a view: only known values are accepted.
 */
import type { ContainerAction, Freshness, Resource, RestartJob } from "../../../contracts/generated/types";

export type Filter = "all" | "running" | "stopped" | "attention";
export type Sort = "name" | "cpu" | "memory";

export const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "running", label: "Running" },
  { id: "stopped", label: "Stopped" },
  { id: "attention", label: "Needs attention" },
];
export const SORTS: { id: Sort; label: string }[] = [
  { id: "name", label: "Name" },
  { id: "cpu", label: "CPU use" },
  { id: "memory", label: "Memory use" },
];

/** Containers drawn at most. The overview carries at most 64 (the Engine reader's bound). */
export const MAX_ROWS = 200;
export const SEARCH_MAX = 100;

export interface ViewState {
  filter: Filter;
  query: string;
  group: boolean;
  sort: Sort;
  /** The container whose detail is open, by name. */
  detail: string | null;
}
export const DEFAULT_VIEW: ViewState = { filter: "all", query: "", group: false, sort: "name", detail: null };

// ---------------------------------------------------------------- URL state

const filterIds = new Set<string>(FILTERS.map((f) => f.id));
const sortIds = new Set<string>(SORTS.map((s) => s.id));

/** Reads `?filter=&q=&group=stack&sort=&container=`; unknown values fall back to the defaults. */
export function parseView(search: string): ViewState {
  const params = new URLSearchParams(search);
  const filter = params.get("filter") ?? "";
  const sort = params.get("sort") ?? "";
  const query = (params.get("q") ?? "").slice(0, SEARCH_MAX);
  const detail = params.get("container");
  return {
    filter: filterIds.has(filter) ? (filter as Filter) : "all",
    query,
    group: params.get("group") === "stack",
    sort: sortIds.has(sort) ? (sort as Sort) : "name",
    detail: detail && detail.length <= 128 ? detail : null,
  };
}

/** The query string for a view ("" for the default view), in a stable order. */
export function serializeView(view: ViewState): string {
  const params = new URLSearchParams();
  if (view.filter !== "all") params.set("filter", view.filter);
  if (view.query.trim()) params.set("q", view.query.slice(0, SEARCH_MAX));
  if (view.group) params.set("group", "stack");
  if (view.sort !== "name") params.set("sort", view.sort);
  if (view.detail) params.set("container", view.detail);
  const text = params.toString();
  return text ? `?${text}` : "";
}

// ---------------------------------------------------------------- docker source

export type DockerState = "fresh" | "stale" | "unavailable" | "loading" | "missing";
export interface DockerSource {
  state: DockerState;
  sampledAt: number | null;
  warnings: string[];
}

/**
 * The docker source's state. "stale" uses the same rule as the Overview
 * (`sourceStale` in overview-model): reported fresh but older than its own
 * freshness limit counts as stale. A source that is not listed is "missing".
 */
export function dockerSource(sources: Freshness[], now: number): DockerSource {
  const source = sources.find((s) => s.source === "docker");
  if (!source) return { state: "missing", sampledAt: null, warnings: [] };
  const old = source.sampled_at === null || now - source.sampled_at > source.max_age_seconds;
  const state: DockerState = source.state === "fresh" ? (old ? "stale" : "fresh") : source.state;
  return { state, sampledAt: source.sampled_at, warnings: source.warnings };
}

/** Whether per-container readings (CPU, memory) can be shown as current. */
export const readingsCurrent = (docker: DockerSource) => docker.state === "fresh";

// ---------------------------------------------------------------- images

export interface ImageRef {
  /** As Docker reports it. */
  raw: string;
  /** "id" when Docker reports only an image ID (the tag moved or was removed). */
  kind: "reference" | "id" | "unknown";
  /** A registry host when the reference names one (lscr.io, ghcr.io, localhost:5000). */
  registry: string | null;
  repository: string;
  tag: string | null;
  digest: string | null;
}

const HEX64 = /^[0-9a-f]{64}$/;

/** Splits an image reference into registry, repository, tag and digest. */
export function parseImage(raw: string | null | undefined): ImageRef {
  const value = (raw ?? "").trim();
  if (!value) return { raw: "", kind: "unknown", registry: null, repository: "", tag: null, digest: null };
  const bareId = value.startsWith("sha256:") ? value.slice(7) : value;
  if (HEX64.test(bareId)) return { raw: value, kind: "id", registry: null, repository: "", tag: null, digest: `sha256:${bareId}` };
  let rest = value;
  let digest: string | null = null;
  const at = rest.indexOf("@");
  if (at !== -1) {
    digest = rest.slice(at + 1) || null;
    rest = rest.slice(0, at);
  }
  let tag: string | null = null;
  const colon = rest.lastIndexOf(":");
  if (colon > rest.lastIndexOf("/")) {
    tag = rest.slice(colon + 1) || null;
    rest = rest.slice(0, colon);
  }
  let registry: string | null = null;
  const slash = rest.indexOf("/");
  if (slash !== -1) {
    const first = rest.slice(0, slash);
    if (first.includes(".") || first.includes(":") || first === "localhost") {
      registry = first;
      rest = rest.slice(slash + 1);
    }
  }
  // Docker Hub's official images live under library/; nobody types that.
  if ((registry === null || registry === "docker.io") && rest.startsWith("library/")) rest = rest.slice(8);
  return { raw: value, kind: "reference", registry, repository: rest, tag, digest };
}

/** The short form shown in a row: repository and tag, or a short image ID. */
export function imageLabel(image: ImageRef): string {
  if (image.kind === "unknown") return "Unknown image";
  if (image.kind === "id") return `image ${image.digest!.slice(7, 19)}`;
  return image.tag ? `${image.repository}:${image.tag}` : image.repository;
}

// ---------------------------------------------------------------- containers

export const containerId = (r: Resource) => (r.id.startsWith("container:") ? r.id.slice(10) : r.id);
export const shortId = (r: Resource) => containerId(r).slice(0, 12);
export const stackOf = (r: Resource): string | null => (r.parent?.startsWith("stack:") ? r.parent.slice(6) || null : null);
export const isRunning = (r: Resource) => r.status === "running";
/** Statuses the start action accepts, as the host's plan does. */
export const STARTABLE = new Set(["exited", "created", "stopped"]);

const IN_FLIGHT = new Set(["queued", "running", "verifying"]);
const STUCK = new Set(["needs_intervention", "outcome_unknown"]);
export const jobInFlight = (job: RestartJob) => IN_FLIGHT.has(job.state);
export const jobStuck = (job: RestartJob) => STUCK.has(job.state);

/** The newest job per container resource id (jobs arrive newest first). */
export function latestJobs(jobs: RestartJob[]): Map<string, RestartJob> {
  const latest = new Map<string, RestartJob>();
  for (const job of jobs) {
    const id = job.plan.expected.resource;
    if (!latest.has(id)) latest.set(id, job);
  }
  return latest;
}

/**
 * Why a container needs attention, or null. Precisely:
 * 1. its healthcheck reports "unhealthy";
 * 2. Docker reports it "restarting" (a restart loop) or "dead";
 * 3. the host no longer reports it ("missing" after it was seen);
 * 4. it "exited" while another member of its stack is running. Docker's
 *    exit code is not in the overview, so a stopped stack member beside
 *    running siblings is the evidence of an unexpected exit; a standalone
 *    exited container, or a wholly stopped stack, is a deliberate stop;
 * 5. the principal's latest operation on it needs intervention or has an
 *    unknown outcome.
 * Stopped, created and paused containers are not, by themselves, a problem.
 */
export function attentionReason(r: Resource, runningStacks: Set<string>, job?: RestartJob): string | null {
  if (r.health === "unhealthy") return "Its healthcheck is failing";
  if (r.status === "restarting") return "Docker keeps restarting it";
  if (r.status === "dead") return "Docker reports it dead";
  if (r.status === "missing") return "The host no longer reports it";
  const stack = stackOf(r);
  if (r.status === "exited" && stack && runningStacks.has(stack)) return "Stopped while the rest of its stack runs";
  if (job && jobStuck(job)) return "Its last operation needs attention";
  return null;
}

/** Stacks with at least one running member, from the whole list (not a filtered view). */
export function stacksWithRunning(containers: Resource[]): Set<string> {
  const result = new Set<string>();
  for (const c of containers) {
    const stack = stackOf(c);
    if (stack && isRunning(c)) result.add(stack);
  }
  return result;
}

export interface Row {
  resource: Resource;
  name: string;
  stack: string | null;
  image: ImageRef;
  running: boolean;
  /** Null when not needing attention. */
  attention: string | null;
  /** Null when unknown (not reported, or the docker source is not current). */
  cpu: number | null;
  memory: number | null;
  job: RestartJob | null;
}

const finite = (value: number | null | undefined): value is number => typeof value === "number" && Number.isFinite(value);

/** Every container in the overview as a row; readings are withheld unless current. */
export function buildRows(resources: Resource[], jobs: RestartJob[], current: boolean): Row[] {
  const containers = resources.filter((r) => r.kind === "container");
  const running = stacksWithRunning(containers);
  const latest = latestJobs(jobs);
  return containers.map((resource) => {
    const job = latest.get(resource.id) ?? null;
    return {
      resource,
      name: resource.name,
      stack: stackOf(resource),
      image: parseImage(resource.image),
      running: isRunning(resource),
      attention: attentionReason(resource, running, job ?? undefined),
      cpu: current && finite(resource.cpu_percent) ? resource.cpu_percent : null,
      memory: current && finite(resource.memory_percent) ? resource.memory_percent : null,
      job,
    };
  });
}

export function matchesFilter(row: Row, filter: Filter): boolean {
  if (filter === "running") return row.running;
  if (filter === "stopped") return !row.running;
  if (filter === "attention") return row.attention !== null;
  return true;
}

/** Case-insensitive substring search over name, image (as reported and as shown) and stack. */
export function matchesSearch(row: Row, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return [row.name, row.image.raw, imageLabel(row.image), row.stack ?? ""].some((field) => field.toLowerCase().includes(needle));
}

export function counts(rows: Row[]): Record<Filter, number> {
  return {
    all: rows.length,
    running: rows.filter((r) => r.running).length,
    stopped: rows.filter((r) => !r.running).length,
    attention: rows.filter((r) => r.attention !== null).length,
  };
}

const byName = (a: Row, b: Row) => a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true }) || a.resource.id.localeCompare(b.resource.id);

/** Name A–Z; CPU or memory highest first, unknown last, ties by name. */
export function sortRows(rows: Row[], sort: Sort): Row[] {
  const key = sort === "cpu" ? "cpu" : sort === "memory" ? "memory" : null;
  return [...rows].sort((a, b) => {
    if (key) {
      const x = a[key], y = b[key];
      if (x === null && y !== null) return 1;
      if (y === null && x !== null) return -1;
      if (x !== null && y !== null && x !== y) return y - x;
    }
    return byName(a, b);
  });
}

export function visibleRows(rows: Row[], view: Pick<ViewState, "filter" | "query" | "sort">): Row[] {
  return sortRows(rows.filter((r) => matchesFilter(r, view.filter) && matchesSearch(r, view.query)), view.sort);
}

export interface Group {
  /** The stack's project name; null for containers outside any stack. */
  stack: string | null;
  label: string;
  rows: Row[];
  /** Over the whole stack, not just the rows shown. */
  running: number;
  total: number;
}

/**
 * Groups shown rows by stack: stacks A–Z, then containers outside any stack.
 * running/total count every member of the stack in `all`, so a filtered view
 * still says how the whole stack is doing.
 */
export function groupRows(shown: Row[], all: Row[]): Group[] {
  const groups = new Map<string | null, Row[]>();
  for (const row of shown) {
    const list = groups.get(row.stack) ?? [];
    list.push(row);
    groups.set(row.stack, list);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === null ? 1 : b === null ? -1 : a.localeCompare(b, undefined, { sensitivity: "base", numeric: true })))
    .map(([stack, rows]) => {
      const members = all.filter((r) => r.stack === stack);
      return {
        stack,
        label: stack ?? "Not in a stack",
        rows,
        running: members.filter((r) => r.running).length,
        total: members.length,
      };
    });
}

// ---------------------------------------------------------------- actions

export interface ActionState {
  action: ContainerAction;
  label: string;
  /** Null when allowed; otherwise why not, in words. */
  disabled: string | null;
}

const actionLabels: Record<ContainerAction, string> = { start: "Start", stop: "Stop", restart: "Restart" };
export const actionLabel = (action: ContainerAction) => actionLabels[action];

/** Start, stop and restart with the reason each is unavailable (mirrors the host's plan rules). */
export function actionStates(row: Pick<Row, "resource" | "running" | "job">): ActionState[] {
  const status = row.resource.status;
  const busy = row.job && jobInFlight(row.job) ? "An operation on it is in progress" : null;
  const stuck = row.job && jobStuck(row.job) ? "Its last operation must be resolved first" : null;
  const blocked = busy ?? stuck;
  const notRunning = status === "missing" ? "The host no longer reports it" : `It is ${status}, not running`;
  return [
    {
      action: "start",
      label: "Start",
      disabled: blocked ?? (row.running ? "It is already running" : STARTABLE.has(status) ? null : `It can't be started while ${status}`),
    },
    { action: "stop", label: "Stop", disabled: blocked ?? (row.running ? null : notRunning) },
    { action: "restart", label: "Restart", disabled: blocked ?? (row.running ? null : notRunning) },
  ];
}

/** The one action a row offers directly: Restart when running, Start when it can start, else none. */
export function primaryAction(row: Pick<Row, "resource" | "running">): ContainerAction | null {
  if (row.running) return "restart";
  return STARTABLE.has(row.resource.status) ? "start" : null;
}

// ---------------------------------------------------------------- jobs on a row

export interface JobBadge {
  label: string;
  tone: "info" | "warning" | "danger";
}

const verbs: Record<ContainerAction, [string, string, string]> = {
  restart: ["Restart queued", "Restarting", "Checking the restart"],
  start: ["Start queued", "Starting", "Checking the start"],
  stop: ["Stop queued", "Stopping", "Checking the stop"],
};
/** Seconds a finished-but-failed job stays on its row. */
export const RECENT_FAILURE_SECONDS = 15 * 60;

/** What a row says about its latest job: in flight, stuck, or recently failed; otherwise nothing. */
export function jobBadge(job: RestartJob | null, now: number): JobBadge | null {
  if (!job) return null;
  const action = job.plan.operation ?? "restart";
  const [queued, running, verifying] = verbs[action];
  if (job.state === "queued") return { label: queued, tone: "info" };
  if (job.state === "running") return { label: running, tone: "info" };
  if (job.state === "verifying") return { label: verifying, tone: "info" };
  if (jobStuck(job)) return { label: `${actionLabels[action]} needs attention`, tone: "danger" };
  const recent = now - job.plan.created_at < RECENT_FAILURE_SECONDS;
  if (recent && job.state === "failed") return { label: `${actionLabels[action]} failed`, tone: "warning" };
  if (recent && job.state === "precondition_changed") return { label: `${actionLabels[action]} not run: it changed first`, tone: "warning" };
  return null;
}

/** Job ids that left the in-flight states between two reads. */
export function settledJobs(before: RestartJob[], after: RestartJob[]): string[] {
  const now = new Map(after.map((job) => [job.id, job]));
  return before.filter((job) => jobInFlight(job) && now.has(job.id) && !jobInFlight(now.get(job.id)!)).map((job) => job.id);
}

// ---------------------------------------------------------------- logs

export const LOG_TAILS = [50, 100, 200] as const;
export type LogTail = (typeof LOG_TAILS)[number];
/** The host's limits (crates/domain/src/logs.rs): 200 lines and 24 KiB. */
export const LOG_MAX_LINES = 200;
export const LOG_MAX_BYTES = 24 * 1024;
export const LOG_REFRESH_MS = 5000;

/** Lines in a log text, not counting a final newline. */
export function lineCount(text: string): number {
  if (!text) return 0;
  const lines = text.split("\n");
  return lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
}

/** "Showing the last 100 lines" with the honest qualifiers. */
export function logNote(tail: number, text: string, truncated: boolean): string {
  const lines = lineCount(text);
  if (lines === 0) return `Asked for the last ${tail} lines; the container has written nothing.`;
  if (truncated) return `Showing the newest ${lines} of the last ${tail} lines.`;
  if (lines < tail) return `Showing all ${lines} ${lines === 1 ? "line" : "lines"}; the container has written fewer than ${tail}.`;
  return `Showing the last ${tail} lines.`;
}
