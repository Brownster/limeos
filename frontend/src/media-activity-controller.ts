import { ActivityReadError, readMediaActivity } from "./media-activity-client";
import { activityExpired } from "./media-activity-model";
import type { ActivitySnapshot } from "./media-activity-model";

export type ActivityReadState = { value: ActivitySnapshot | null; loading: boolean; stale: boolean; error: string; expired: boolean; denied: boolean };
type Reader = (signal: AbortSignal) => Promise<ActivitySnapshot>;
type Clock = { now: () => number; schedule: (callback: () => void, ms: number) => ReturnType<typeof setTimeout>; cancel: (id: ReturnType<typeof setTimeout>) => void };
const defaultClock: Clock = { now: () => Date.now() / 1000, schedule: setTimeout, cancel: clearTimeout };
const initial = (): ActivityReadState => ({ value: null, loading: false, stale: true, error: "", expired: false, denied: false });

/** Own one authenticated principal/grant lifetime. Observations never authorize effects. */
export class MediaActivityController {
  private state = initial(); private listeners = new Set<() => void>();
  private pending: Promise<void> | null = null; private abort: AbortController | null = null;
  private epoch = 0; private disposed = false; private timer: ReturnType<typeof setTimeout> | null = null;
  constructor(private expired: () => void, private read: Reader = readMediaActivity, private clock: Clock = defaultClock) {}
  getSnapshot = (): ActivityReadState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<ActivityReadState>): void { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private clearTimer(): void { if (this.timer !== null) this.clock.cancel(this.timer); this.timer = null; }
  private isStale(value: ActivitySnapshot): boolean { return activityExpired(value, Math.floor(this.clock.now())); }
  private armExpiry(value: ActivitySnapshot): void {
    this.clearTimer();
    if (this.isStale(value)) return;
    this.timer = this.clock.schedule(() => {
      this.timer = null;
      if (this.disposed || this.state.value !== value) return;
      if (this.isStale(value)) this.update({ stale: true });
      else this.armExpiry(value); // An early timer keeps the original deadline.
    }, Math.max(1, (value.sampled_at! + value.max_age_seconds + 1 - this.clock.now()) * 1000));
  }
  start(): Promise<void> { this.disposed = false; return this.refresh(); }
  refresh(): Promise<void> {
    if (this.disposed || this.state.expired || this.state.denied) return Promise.resolve();
    if (this.state.value && this.isStale(this.state.value) && !this.state.stale) this.update({ stale: true });
    if (this.pending) return this.pending;
    const epoch = ++this.epoch; const controller = new AbortController(); this.abort = controller;
    const active = () => !this.disposed && epoch === this.epoch;
    const work = async () => {
      if (!active()) return;
      try {
        const value = await this.read(controller.signal); if (!active()) return;
        const stale = this.isStale(value); this.clearTimer();
        this.update({ value, stale, error: "" }); if (!stale) this.armExpiry(value);
      } catch (error) {
        if (!active()) return;
        this.clearTimer(); const status = error instanceof ActivityReadError ? error.status : 0;
        if (status === 401 || status === 403) {
          this.update({ value: null, stale: true, error: status === 401 ? "Your session has expired." : "You no longer have access to media activity.", expired: status === 401, denied: status === 403 });
          if (status === 401) this.expired();
        } else this.update({ stale: true, error: "Media activity is unavailable. Refresh to try again." });
      } finally {
        if (active()) { this.pending = null; this.abort = null; this.update({ loading: false }); }
      }
    };
    // Coalesce even a synchronously throwing adapter and do not launch after disposal.
    const pending = Promise.resolve().then(work); this.pending = pending;
    this.update({ loading: true, error: "" }); return pending;
  }
  dispose(): void {
    this.disposed = true; this.epoch++; this.abort?.abort(); this.abort = null; this.pending = null;
    this.clearTimer(); this.update(initial());
  }
}
