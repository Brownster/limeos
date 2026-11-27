import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type {
  ContainerAction,
  NeedsAttentionJob,
  Resource,
  StackJob,
  StackReceipt,
  EventKind,
} from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import { containFocus, message } from "./operation-ui";
import { mergeAttention, NeedsAttention } from "./ResolutionPanel";
import {
  StackActionController,
  stackTargets,
  type StackActionClient,
} from "./stack-action-controller";
import type { StackView } from "./stack-inventory-client";
import { Icon, type IconName } from "./ui/icons";
import { activeJobStates, jobHeadline, jobOutcomes, shortId } from "./stacks/stack-model";

/**
 * Network failures and 5xx may have reached the host, so the same request is
 * retried. A definite 4xx refusal means the plan or approval is dead.
 */
export function isInterrupted(error: unknown): boolean {
  if (!(error instanceof ApiError)) return true;
  const definite = [
    "invalid_input",
    "unauthenticated",
    "forbidden",
    "conflict",
    "expired",
    "not_found",
  ];
  return (
    !definite.includes(error.detail.code) &&
    !/^http-4\d\d$/.test(error.detail.audit_id)
  );
}
const labels: Record<ContainerAction, string> = {
  restart: "Restart",
  start: "Start",
  stop: "Stop",
};
const actionIcons: Record<ContainerAction, IconName> = { start: "play", stop: "stop", restart: "restart" };
const actions: ContainerAction[] = ["start", "stop", "restart"];
const client: StackActionClient = {
  plan: api.planStack,
  approve: api.approveStack,
  queue: api.queueStack,
};
const noNames = new Map<string, Resource>();

/** Why an action is unavailable, or null. The server decides again at plan time. */
export function stackAvailability(
  stack: StackView,
  stale: boolean,
  action: ContainerAction,
): string | null {
  if (stale) return "Current stack state is unknown.";
  if (stack.issues.length > 0)
    return "Resolve the stack's inventory issue first.";
  if (stack.runtime === "unknown" || stack.runtime === "conflict")
    return "Current stack state is unknown.";
  const counts = stack.counts;
  if (!counts) return "Current stack state is unknown.";
  if (counts.ordinary === 0) return "The stack has no service containers.";
  if (action === "start")
    return counts.running < counts.ordinary
      ? null
      : "Every service container is already running.";
  return counts.running > 0 ? null : "No service container is running.";
}

/** A container named from the overview where it is known, with its short ID. */
function Target({ resource, names }: { resource: string; names: Map<string, Resource> }) {
  const name = names.get(resource)?.name;
  return <>{name && <strong>{name} </strong>}<code>{shortId(resource)}</code></>;
}

/**
 * Whole-stack Start, Stop and Restart. Each previews a durable plan, asks
 * for approval in a dialog and queues one job. A job already in flight for
 * this stack (`busy`) holds the buttons.
 */
