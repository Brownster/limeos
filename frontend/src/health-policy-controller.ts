import { ApiError } from "./api";
import { healthPolicyClient } from "./health-policy-client";
import type { HealthPolicyDefinition, HealthPolicyView } from "./health-policy-client";

type Client = typeof healthPolicyClient;
type State = { value: HealthPolicyView | null; loading: boolean; pending: boolean; stale: boolean; denied: boolean; expired: boolean; error: string; notice: string };
export class HealthPolicyController {
  private state: State = { value: null, loading: false, pending: false, stale: true, denied: false, expired: false, error: "", notice: "" };
  private listeners = new Set<() => void>(); private disposed = false; private epoch = 0; private active: AbortController | null = null;
  constructor(private expired: () => void, private client: Client = healthPolicyClient) {}
  getSnapshot = (): State => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(value: Partial<State>) { this.state = { ...this.state, ...value }; this.listeners.forEach(listener => listener()); }
  private current(epoch: number) { return !this.disposed && !this.state.expired && this.epoch === epoch; }
  private failure(error: unknown) {
    const code = error instanceof ApiError ? error.detail.code : "unavailable";
    if (code === "unauthenticated") { this.update({ value: null, expired: true, loading: false, pending: false, stale: true }); this.expired(); return; }
    this.update({ error: error instanceof ApiError ? error.message : "Health settings are unavailable. Refresh to check saved state.", stale: true, ...(code === "forbidden" ? { value: null, denied: true } : {}) });
  }
  async refresh() {
    if (this.disposed || this.state.expired || this.state.pending) return;
    this.active?.abort(); const abort = new AbortController(); this.active = abort; const epoch = ++this.epoch;
    this.update({ loading: true, stale: true, error: "" });
    try { const value = await this.client.read(abort.signal); if (this.current(epoch)) this.update({ value, loading: false, stale: false, denied: false }); }
    catch (error) { if (this.current(epoch)) { this.update({ loading: false }); this.failure(error); } }
  }
  async save(definition: HealthPolicyDefinition, csrf: string) {
    if (this.disposed || this.state.expired || !this.state.value || this.state.pending || this.state.loading || this.state.stale || this.state.denied) return;
    const revision = this.state.value.revision; this.active?.abort(); const abort = new AbortController(); this.active = abort; const epoch = ++this.epoch;
    this.update({ pending: true, error: "", notice: "" });
    try {
      const value = await this.client.replace({ revision, definition }, csrf, abort.signal);
      if (this.current(epoch)) this.update({ value, pending: false, stale: false, notice: "Health settings saved. Health coverage remains partial." });
    } catch (error) {
      if (!this.current(epoch)) return;
      this.update({ pending: false, stale: true, notice: "The save was not confirmed. Check refreshed saved settings before saving again." }); this.failure(error);
      if (!this.state.expired && !this.state.denied) await this.refresh();
    }
  }
  dispose() { this.disposed = true; this.epoch++; this.active?.abort(); this.active = null; this.listeners.clear(); }
}
