/**
 * Operator resolution of a needs_intervention job (docs/p06-job-resolution.md).
 * Read fresh evidence, let the administrator choose a declaration, confirm it
 * bound to the evidence digest, then submit with the administrator's password.
 * Resolution is bookkeeping: nothing here starts, stops or restarts anything.
 * The password is passed to submit() and never stored; a POST is never repeated
 * automatically.
 */
import type {
  DeclaredOutcome,
  JobResolution,
  ResolutionView,
  ResolveJobInput,
  ResolveJobResponse,
  ResolveStatus,
} from "../../contracts/generated/types";

/** An HTTP answer with its parsed JSON body (null when it was not JSON). */
export interface RawResponse {
  status: number;
  body: unknown;
}
export interface ResolutionClient {
  read(job: string): Promise<RawResponse>;
  submit(job: string, input: ResolveJobInput, csrf: string): Promise<RawResponse>;
  /** Whether the session cookie still works; "unknown" when unreachable. */
  session(): Promise<"valid" | "expired" | "unknown">;
}
export type Step = "choose" | "confirm";
export interface ResolutionState {
  view: ResolutionView | null;
  loading: boolean;
  submitting: boolean;
  declared: DeclaredOutcome | null;
  reason: string;
  step: Step;
  /** Client-side validation message for the current choice. */
  invalid: string;
  error: string;
  /** Non-error news, e.g. that the state changed since it was shown. */
  notice: string;
  /** Container IDs that block an unsupported declaration. */
  blocking: string[];
  result: "resolved" | "pending" | null;
  resolution: JobResolution | null;
  /** The last POST outcome is unknown (network); repeating is safe. */
  interrupted: boolean;
  /** The session ended; the caller has been told. */
  expired: boolean;
}
const idle: ResolutionState = {
  view: null,
  loading: false,
  submitting: false,
  declared: null,
  reason: "",
  step: "choose",
  invalid: "",
  error: "",
  notice: "",
  blocking: [],
  result: null,
  resolution: null,
  interrupted: false,
  expired: false,
};

export const declarations: DeclaredOutcome[] = [
  "verified_done",
  "not_done",
  "accepted_divergence",
];
export const declarationLabels: Record<DeclaredOutcome, string> = {
  verified_done: "Intended end state is present",
  not_done: "Nothing changed — release the lock only",
  accepted_divergence: "Accept the present state as it is",
};
export const MAX_REASON = 500;
export const MIN_DIVERGENCE_REASON = 20;
const jobId = /^[0-9a-f]{64}$/;
export const isJobId = (value: string) => jobId.test(value);

/** Mirrors the server: 1..=500 characters (20 for divergence), no control characters, no surrounding whitespace. */
export function validateReason(
  declared: DeclaredOutcome,
  reason: string,
): string {
  const length = [...reason].length;
  if (length === 0) return "Write what you found and why this is acceptable.";
  if (reason !== reason.trim())
    return "The reason cannot start or end with spaces or line breaks.";
  if (/\p{Cc}/u.test(reason))
    return "The reason cannot contain control characters or line breaks.";
  if (length > MAX_REASON)
    return `The reason is limited to ${MAX_REASON} characters (${length} written).`;
  if (declared === "accepted_divergence" && length < MIN_DIVERGENCE_REASON)
    return `Accepting a divergence needs a reason of at least ${MIN_DIVERGENCE_REASON} characters (${length} written).`;
  return "";
}
/** Why a declaration is unavailable for this evidence, or null when allowed. */
export function disabledReason(
  view: ResolutionView,
  declared: DeclaredOutcome,
): string | null {
  if (view.allowed.includes(declared)) return null;
  const short = (ids: string[]) => ids.map((id) => id.slice(0, 12)).join(", ");
  if (declared === "verified_done")
    return `Not available: ${short(view.targets.filter((t) => !t.measured.intended_state_reached).map((t) => t.measured.container))} is not in the intended end state.`;
  if (declared === "not_done")
    return `Not available: ${short(view.targets.filter((t) => !t.measured.matches_expected).map((t) => t.measured.container))} differs from the plan.`;
  return "Not available: this job is not awaiting resolution.";
}

