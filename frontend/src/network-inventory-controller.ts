import { NetworkReadError, networkInventoryExpired, readNetworkInventory } from "./network-inventory-client";
import type { NetworkInventory } from "./network-inventory-client";

export type NetworkReadState = { value: NetworkInventory | null; loading: boolean; stale: boolean; error: string; expired: boolean; denied: boolean };
type Reader = (signal: AbortSignal) => Promise<NetworkInventory>;
type Clock = { now: () => number; schedule: (callback: () => void, ms: number) => ReturnType<typeof setTimeout>; cancel: (id: ReturnType<typeof setTimeout>) => void };
const defaultClock: Clock = { now: () => Date.now() / 1000, schedule: setTimeout, cancel: clearTimeout };
const initial = (): NetworkReadState => ({ value: null, loading: false, stale: true, error: "", expired: false, denied: false });

/** One authenticated principal/grant lifetime. Refresh reads only the cache. */
export class NetworkInventoryController {
  private state = initial(); private listeners = new Set<() => void>();
  private pending: Promise<void> | null = null; private abort: AbortController | null = null;
  private epoch = 0; private disposed = false; private timer: ReturnType<typeof setTimeout> | null = null;
  constructor(private expired: () => void, private read: Reader = readNetworkInventory, private clock: Clock = defaultClock) {}
  getSnapshot = (): NetworkReadState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<NetworkReadState>): void { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private clearTimer(): void { if (this.timer !== null) this.clock.cancel(this.timer); this.timer = null; }
  private isStale(value: NetworkInventory): boolean { return networkInventoryExpired(value, this.clock.now()); }
  private expireValue(): void { this.clearTimer(); this.update({ value: null, stale: true, error: "The network observation has expired. Refresh to try again." }); }
  private armExpiry(value: NetworkInventory): void {
    this.clearTimer(); if (this.isStale(value)) return;
    this.timer = this.clock.schedule(() => {
      this.timer = null;
      if (this.disposed || this.state.value !== value) return;
      if (this.isStale(value)) this.expireValue();
      else this.armExpiry(value);
    }, Math.max(1, (value.sampled_at! + value.max_age_seconds + 1 - this.clock.now()) * 1000));
  }
  start(): Promise<void> { this.disposed = false; return this.refresh(); }
  refresh(): Promise<void> {
    if (this.disposed || this.state.expired || this.state.denied) return Promise.resolve();
    if (this.state.value && this.isStale(this.state.value)) this.expireValue();
    if (this.pending) return this.pending;
    const epoch = ++this.epoch; const controller = new AbortController(); this.abort = controller;
    const active = () => !this.disposed && epoch === this.epoch;
    const work = async () => {
      if (!active()) return;
      try {
        const value = await this.read(controller.signal); if (!active()) return;
        const stale = this.isStale(value); this.clearTimer();
        if (stale && value.source === "fresh") this.expireValue();
        else this.update({ value, stale, error: "" });
        if (!stale) this.armExpiry(value);
      } catch (error) {
        if (!active()) return;
        this.clearTimer(); const status = error instanceof NetworkReadError ? error.status : 0;
        this.update({ value: null, stale: true, error: status === 401 ? "Your session has expired." : status === 403 ? "You no longer have access to network groups." : "Network groups are unavailable. Refresh to try again.", expired: status === 401, denied: status === 403 });
        if (status === 401) this.expired();
      } finally {
        if (active()) { this.pending = null; this.abort = null; this.update({ loading: false }); }
      }
    };
    const pending = Promise.resolve().then(work); this.pending = pending;
    this.update({ loading: true, error: "" }); return pending;
  }
  dispose(): void {
    this.disposed = true; this.epoch++; this.abort?.abort(); this.abort = null; this.pending = null;
    this.clearTimer(); this.update(initial());
  }
}
