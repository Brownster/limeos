import { inventoryStale, readStackInventory, StackReadError } from "./stack-inventory-client";
import type { StackInventory } from "./stack-inventory-client";

export type StackReadState = { value: StackInventory | null; loading: boolean; stale: boolean; error: string; expired: boolean; denied: boolean };
type Reader = (signal: AbortSignal) => Promise<StackInventory>;
type Clock = { now: () => number; schedule: (callback: () => void, ms: number) => ReturnType<typeof setTimeout>; cancel: (id: ReturnType<typeof setTimeout>) => void };
const defaultClock: Clock = { now: () => Date.now() / 1000, schedule: setTimeout, cancel: clearTimeout };
const initial = (): StackReadState => ({ value: null, loading: false, stale: true, error: "", expired: false, denied: false });

/** One owner per authenticated principal/grant revision. No polling or writes. */
export class StackInventoryController {
  private state = initial(); private listeners = new Set<() => void>();
  private pending: Promise<void> | null = null; private abort: AbortController | null = null;
  private epoch = 0; private disposed = false; private timer: ReturnType<typeof setTimeout> | null = null;
  constructor(private expired: () => void, private read: Reader = readStackInventory, private clock: Clock = defaultClock) {}
  getSnapshot = (): StackReadState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<StackReadState>): void { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private clearTimer(): void { if (this.timer !== null) this.clock.cancel(this.timer); this.timer = null; }
  start(): Promise<void> { this.disposed = false; return this.refresh(); }
  refresh(): Promise<void> {
    if (this.disposed || this.state.expired || this.state.denied) return Promise.resolve();
    if (this.pending) return this.pending;
    const epoch = ++this.epoch; const controller = new AbortController(); this.abort = controller;
    const active = () => !this.disposed && epoch === this.epoch;
    const work = async () => {
      if (!active()) return;
      try {
        const value = await this.read(controller.signal); if (!active()) return;
        this.clearTimer(); const stale = inventoryStale(value, this.clock.now());
        this.update({ value, stale, error: "" });
        if (!stale) {
          const expires = Math.min(...[value.catalog, value.engine].map(s => s.sampled_at! + s.max_age_seconds));
          this.timer = this.clock.schedule(() => { this.timer = null; if (!this.disposed && this.state.value === value) this.update({ stale: true }); }, Math.max(1, (expires - this.clock.now()) * 1000 + 1));
        }
      } catch (error) {
        if (!active()) return;
        const status = error instanceof StackReadError ? error.status : 0;
        if (status === 401 || status === 403) {
          this.clearTimer(); this.update({ value: null, stale: true, error: error instanceof Error ? error.message : "Access is unavailable.", expired: status === 401, denied: status === 403 });
          if (status === 401) this.expired();
        } else this.update({ stale: true, error: error instanceof Error ? error.message : "Stack inventory is unavailable. Refresh to try again." });
      } finally {
        if (active()) { this.pending = null; this.abort = null; this.update({ loading: false }); }
      }
    };
    // Deferring also makes coalescing hold when a test/backend throws synchronously.
    const pending = Promise.resolve().then(work); this.pending = pending;
    this.update({ loading: true, error: "", stale: this.state.stale || (this.state.value !== null && inventoryStale(this.state.value, this.clock.now())) });
    return pending;
  }
  dispose(): void {
    this.disposed = true; this.epoch++; this.abort?.abort(); this.abort = null; this.pending = null;
    this.clearTimer(); this.update(initial());
  }
}