const statuses: ResolveStatus[] = [
  "resolved",
  "pending",
  "stale_evidence",
  "unsupported_declaration",
];
const record = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === "object" ? (value as Record<string, unknown>) : null;
export function asView(body: unknown): ResolutionView | null {
  const v = record(body);
  return v &&
    typeof v.job === "string" &&
    typeof v.state === "string" &&
    Array.isArray(v.targets) &&
    Array.isArray(v.allowed)
    ? (body as ResolutionView)
    : null;
}
function asResponse(body: unknown): ResolveJobResponse | null {
  const r = record(body);
  return r && statuses.includes(r.status as ResolveStatus) && typeof r.message === "string"
    ? (body as ResolveJobResponse)
    : null;
}
/** The server's message plus audit reference from an ErrorEnvelope. */
function envelope(body: unknown, fallback: string): string {
  const e = record(body);
  if (!e || typeof e.message !== "string" || !e.message) return fallback;
  return typeof e.audit_id === "string"
    ? `${e.message} · reference ${e.audit_id}`
    : e.message;
}

export class ResolutionController {
  private state: ResolutionState = idle;
  private listeners = new Set<() => void>();
  private serial = 0;
  private alive = true;
  constructor(
    private readonly job: string,
    private readonly csrf: string,
    private readonly client: ResolutionClient,
    /** The job list should be refreshed. */
    private readonly changed: () => void,
    /** The session is gone. */
    private readonly expire: () => void,
    /** Pause between GET retries. */
    private readonly sleep: (ms: number) => Promise<void> = (ms) =>
      new Promise((resolve) => setTimeout(resolve, ms)),
    private readonly attempts = 3,
  ) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.state;
  private set(patch: Partial<ResolutionState>) {
    if (!this.alive) return;
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  dispose() {
    this.alive = false;
    this.serial += 1;
    this.listeners.clear();
  }
  /** Fetch fresh evidence. Only reads are retried, and only on network or 5xx failure. */
  async load() {
    const mine = ++this.serial;
    this.set({ loading: true });
    let last = "";
    for (let attempt = 0; attempt < this.attempts; attempt += 1) {
      if (attempt > 0) await this.sleep(500 * attempt);
      if (mine !== this.serial) return;
      try {
        const raw = await this.client.read(this.job);
        if (mine !== this.serial) return;
        if (raw.status >= 500) {
          last =
            raw.status === 503
              ? "The evidence could not be gathered right now. Try again shortly."
              : envelope(raw.body, "The host could not complete the request.");
          continue;
        }
        const view = raw.status === 200 ? asView(raw.body) : null;
        if (view) {
          this.set({
            view,
            loading: false,
            error: "",
            declared:
              this.state.declared && view.allowed.includes(this.state.declared)
                ? this.state.declared
                : null,
          });
          return;
        }
        if (raw.status === 401) {
          this.lapse();
          return;
        }
        this.set({
          loading: false,
          error:
            raw.status === 403
              ? "Only an administrator can review this job."
              : raw.status === 404
                ? "This job no longer exists."
                : envelope(raw.body, "The host sent an unexpected answer."),
        });
        return;
      } catch {
        last = "The host is unavailable.";
      }
    }
    if (mine === this.serial) this.set({ loading: false, error: last });
  }
  private lapse() {
    this.set({
      loading: false,
      submitting: false,
      expired: true,
      error: "Your session ended. Sign in again.",
    });
    this.expire();
  }
  choose(declared: DeclaredOutcome) {
    const view = this.state.view;
    if (!view || this.state.submitting || !view.allowed.includes(declared))
      return;
    this.set({ declared, invalid: "", step: "choose", result: null });
  }
  setReason(reason: string) {
    this.set({ reason, invalid: "" });
  }
  /** Validate and show the confirmation summary; sends nothing. */
  review() {
    const { view, declared, reason } = this.state;
    if (!view?.evidence_digest || !declared) {
      this.set({ invalid: "Choose one of the outcomes first." });
      return;
    }
    const invalid = validateReason(declared, reason);
    this.set(
      invalid
        ? { invalid }
        : { invalid: "", step: "confirm", error: "", notice: "", result: null },
    );
  }
  back() {
    if (this.state.submitting) return;
    this.set({ step: "choose", result: null, interrupted: false, error: "" });
  }
  /**
   * Send the decision. The password is used for this one request only. A
   * pending answer may be submitted again as is; nothing is ever re-sent
   * without the administrator pressing the button.
   */
  async submit(password: string) {
    const { view, declared, reason, step, submitting, result } = this.state;
    if (submitting || step !== "confirm" || result === "resolved") return;
    if (!view?.evidence_digest || !declared) return;
    if (validateReason(declared, reason)) return;
    if (!password) {
      this.set({ error: "Enter your password to confirm." });
      return;
    }
    const mine = ++this.serial;
    this.set({ submitting: true, error: "", notice: "", interrupted: false });
    let raw: RawResponse;
    try {
      raw = await this.client.submit(
        this.job,
        { declared, reason, evidence_digest: view.evidence_digest, password },
        this.csrf,
      );
    } catch {
      this.set({
        submitting: false,
        interrupted: true,
        error:
          "The answer was lost. The decision may have been recorded: refresh to check before submitting again.",
      });
      this.changed();
      return;
    }
    if (mine !== this.serial) return;
    await this.settle(raw);
  }
  private async settle(raw: RawResponse) {
    const response = asResponse(raw.body);
    switch (raw.status) {
      case 200:
        if (response?.status === "resolved") {
          this.set({
            submitting: false,
            result: "resolved",
            resolution: response.resolution,
            notice: response.message,
          });
          this.changed();
          return;
        }
        break;
      case 202:
        if (response?.status === "pending") {
          this.set({
            submitting: false,
            result: "pending",
            notice:
              "Recorded, waiting for the executor. The job stays locked until it completes. Submitting the same decision again is safe.",
          });
          this.changed();
          return;
        }
        break;
      case 409:
        if (response?.status === "stale_evidence") {
          const view = response.view;
          this.set({
            submitting: false,
            step: "choose",
            declared:
              view && this.state.declared && view.allowed.includes(this.state.declared)
                ? this.state.declared
                : null,
            view: view ?? this.state.view,
            blocking: [],
            notice:
              "The state changed since you reviewed it. The evidence below is current; check it and decide again.",
          });
          if (!view) await this.load();
          return;
        }
        if (response?.status === "unsupported_declaration") {
          this.set({
            submitting: false,
            step: "choose",
            declared: null,
            view: response.view ?? this.state.view,
            blocking: response.blocking,
            error: `The evidence does not support that outcome. ${response.message}`,
          });
          return;
        }
        {
          const why = envelope(raw.body, "Another request is open for this job, or it is no longer awaiting resolution.");
          this.set({ submitting: false, step: "choose" });
          await this.load();
          this.set({ error: why });
        }
        return;
      case 401: {
        const session = await this.client.session();
        if (session === "expired") {
          this.lapse();
          return;
        }
        this.set({
          submitting: false,
          error:
            session === "valid"
              ? "The password was not accepted. Nothing was recorded."
              : "Authentication failed and the session could not be checked. Nothing was recorded.",
        });
        return;
      }
      case 403:
        this.set({
          submitting: false,
          error:
            "Refused. Only an administrator on a signed-in session can resolve a job. " +
            envelope(raw.body, ""),
        });
        return;
      case 429:
        this.set({
          submitting: false,
          error:
            "Too many password attempts. Wait a few minutes before trying again. Nothing was recorded.",
        });
        return;
      case 503:
        this.set({
          submitting: false,
          error:
            "The evidence could not be gathered, so nothing was recorded. Try again shortly.",
        });
        return;
      default:
    }
    this.set({
      submitting: false,
      interrupted: raw.status >= 500,
      error:
        raw.status >= 500
          ? `The host could not complete the request. ${envelope(raw.body, "")} It may have been recorded: refresh to check before submitting again.`
          : envelope(raw.body, "The request was refused."),
    });
  }
}