export function StackActions({
  stack,
  stale,
  csrf,
  changed,
  names = noNames,
  busy = false,
}: {
  stack: StackView;
  stale: boolean;
  csrf: string;
  changed: () => void;
  /** Overview containers by resource id, to name the containers a plan touches. */
  names?: Map<string, Resource>;
  busy?: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const latest = useRef(changed);
  latest.current = changed;
  const controller = useMemo(
    () =>
      new StackActionController(
        stack.project,
        csrf,
        client,
        () => latest.current(),
        message,
        isInterrupted,
      ),
    [stack.project, csrf],
  );
  useEffect(() => () => controller.dispose(), [controller]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );
  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (state.action && !state.queued) {
      if (!element.open) element.showModal();
    } else if (element.open) element.close();
  }, [state.action, state.queued]);
  const action = state.action ?? "restart";
  const targets = state.proposal ? stackTargets(state.proposal, action) : [];
  const heading = `stack-action-${stack.project}`;
  return (
    <>
      <div className="container-controls stack-controls" role="group" aria-label={`Whole-stack actions for ${stack.stack}`}>
        {actions.map((name) => {
          const reason = busy ? "An operation on this stack is in progress." : stackAvailability(stack, stale, name);
          return (
            <button
              key={name}
              type="button"
              className={`container-action stack-action action-${name}`}
              disabled={state.pending || reason !== null}
              title={reason ?? undefined}
              onClick={() => void controller.preview(name)}
            >
              <Icon name={actionIcons[name]} size={14} />{labels[name]} stack
            </button>
          );
        })}
      </div>
      <dialog
        ref={dialog}
        className="operation-dialog"
        aria-labelledby={heading}
        onClose={() => controller.close()}
        onKeyDown={containFocus}
      >
        <header className="page-heading">
          <h2 id={heading}>
            {labels[action]} stack {stack.stack}
          </h2>
          <button
            className="quiet"
            autoFocus
            onClick={() => controller.close()}
            aria-label={`Close ${action} stack preview`}
          >
            Close
          </button>
        </header>
        <p>
          {action === "start"
            ? "This starts the stopped service containers listed below."
            : action === "stop"
              ? "This stops the running service containers listed below until they are started again."
              : "This restarts the running service containers listed below, one after another, interrupting each briefly."}{" "}
          Containers are changed one at a time by exact ID. No Compose file is
          read or written, and no container is created or removed. One-off
          containers are not touched. If a container fails, later ones are not
          attempted.
        </p>
        {state.pending && !state.proposal && (
          <p role="status">Inspecting the stack…</p>
        )}
        {state.proposal && (
          <>
            <dl className="plan-details">
              <dt>Stack</dt>
              <dd>{stack.project}</dd>
              <dt>Containers to {action}</dt>
              <dd>
                <ul className="plan-targets">
                  {targets.map((member) => (
                    <li key={member.resource}>
                      <Target resource={member.resource} names={names} />{" "}
                      {member.running ? "running" : "not running"}
                    </li>
                  ))}
                </ul>
              </dd>
              <dt>Left unchanged</dt>
              <dd>
                {state.proposal.plan.expected.members.length - targets.length}{" "}
                other service containers
              </dd>
              <dt>Approval expires</dt>
              <dd>
                {new Date(
                  state.proposal.plan.expires_at * 1000,
                ).toLocaleTimeString()}
              </dd>
            </dl>
          </>
        )}
        {state.error && (
          <p className="error" role="alert">
            {state.error}
          </p>
        )}
        {state.error && !state.proposal && !state.pending && state.action && (
          <p className="notice">
            This request was refused and cannot be repeated. Close this dialog
            and start again to preview a fresh plan.
          </p>
        )}
        {state.retained && state.interrupted && (
          <p className="notice">
            The response was interrupted. Check recent operations or retry this
            same request; it retains its original request key.
          </p>
        )}
        <button
          disabled={!state.proposal || state.pending}
          onClick={() => void controller.confirm()}
        >
          {state.pending
            ? "Submitting…"
            : state.retained
              ? "Retry same request"
              : `Approve and ${action} stack`}
        </button>
      </dialog>
    </>
  );
}

const outcome = (state: string) => state.replaceAll("_", " ");
/** Per-member outcomes recorded by the executor; absent until the job runs. */
export function StackReceiptView({ receipt, names = noNames }: { receipt: StackReceipt; names?: Map<string, Resource> }) {
  return (
    <ul className="job-events" aria-label="Per-container outcomes">
      {receipt.members.map((member) => (
        <li key={member.resource}>
          {names.get(member.resource) && <>{names.get(member.resource)!.name} </>}
          <code>{member.resource.slice(10, 22)}</code> ·{" "}
          {member.state === "precondition_changed"
            ? "not attempted"
            : outcome(member.state)}
          {member.error ? ` (${member.error.replaceAll("_", " ")})` : ""}
        </li>
      ))}
    </ul>
  );
}
/** Verified members of a partly-done run, out of all acted-on members. */
export function partlyDone(receipt?: StackReceipt | null) {
  if (!receipt || receipt.state !== "partly_done") return null;
  return {
    done: receipt.members.filter((m) => m.state === "verified").length,
    total: receipt.members.length,
  };
}
export function StackJobRow({
  job,
  receipt,
  labels: eventLabels,
  cancel,
  details,
  names = noNames,
}: {
  job: StackJob;
  receipt?: StackReceipt | null;
  labels?: string[];
  cancel: () => void;
  details: () => void;
  names?: Map<string, Resource>;
}) {
  return (
    <article className="job-row">
      <div>
        <strong>
          {labels[job.plan.operation]} stack ·{" "}
          {job.plan.expected.resource.slice(6)}
        </strong>
        <p role="status">
          {partlyDone(receipt)
            ? `Partly done, ${partlyDone(receipt)!.done} of ${partlyDone(receipt)!.total} members. Not a success: the rest were not changed.`
            : outcome(job.state)}
        </p>
        {job.state === "needs_intervention" && (
          <p className="notice">
            The outcome needs inspection. The stack and its containers remain
            locked and the operation will not be repeated.
          </p>
        )}
      </div>
      <button className="quiet" onClick={details}>
        Progress
      </button>
      {job.state === "queued" && (
        <button className="quiet" onClick={cancel}>
          Cancel queued {job.plan.operation} stack
        </button>
      )}
      {eventLabels && (
        <ol className="job-events">
          {eventLabels.map((label, i) => (
            <li key={i}>{label}</li>
          ))}
        </ol>
      )}
      {receipt && <StackReceiptView receipt={receipt} names={names} />}
    </article>
  );
}

