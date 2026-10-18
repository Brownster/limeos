import type { PlannedStorageMount, StorageDevice, StorageMountJob } from "../../../contracts/generated/types";
import { ApiError } from "../api";
import { mountApi } from "./client";
import type { MountApi } from "./client";

export type MountState = {
  target: string; targets: StorageDevice[]; catalogLoading: boolean; catalogError: string;
  jobs: StorageMountJob[]; historyLoading: boolean; historyError: string;
  proposal: PlannedStorageMount | null; reviewOpen: boolean; pending: boolean; error: string;
  retryAvailable: boolean; canceling: string | null; expired: boolean; denied: boolean;
};
const initial = (): MountState => ({ target: "", targets: [], catalogLoading: true, catalogError: "", jobs: [], historyLoading: true, historyError: "", proposal: null, reviewOpen: false, pending: false, error: "", retryAvailable: false, canceling: null, expired: false, denied: false });
const invalid = () => new Error("Invalid mount response");
const opaque = (value: unknown): value is string => typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
const text = (value: unknown, limit: number): value is string => typeof value === "string" && value.length > 0 && new TextEncoder().encode(value).length <= limit && !/\p{Cc}/u.test(value);
function device(value: StorageDevice): boolean {
  return !!value && text(value.id, 64) && value.role === "data" && value.filesystem === "ext4" && text(value.filesystem_uuid, 128) && text(value.mountpoint, 512) && value.mountpoint.startsWith("/");
}
function plan(value: PlannedStorageMount["plan"]): boolean {
  return !!value && opaque(value.id) && value.version === 1 && text(value.principal, 64) && Number.isSafeInteger(value.grant_revision) && value.grant_revision > 0 && !!value.effect && value.effect.version === 1 && value.effect.operation === "storage.mount" && opaque(value.effect.action) && device(value.effect.device) && Number.isSafeInteger(value.effect.created_at) && value.effect.created_at >= 0 && Number.isSafeInteger(value.effect.expires_at) && value.effect.expires_at > value.effect.created_at && value.effect.expires_at - value.effect.created_at <= 300;
}
function job(value: StorageMountJob): boolean {
  return !!value && opaque(value.id) && ["queued", "running", "verifying", "succeeded", "failed", "canceled", "precondition_changed", "outcome_unknown", "needs_intervention"].includes(value.state) && plan(value.plan) && value.id === value.plan.effect.action;
}
function message(error: unknown): string {
  if (!(error instanceof ApiError)) return "Mount operations are unavailable. Check recent jobs before retrying.";
  const messages: Partial<Record<ApiError["detail"]["code"], string>> = {
    expired: "The mount review or approval expired. Review the managed disk again.",
    conflict: "The mount state changed. Check recent jobs and review the managed disk again.",
    storage_mount_limit: "The mount check exceeded its supported collection limits.",
    corrupt_configuration: "The protected storage configuration could not be verified.",
    state_not_durable: "The mount request could not be recorded safely. Check recent jobs before retrying.",
  };
  const reference = /^[A-Za-z0-9_-]{1,128}$/.test(error.detail.audit_id) ? ` · reference ${error.detail.audit_id}` : "";
  const reason = Object.hasOwn(messages, error.detail.code) ? messages[error.detail.code] : undefined;
  return `${reason ?? "Mount operations are unavailable. Check recent jobs before retrying."}${reference}`;
}
/** One CSRF session lifetime. Server grants/physical authority remain mandatory. */
export class StorageMountController {
  private state = initial(); private listeners = new Set<() => void>();
  private generation = 0; private disposed = false; private controllers = new Set<AbortController>();
  private jobsVersion = 0; private jobsRead: Promise<void> | null = null; private jobsAbort: AbortController | null = null;
  private catalogRead: Promise<void> | null = null;
  private submission: { proposal: PlannedStorageMount; approval: string; key: string } | null = null;
  constructor(private csrf: string, private expired: () => void, private api: MountApi = mountApi, private now = () => Date.now() / 1000, private key = () => crypto.randomUUID()) {}
  getSnapshot = (): MountState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(patch: Partial<MountState>): void { this.state = { ...this.state, ...patch }; this.listeners.forEach(listener => listener()); }
  private available(): boolean { return !this.disposed && !this.state.expired && !this.state.denied; }
  private scope() {
    const generation = this.generation, controller = new AbortController(); this.controllers.add(controller);
    return { controller, signal: controller.signal, active: () => this.available() && generation === this.generation, close: () => { this.controllers.delete(controller); } };
  }
  private abortAll(): void { this.controllers.forEach(controller => controller.abort()); this.controllers.clear(); this.jobsRead = null; this.catalogRead = null; this.jobsAbort = null; this.jobsVersion++; }
  private accessFailure(error: unknown): boolean {
    const code = error instanceof ApiError ? error.detail.code : "";
    if (code !== "unauthenticated" && code !== "forbidden") return false;
    this.generation++; this.abortAll(); this.submission = null;
    this.update({ ...initial(), catalogLoading: false, historyLoading: false, expired: code === "unauthenticated", denied: code === "forbidden", error: code === "unauthenticated" ? "Your session has expired." : "You no longer have access to mount operations." });
    if (code === "unauthenticated") this.expired(); return true;
  }
  async start(): Promise<void> { this.disposed = false; await Promise.all([this.refreshTargets(), this.refreshJobs()]); }
  setTarget(target: string): void { if (this.available() && !this.state.pending && this.state.targets.some(value => value.id === target)) this.update({ target }); }
  closeReview(): void { if (!this.state.pending) this.update({ reviewOpen: false }); }
  reopenReview(): void { if (this.available() && !this.state.pending && this.submission && this.state.proposal) this.update({ reviewOpen: true }); }
  private invalidateJobs(): void { this.jobsAbort?.abort(); this.jobsAbort = null; this.jobsVersion++; this.jobsRead = null; this.update({ jobs: [], historyError: "", historyLoading: true }); }
  refreshTargets(): Promise<void> {
    if (!this.available() || this.state.pending) return Promise.resolve(); if (this.catalogRead) return this.catalogRead;
    const scope = this.scope();
    const work = async () => {
      if (!scope.active()) { scope.close(); return; }
      try {
        const catalog = await this.api.targets(scope.signal); if (!scope.active()) return;
        if (!catalog || !Array.isArray(catalog.targets) || catalog.targets.length > 32 || !catalog.targets.every(device) || new Set(catalog.targets.map(value => value.id)).size !== catalog.targets.length) throw invalid();
        const target = catalog.targets.some(value => value.id === this.state.target) ? this.state.target : catalog.targets[0]?.id ?? "";
        this.update({ targets: catalog.targets, target, catalogError: "" });
      } catch (error) { if (scope.active() && !this.accessFailure(error)) this.update({ targets: [], target: "", catalogError: message(error) }); }
      finally { scope.close(); if (scope.active()) { this.catalogRead = null; this.update({ catalogLoading: false }); } }
    };
    const pending = Promise.resolve().then(work); this.catalogRead = pending; this.update({ catalogLoading: true, catalogError: "" }); return pending;
  }
  refreshJobs(): Promise<void> {
    if (!this.available() || this.state.pending || this.state.canceling !== null) return Promise.resolve(); if (this.jobsRead) return this.jobsRead;
    const scope = this.scope(), version = ++this.jobsVersion; this.jobsAbort = scope.controller;
    const active = () => scope.active() && version === this.jobsVersion;
    const work = async () => {
      if (!active()) { scope.close(); return; }
      try {
        const jobs = await this.api.jobs(scope.signal); if (!active()) return;
        if (!Array.isArray(jobs) || jobs.length > 32 || !jobs.every(job) || new Set(jobs.map(value => value.id)).size !== jobs.length) throw invalid();
        this.update({ jobs, historyError: "" });
      } catch (error) { if (active() && !this.accessFailure(error)) this.update({ jobs: [], historyError: message(error) }); }
      finally { scope.close(); if (active()) { this.jobsRead = null; this.jobsAbort = null; this.update({ historyLoading: false }); } }
    };
    const pending = Promise.resolve().then(work); this.jobsRead = pending; this.update({ historyLoading: true, historyError: "" }); return pending;
  }
  async preview(): Promise<void> {
    if (!this.available() || this.state.pending || this.state.canceling !== null || !this.state.targets.some(value => value.id === this.state.target)) return;
    const target = this.state.target, scope = this.scope(); this.submission = null;
    this.update({ proposal: null, retryAvailable: false, pending: true, reviewOpen: true, error: "" });
    try {
      if (!scope.active()) return;
      const proposal = await this.api.plan(target, this.csrf, scope.signal); if (!scope.active()) return;
      if (!proposal || !opaque(proposal.digest) || !plan(proposal.plan) || proposal.plan.effect.device.id !== target) throw invalid();
      this.update({ proposal });
    } catch (error) { if (scope.active() && !this.accessFailure(error)) this.update({ error: message(error) }); }
    finally { scope.close(); if (scope.active()) this.update({ pending: false }); }
  }
  async confirm(): Promise<void> {
    const proposal = this.state.proposal;
    if (!this.available() || !proposal || this.state.pending || this.state.canceling !== null) return;
    const scope = this.scope(); this.update({ pending: true, error: "" });
    try {
      if (!scope.active()) return;
      this.invalidateJobs(); if (!scope.active()) return;
      if (!this.submission) {
        const now = this.now(); if (!Number.isFinite(now) || now < proposal.plan.effect.created_at || now >= proposal.plan.effect.expires_at) throw new ApiError({ code: "expired", message: "Expired", retry: false, audit_id: "mount-review" });
        const approval = await this.api.approve(proposal, this.csrf, scope.signal); if (!scope.active()) return;
        if (!approval || !opaque(approval.token) || !Number.isSafeInteger(approval.expires_at) || approval.expires_at <= this.now()) throw invalid();
        this.submission = { proposal, approval: approval.token, key: this.key() };
        this.update({ retryAvailable: true });
      }
      if (!scope.active()) return;
      const saved = this.submission;
      const result = await this.api.queue(saved.proposal, saved.approval, saved.key, this.csrf, scope.signal); if (!scope.active()) return;
      if (!job(result) || result.plan.id !== saved.proposal.plan.id || result.plan.effect.action !== saved.proposal.plan.effect.action) throw invalid();
      this.submission = null; this.update({ proposal: null, retryAvailable: false, reviewOpen: false });
    } catch (error) { if (scope.active() && !this.accessFailure(error)) this.update({ error: message(error) }); }
    finally { scope.close(); if (scope.active()) { this.update({ pending: false }); await this.refreshJobs(); } }
  }
  async cancel(id: string): Promise<void> {
    if (!this.available() || this.state.pending || this.state.canceling !== null || !this.state.jobs.some(value => value.id === id && value.state === "queued")) return;
    const scope = this.scope(); let failure = ""; this.update({ canceling: id, historyError: "" });
    try { if (!scope.active()) return; this.invalidateJobs(); if (!scope.active()) return; await this.api.cancel(id, this.csrf, scope.signal); }
    catch (error) { if (scope.active() && !this.accessFailure(error)) failure = message(error); }
    finally { scope.close(); if (scope.active()) { this.update({ canceling: null }); await this.refreshJobs(); if (scope.active() && failure) this.update({ historyError: `${failure}${this.state.historyError ? ` ${this.state.historyError}` : ""}` }); } }
  }
  dispose(): void { this.disposed = true; this.generation++; this.abortAll(); this.submission = null; this.update(initial()); }
}
