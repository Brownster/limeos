/**
 * Whole-stack start/stop/restart flow: preview a durable plan, approve it as a
 * separate human step, then queue with one request key that is retained across
 * interrupted responses. Mirrors the single-container dialog flow.
 */
import type {
  ContainerAction,
  PlanApproval,
  PlannedStackAction,
  StackJob,
} from "../../contracts/generated/types";

export interface StackActionClient {
  plan(
    project: string,
    action: ContainerAction,
    csrf: string,
  ): Promise<PlannedStackAction>;
  approve(proposal: PlannedStackAction, csrf: string): Promise<PlanApproval>;
  queue(
    proposal: PlannedStackAction,
    approval: string,
    key: string,
    csrf: string,
  ): Promise<StackJob>;
}
export interface StackActionState {
  action: ContainerAction | null;
  proposal: PlannedStackAction | null;
  pending: boolean;
  error: string;
  /** An approval exists and the same request will be retried. */
  retained: boolean;
  /** The last failure may have reached the host (network, 5xx); retry is safe. */
  interrupted: boolean;
  /** The job was queued; the dialog can close. */
  queued: boolean;
}
const idle: StackActionState = {
  action: null,
  proposal: null,
  pending: false,
  error: "",
  retained: false,
  interrupted: false,
  queued: false,
};

/** Start acts on stopped members; stop and restart on running members. */
export function stackTargets(
  proposal: PlannedStackAction,
  action: ContainerAction,
) {
  return proposal.plan.expected.members.filter(
    (member) => member.running !== (action === "start"),
  );
}

export class StackActionController {
  private state: StackActionState = idle;
  private listeners = new Set<() => void>();
  private submission: {
    proposal: PlannedStackAction;
    approval: string;
    key: string;
  } | null = null;
  private serial = 0;
  private alive = true;
  constructor(
    private readonly project: string,
    private readonly csrf: string,
    private readonly client: StackActionClient,
    private readonly changed: () => void,
    private readonly describe: (error: unknown) => string,
    /** True for network/5xx failures; a definite 4xx refusal is not retried. */
    private readonly isInterrupted: (error: unknown) => boolean = () => true,
    private readonly newKey: () => string = () => crypto.randomUUID(),
  ) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  getSnapshot = () => this.state;
  private set(patch: Partial<StackActionState>) {
    if (!this.alive) return;
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  dispose() {
    this.alive = false;
    this.serial += 1;
    this.listeners.clear();
  }
  close() {
    this.serial += 1;
    this.submission = null;
    this.set({ ...idle });
  }
  async preview(action: ContainerAction) {
    const mine = ++this.serial;
    this.submission = null;
    this.set({ ...idle, action, pending: true });
    try {
      const next = await this.client.plan(this.project, action, this.csrf);
      if (
        next.plan.expected.resource !== `stack:${this.project}` ||
        next.plan.operation !== action
      )
        throw new Error("Plan identity mismatch");
      if (mine === this.serial) this.set({ proposal: next });
    } catch (e) {
      if (mine === this.serial) this.set({ error: this.describe(e) });
    } finally {
      if (mine === this.serial) this.set({ pending: false });
    }
  }
  async confirm() {
    const { proposal, pending } = this.state;
    if (!proposal || pending) return;
    const mine = this.serial;
    this.set({ pending: true, error: "" });
    try {
      if (!this.submission) {
        const approval = await this.client.approve(proposal, this.csrf);
        this.submission = {
          proposal,
          approval: approval.token,
          key: this.newKey(),
        };
        this.set({ retained: true });
      }
      const { proposal: approved, approval, key } = this.submission;
      await this.client.queue(approved, approval, key, this.csrf);
      this.submission = null;
      this.changed();
      if (mine === this.serial) this.set({ queued: true, retained: false, interrupted: false });
    } catch (e) {
      if (this.isInterrupted(e)) {
        if (mine === this.serial)
          this.set({ error: this.describe(e), interrupted: true });
      } else {
        // A definite refusal: the request cannot be repeated. Start again.
        this.submission = null;
        if (mine === this.serial)
          this.set({
            error: this.describe(e),
            interrupted: false,
            retained: false,
            proposal: null,
          });
      }
      this.changed();
    } finally {
      if (mine === this.serial) this.set({ pending: false });
    }
  }
}
