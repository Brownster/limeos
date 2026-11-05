import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type {
  ContainerAction,
  StackJob,
  StackReceipt,
} from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import { containFocus, message } from "./operation-ui";
import {
  StackActionController,
  stackTargets,
  type StackActionClient,
} from "./stack-action-controller";
import type { StackView } from "./stack-inventory-client";

const labels: Record<ContainerAction, string> = {
  restart: "Restart",
  start: "Start",
  stop: "Stop",
};
const actions: ContainerAction[] = ["start", "stop", "restart"];
const client: StackActionClient = {
  plan: api.planStack,
  approve: api.approveStack,
  queue: api.queueStack,
};

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

export function StackActions({
  stack,
  stale,
  csrf,
  changed,
}: {
  stack: StackView;
  stale: boolean;
  csrf: string;
  changed: () => void;
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
      <div className="container-controls" role="group" aria-label={`Whole-stack actions for ${stack.stack}`}>
        {actions.map((name) => {
          const reason = stackAvailability(stack, stale, name);
          return (
            <button
              key={name}
              className="container-action"
              disabled={state.pending || reason !== null}
              title={reason ?? undefined}
              onClick={() => void controller.preview(name)}
            >
              {labels[name]} stack
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
                <ul>
                  {targets.map((member) => (
                    <li key={member.resource}>
                      <code>{member.resource.slice(10, 22)}</code>{" "}
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
        {state.retained && (
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
export function StackReceiptView({ receipt }: { receipt: StackReceipt }) {
  return (
    <ul className="job-events" aria-label="Per-container outcomes">
      {receipt.members.map((member) => (
        <li key={member.resource}>
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
export function StackJobRow({
  job,
  receipt,
  labels: eventLabels,
  cancel,
  details,
}: {
  job: StackJob;
  receipt?: StackReceipt | null;
  labels?: string[];
  cancel: () => void;
  details: () => void;
}) {
  return (
    <article className="job-row">
      <div>
        <strong>
          {labels[job.plan.operation]} stack ·{" "}
          {job.plan.expected.resource.slice(6)}
        </strong>
        <p role="status">{outcome(job.state)}</p>
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
      {receipt && <StackReceiptView receipt={receipt} />}
    </article>
  );
}
export function StackJobs({
  csrf,
  revision,
  expired,
}: {
  csrf: string;
  revision: number;
  expired: () => void;
}) {
  const [jobs, setJobs] = useState<StackJob[]>([]);
  const [error, setError] = useState("");
  const [shown, setShown] = useState<{
    id: string;
    labels: string[];
    receipt: StackReceipt | null;
  } | null>(null);
  useEffect(() => {
    let active = true,
      inFlight = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await api.stackJobs();
        if (!active) return;
        setJobs(next);
        setError("");
        if (
          next.some((job) =>
            ["queued", "running", "verifying"].includes(job.state),
          )
        )
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
  }, [revision, expired]);
  async function cancel(id: string) {
    try {
      await api.cancelStack(id, csrf);
      setJobs(await api.stackJobs());
    } catch (e) {
      setError(message(e));
    }
  }
  async function details(id: string) {
    try {
      const progress = await api.stackProgress(id);
      setShown({
        id,
        receipt: progress.receipt,
        labels: progress.events.map((e) =>
          e.event.kind === "job_transition"
            ? e.event.state.replaceAll("_", " ")
            : e.event.kind.replaceAll("_", " "),
        ),
      });
    } catch (e) {
      setError(message(e));
    }
  }
  if (!jobs.length && !error) return null;
  return (
    <section className="operations" aria-labelledby="stack-operations-heading">
      <h3 id="stack-operations-heading">Recent stack operations</h3>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {jobs.slice(0, 8).map((job) => (
        <StackJobRow
          key={job.id}
          job={job}
          labels={shown?.id === job.id ? shown.labels : undefined}
          receipt={shown?.id === job.id ? shown.receipt : undefined}
          cancel={() => void cancel(job.id)}
          details={() => void details(job.id)}
        />
      ))}
    </section>
  );
}