export interface StackJobsData {
  jobs: StackJob[];
  /** Other principals' stuck jobs, for administrators only. */
  listed: NeedsAttentionJob[];
  /** Per-member receipts by job id, where the job reported one. */
  receipts: Record<string, StackReceipt | null>;
  error: string;
  shown: { id: string; labels: string[]; receipt: StackReceipt | null } | null;
  cancel: (id: string) => void;
  details: (id: string) => void;
}

/**
 * The signed-in principal's stack jobs, read while the Stacks page is open
 * and polled every 2 s only while one is queued or running. Receipts are read
 * every poll for a job in flight (so its card shows per-container progress),
 * and once for a failed job or one finished in the last 15 minutes.
 */
export function useStackJobs({
  csrf,
  revision,
  expired,
  admin = false,
  enabled = true,
  recentSince = 0,
}: {
  csrf: string;
  revision: number;
  expired: () => void;
  admin?: boolean;
  enabled?: boolean;
  /** Finished jobs planned after this time (epoch seconds) get their receipt once. */
  recentSince?: number;
}): StackJobsData {
  const [jobs, setJobs] = useState<StackJob[]>([]);
  const [listed, setListed] = useState<NeedsAttentionJob[]>([]);
  const [receipts, setReceipts] = useState<Record<string, StackReceipt | null>>({});
  const requested = useRef(new Set<string>());
  const [error, setError] = useState("");
  const [shown, setShown] = useState<StackJobsData["shown"]>(null);
  const since = useRef(recentSince);
  since.current = recentSince;
  useEffect(() => {
    if (!enabled) return;
    let active = true,
      inFlight = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const receipt = (id: string) =>
      api
        .stackProgress(id)
        .then((p) => {
          if (active) setReceipts((all) => ({ ...all, [id]: p.receipt }));
        })
        .catch(() => requested.current.delete(id));
    async function refresh() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await api.stackJobs();
        if (!active) return;
        setJobs(next);
        // Read-only and best effort: the banner falls back to the own jobs.
        if (admin)
          api
            .needsAttention()
            .then((all) => active && setListed(all))
            .catch(() => active && setListed([]));
        setError("");
        for (const job of next) {
          if (activeJobStates.includes(job.state)) {
            void receipt(job.id);
            continue;
          }
          const recent = job.plan.created_at >= since.current;
          if ((job.state !== "failed" && !recent) || requested.current.has(job.id)) continue;
          requested.current.add(job.id);
          void receipt(job.id);
        }
        if (next.some((job) => activeJobStates.includes(job.state)))
          timer = setTimeout(() => void refresh(), 2000);
      } catch (e) {
        if (active) {
          if (e instanceof ApiError && e.detail.code === "unauthenticated")
            expired();
          else setError(message(e));
        }
      } finally {
        inFlight = false;
      }
    }
    void refresh();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [revision, expired, admin, enabled]);
  const cancel = useCallback(async (id: string) => {
    try {
      await api.cancelStack(id, csrf);
      setJobs(await api.stackJobs());
    } catch (e) {
      setError(message(e));
    }
  }, [csrf]);
  const details = useCallback(async (id: string) => {
    try {
      const progress = await api.stackProgress(id);
      setShown({
        id,
        receipt: progress.receipt,
        labels: progress.events.map((e) => eventLabel(e.event)),
      });
    } catch (e) {
      setError(message(e));
    }
  }, []);
  return {
    jobs, listed, receipts, error, shown,
    cancel: (id) => void cancel(id),
    details: (id) => void details(id),
  };
}

