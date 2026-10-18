import { ToolsReadError, ageToolsStatus, readToolsStatus, toolsExpiryDelay } from "./tools-status-client";
import type { ToolsStatus } from "./tools-status-client";

export type ToolsReadState = { value: ToolsStatus | null; loading: boolean; error: string; expired: boolean; denied: boolean };
type Reader = (signal: AbortSignal) => Promise<ToolsStatus>;
type Clock = { now: () => number; schedule: (callback: () => void, ms: number) => ReturnType<typeof setTimeout>; cancel: (id: ReturnType<typeof setTimeout>) => void };
const defaultClock: Clock = { now: () => Date.now() / 1000, schedule: setTimeout, cancel: clearTimeout };
const initial = (): ToolsReadState => ({ value: null, loading: false, error: "", expired: false, denied: false });

/** One principal, grant and host/boot/cache lifetime; cache reads never poll. */
export class ToolsStatusController {
  private state = initial(); private listeners = new Set<() => void>();
  private pending: Promise<void> | null = null; private abort: AbortController | null = null;
  private epoch = 0; private disposed = false; private timer: ReturnType<typeof setTimeout> | null = null;
  constructor(private expired: () => void, private read: Reader = readToolsStatus, private clock: Clock = defaultClock) {}
  getSnapshot = (): ToolsReadState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<ToolsReadState>): void { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private clearTimer(): void { if (this.timer !== null) this.clock.cancel(this.timer); this.timer = null; }
  private armExpiry(): void {
    this.clearTimer(); const value = this.state.value; if (!value) return;
    const delay = toolsExpiryDelay(value, this.clock.now()); if (delay === null) return;
    this.timer = this.clock.schedule(() => {
      this.timer = null; if (this.disposed || this.state.value !== value) return;
      const aged = ageToolsStatus(value, this.clock.now());
      this.update({ value: aged, error: aged === null ? "The tools observation is no longer current after a clock change. Refresh to try again." : "" });
      this.armExpiry();
    }, delay);
  }
  private reconcile(): void {
    const value = this.state.value; if (!value) return;
    const aged = ageToolsStatus(value, this.clock.now()); if (aged === value) return;
    this.clearTimer(); this.update({ value: aged, error: aged === null ? "The tools observation is no longer current after a clock change. Refresh to try again." : "" });
    this.armExpiry();
  }
  start(): Promise<void> { this.disposed = false; return this.refresh(); }
  refresh(): Promise<void> {
    if (this.disposed || this.state.expired || this.state.denied) return Promise.resolve();
    this.reconcile(); if (this.pending) return this.pending;
    const epoch = ++this.epoch; const controller = new AbortController(); this.abort = controller;
    const active = () => !this.disposed && epoch === this.epoch;
    const work = async () => {
      if (!active()) return;
      try {
        const read = await this.read(controller.signal); if (!active()) return;
        const value = ageToolsStatus(read, this.clock.now()); this.clearTimer();
        this.update({ value, error: value === null ? "The tools observation is no longer current after a clock change. Refresh to try again." : "" });
        this.armExpiry();
      } catch (error) {
        if (!active()) return;
        this.clearTimer(); const status = error instanceof ToolsReadError ? error.status : 0;
        this.update({ value: null, error: status === 401 ? "Your session has expired." : status === 403 ? "You no longer have access to tools status." : "Tools status is unavailable. Refresh to try again.", expired: status === 401, denied: status === 403 });
        if (status === 401) this.expired();
      } finally {
        if (active()) { this.pending = null; this.abort = null; this.update({ loading: false }); }
      }
    };
    const pending = Promise.resolve().then(work); this.pending = pending; this.update({ loading: true, error: "" }); return pending;
  }
  dispose(): void {
    this.disposed = true; this.epoch++; this.abort?.abort(); this.abort = null; this.pending = null;
    this.clearTimer(); this.update(initial());
  }
}
