import { ApiError } from "./api";
import { automationClient } from "./automation-client";
import type { AlertState, Notification, Page, ScheduleView } from "./automation-client";

export type PageKind = "schedules" | "alerts" | "notifications";
type Rows = { schedules: ScheduleView; alerts: AlertState; notifications: Notification };
export type ReadState<T> = {
  page: Page<T> | null;
  after: string | undefined;
  loading: boolean;
  stale: boolean;
  error: string | null;
};
export type AutomationState = { [K in PageKind]: ReadState<Rows[K]> } & { expired: boolean };
type Reader = Pick<typeof automationClient, PageKind>;
const kinds: PageKind[] = ["schedules", "alerts", "notifications"];
function empty<T>(): ReadState<T> { return { page: null, after: undefined, loading: true, stale: false, error: null }; }
function initial(): AutomationState { return { schedules: empty(), alerts: empty(), notifications: empty(), expired: false }; }
function code(error: unknown): string | undefined { return error instanceof ApiError ? error.detail.code : undefined; }
function message(error: unknown): string {
  return error instanceof ApiError ? `${error.message} · ${error.detail.code} · reference ${error.detail.audit_id}` : "Unable to refresh. Check the connection and try again.";
}

/** One page per section. This owns no authority or durable state and never sends a mutation. */
export class AutomationController {
  private state = initial();
  private readonly listeners = new Set<() => void>();
  private readonly active: Partial<Record<PageKind, { serial: number; abort: AbortController; after: string | undefined }>> = {};
  private serial = 0;
  private lifetime = 0;
  private closed = false;
  private running: Promise<void> | undefined;
  private refreshAgain = false;
  constructor(private readonly expired: () => void, private readonly reader: Reader = automationClient) {}
  getSnapshot = (): AutomationState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private emit(): void { for (const listener of this.listeners) listener(); }
  private set<K extends PageKind>(kind: K, state: ReadState<Rows[K]>): void {
    this.state = { ...this.state, [kind]: state }; this.emit();
  }
  private get<K extends PageKind>(kind: K): ReadState<Rows[K]> {
    // Preserve the correlation between the selected page and its row type.
    return this.state[kind] as ReadState<Rows[K]>;
  }
  private cancel(): void {
    for (const kind of kinds) { this.active[kind]?.abort.abort(); delete this.active[kind]; }
  }
  /** May restart after React's development effect cleanup; old request generations stay invalid. */
  start(): Promise<void> {
    if (this.state.expired) return Promise.resolve();
    if (this.closed) { this.closed = false; this.lifetime++; this.running = undefined; }
    return this.refresh();
  }
  dispose(): void {
    if (this.closed) return;
    this.closed = true; this.lifetime++; this.refreshAgain = false;
    this.cancel(); this.state = { ...initial(), expired: this.state.expired }; this.emit();
  }
  private expire(): void {
    if (this.closed || this.state.expired) return;
    this.lifetime++; this.refreshAgain = false; this.cancel();
    this.state = { ...initial(), expired: true }; this.emit(); this.expired();
  }
  private async read<K extends PageKind>(kind: K, after: string | undefined): Promise<void> {
    if (this.closed || this.state.expired) return;
    this.active[kind]?.abort.abort();
    const request = { serial: ++this.serial, abort: new AbortController(), after };
    const lifetime = this.lifetime;
    this.active[kind] = request;
    const previous = this.get(kind);
    this.set(kind, { ...previous, loading: true, stale: previous.page !== null, error: null });
    const current = () => !this.closed && !this.state.expired && this.lifetime === lifetime && this.active[kind]?.serial === request.serial;
    try {
      const fetchPage = this.reader[kind] as (after?: string, signal?: AbortSignal) => Promise<Page<Rows[K]>>;
      const page = await fetchPage(after, request.abort.signal);
      if (current()) this.set(kind, { page, after, loading: false, stale: false, error: null });
    } catch (error) {
      if (!current()) return;
      if (code(error) === "unauthenticated") { this.expire(); return; }
      const denied = code(error) === "forbidden" || code(error) === "expired";
      this.set(kind, { ...previous, page: denied ? null : previous.page, loading: false, stale: previous.page !== null, error: message(error) });
    } finally {
      if (this.active[kind]?.serial === request.serial) delete this.active[kind];
    }
  }
  /** One batch of three reads at a time; pending triggers coalesce into one follow-up. */
  refresh(): Promise<void> {
    if (this.closed || this.state.expired) return Promise.resolve();
    if (this.running) { this.refreshAgain = true; return this.running; }
    const lifetime = this.lifetime;
    const task = (async () => {
      do {
        this.refreshAgain = false;
        await Promise.all(kinds.map(kind => this.read(kind, this.active[kind] ? this.active[kind]!.after : this.state[kind].after)));
      } while (this.refreshAgain && !this.closed && !this.state.expired && this.lifetime === lifetime);
    })();
    this.running = task;
    void task.finally(() => { if (this.running === task) this.running = undefined; }).catch(() => undefined);
    return task;
  }
  next(kind: PageKind): Promise<void> {
    const cursor = this.state[kind].page?.next;
    return cursor ? this.read(kind, cursor) : Promise.resolve();
  }
  first(kind: PageKind): Promise<void> { return this.read(kind, undefined); }
  private invalidate(kind: PageKind): void {
    this.active[kind]?.abort.abort(); delete this.active[kind];
    this.set(kind, { ...this.state[kind], stale: true });
  }
  committed(kind: PageKind): Promise<void> {
    if (this.closed || this.state.expired) return Promise.resolve();
    this.invalidate(kind);
    return this.refresh();
  }
  mutationFailed(kind: PageKind, error: unknown): Promise<void> {
    if (this.closed || this.state.expired) return Promise.resolve();
    if (code(error) === "unauthenticated") { this.expire(); return Promise.resolve(); }
    this.invalidate(kind);
    const denied = code(error) === "forbidden" || code(error) === "expired";
    const previous = this.state[kind];
    this.set(kind, { ...previous, page: denied ? null : previous.page, stale: true, error: message(error) });
    // Re-read committed state; never retry the failed POST.
    return this.refresh();
  }
}