/** A stack's job in flight (or just finished) on its card, with per-container progress. */
export function StackJobStrip({
  job,
  receipt,
  names = noNames,
}: {
  job: StackJob;
  receipt?: StackReceipt | null;
  names?: Map<string, Resource>;
}) {
  const headline = jobHeadline(job, receipt);
  const rows = jobOutcomes(job, receipt, names);
  const inFlight = activeJobStates.includes(job.state);
  return (
    <div className={`stack-job tone-${headline.tone}`} role="status" aria-label={`Operation on ${job.plan.catalog_stack}`}>
      <p className="stack-job-head">
        <Icon name={inFlight ? "clock" : headline.tone === "success" ? "ok" : headline.tone === "danger" ? "warning" : "info"} size={15} className={inFlight ? "pulse" : undefined} />
        <span>{headline.text}</span>
      </p>
      {rows.length > 0 && (
        <ul className="stack-job-members" aria-label="Containers in this operation">
          {rows.map((row) => (
            <li key={row.resource}>
              <span className="member-name">{row.label}</span>
              <span className={`tone-text-${row.tone === "neutral" ? "neutral" : row.tone} member-outcome`}>{row.outcome}</span>
            </li>
          ))}
        </ul>
      )}
      {job.state === "needs_intervention" && (
        <p className="stack-job-note">The stack stays locked until an administrator settles this operation (see Recent stack operations).</p>
      )}
      {job.state === "queued" && (
        <p className="stack-job-note">It can still be canceled under Recent stack operations.</p>
      )}
    </div>
  );
}

/** "Recent stack operations": the job list and anything needing intervention. */
export function StackJobs({
  data,
  csrf,
  expired,
  admin = false,
  changed = () => {},
  names = noNames,
}: {
  data: StackJobsData;
  csrf: string;
  expired: () => void;
  /** Only administrators may resolve; others see the banner as information. */
  admin?: boolean;
  changed?: () => void;
  names?: Map<string, Resource>;
}) {
  const { jobs, listed, receipts, error, shown } = data;
  const attention = mergeAttention(
    jobs
      .filter((job) => job.state === "needs_intervention")
      .map((job) => ({
        id: job.id,
        title: `${labels[job.plan.operation]} stack · ${job.plan.expected.resource.slice(6)}`,
      })),
    listed,
    "stack",
  );
  if (!jobs.length && !attention.length && !error) return null;
  return (
    <section className="operations stack-operations" aria-labelledby="stack-operations-heading">
      <h3 id="stack-operations-heading">Recent stack operations</h3>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <NeedsAttention
        jobs={attention}
        admin={admin}
        csrf={csrf}
        expired={expired}
        changed={changed}
      />
      {jobs.slice(0, 8).map((job) => (
        <StackJobRow
          key={job.id}
          job={job}
          names={names}
          labels={shown?.id === job.id ? shown.labels : undefined}
          receipt={shown?.id === job.id ? shown.receipt : receipts[job.id]}
          cancel={() => data.cancel(job.id)}
          details={() => data.details(job.id)}
        />
      ))}
    </section>
  );
}

/** One line of a job's history: its state, or what happened to it. */
function eventLabel(event: EventKind): string {
  if (event.kind === "job_transition") return event.state.replaceAll("_", " ");
  if (event.kind === "executor_budget_spent") {
    const until =
      event.frees_at === undefined
        ? ""
        : ` until ${new Date(event.frees_at * 1000).toLocaleString()}`;
    return `refused: the effect budget of a container it acts on is spent${until}`;
  }
  return event.kind.replaceAll("_", " ");
}
