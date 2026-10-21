import { ApiError } from "./api";
import { automationClient } from "./automation-client";
import type { ObserveHealthEnabled } from "../../contracts/generated/types";

type State = { pending: boolean; confirmed: boolean; error: string | null };
const initial = (): State => ({ pending: false, confirmed: false, error: null });

/** Current-owner password confirmation. No credential is retained in state. */
export class HealthEnablementController {
  private state = initial();
  private listeners = new Set<() => void>();
  private request: { abort: AbortController } | undefined;
  private lifetime = 0;
  private closed = false;
  constructor(private readonly settled: (result?: ObserveHealthEnabled) => void, private readonly expired: () => void, private readonly send = automationClient.enableHealth) {}
  getSnapshot = (): State => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private emit(): void { for (const listener of this.listeners) listener(); }
  start(): void { if (this.closed) { this.closed = false; this.lifetime++; this.state = initial(); this.emit(); } }
  dispose(): void {
    if (this.closed) return;
    this.closed = true; this.lifetime++; this.request?.abort.abort(); this.request = undefined;
    this.state = initial(); this.emit();
  }
  async confirm(password: string, csrf: string): Promise<void> {
    if (this.closed || this.state.pending || this.state.confirmed) return;
    const request = { abort: new AbortController() }; const lifetime = this.lifetime;
    this.request = request; this.state = { pending: true, confirmed: false, error: null }; this.emit();
    const current = () => !this.closed && this.lifetime === lifetime && this.request === request;
    if (!current()) return;
    try {
      const result = await this.send(password, csrf, request.abort.signal);
      if (!current()) return;
      this.state = { pending: false, confirmed: true, error: null }; this.emit();
      if (current()) this.settled(result);
    } catch (error) {
      if (!current()) return;
      if (error instanceof ApiError && error.detail.code === "unauthenticated") {
        const disposed = this.lifetime + 1; this.dispose();
        if (this.closed && this.lifetime === disposed) this.expired();
        return;
      }
      const uncertain = !(error instanceof ApiError) || ["unavailable", "timeout", "state_not_durable"].includes(error.detail.code);
      this.state = { pending: false, confirmed: false, error: uncertain ? "Permission change is unconfirmed. Refresh or sign in again before another request." : `${error.message} · reference ${error.detail.audit_id}` };
      this.emit(); if (current()) this.settled();
    } finally { if (this.request === request) this.request = undefined; }
  }
}
