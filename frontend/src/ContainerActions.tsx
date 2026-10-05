// Ported from pi-health@80593 container-list's Restart control and pending label.
// The action now previews a native plan and queues a separately approved job.
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type {
  PlannedRestart,
  Resource,
  RestartJob,
  ContainerAction,
  ContainerLogs,
} from "../../contracts/generated/types";
import { api, ApiError } from "./api";

function message(error: unknown): string {
  return error instanceof ApiError
    ? `${error.message} · reference ${error.detail.audit_id}`
    : "The host is unavailable. Check recent operations before trying again.";
}
const labels: Record<ContainerAction, string> = {
  restart: "Restart",
  start: "Start",
  stop: "Stop",
};
function containFocus(event: KeyboardEvent<HTMLDialogElement>) {
  if (event.key !== "Tab") return;
  const controls = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      "button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex='0']",
    ),
  ).filter((element) => element.getClientRects().length > 0);
  const first = controls[0],
    last = controls.at(-1);
  if (!first || !last) {
    event.preventDefault();
    event.currentTarget.focus();
    return;
  }
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}
export function ContainerActions({
  resource,
  csrf,
  changed,
}: {
  resource: Resource;
  csrf: string;
  changed: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [proposal, setProposal] = useState<PlannedRestart | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [operation, setOperation] = useState<ContainerAction>("restart");
  const submission = useRef<{
    proposal: PlannedRestart;
    approval: string;
    key: string;
  } | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  async function preview(action: ContainerAction) {
    setPending(true);
    setError("");
    setProposal(null);
    submission.current = null;
    setOperation(action);
    dialog.current?.showModal();
    try {
      const next = await api.planContainer(resource.id, action, csrf);
      if (
        next.plan.expected.resource !== resource.id ||
        (next.plan.operation ?? "restart") !== action
      )
        throw new Error("Plan identity mismatch");
      if (alive.current) setProposal(next);
    } catch (e) {
      if (alive.current) setError(message(e));
    } finally {
      if (alive.current) setPending(false);
    }
  }
  async function confirm() {
    if (!proposal || pending) return;
    setPending(true);
    setError("");
    try {
      if (!submission.current) {
        const approval = await api.approveRestart(proposal, csrf);
        submission.current = {
          proposal,
          approval: approval.token,
          key: crypto.randomUUID(),
        };
      }
      const { proposal: approved, approval, key } = submission.current;
      await api.queueRestart(approved, approval, key, csrf);
      submission.current = null;
      changed();
      if (alive.current) dialog.current?.close();
    } catch (e) {
      if (alive.current) setError(message(e));
      changed();
    } finally {
      if (alive.current) setPending(false);
    }
  }
  return (
    <>
      <div className="container-controls">
        {(["start", "stop", "restart"] as ContainerAction[]).map((action) => (
          <button
            key={action}
            className="container-action"
            disabled={
              pending ||
              (action === "start"
                ? !["exited", "created", "stopped"].includes(resource.status)
                : resource.status !== "running")
            }
            onClick={() => void preview(action)}
          >
            {labels[action]}
          </button>
        ))}
        <ContainerLogView resource={resource} />
      </div>
      <dialog
        ref={dialog}
        className="operation-dialog"
        aria-labelledby={`restart-${resource.id}`}
        onClose={() => setError("")}
        onKeyDown={containFocus}
      >
        <header className="page-heading">
          <h2 id={`restart-${resource.id}`}>
            {labels[operation]} {resource.name}
          </h2>
          <button
            className="quiet"
            autoFocus
            onClick={() => dialog.current?.close()}
            aria-label={`Close ${operation} preview`}
          >
            Close
          </button>
        </header>
        <p>
          {operation === "start"
            ? "This starts the selected service."
            : operation === "stop"
              ? "This stops the selected service until it is started again."
              : "This briefly interrupts the service."}{" "}
          Review the selected container before approving.
        </p>
        {pending && !proposal && <p role="status">Inspecting the container…</p>}
        {proposal && (
          <dl className="plan-details">
            <dt>Container</dt>
            <dd>{proposal.plan.expected.resource.slice(10)}</dd>
            <dt>Image</dt>
            <dd>{proposal.plan.expected.image}</dd>
            <dt>Started</dt>
            <dd>{proposal.plan.expected.started_at}</dd>
            <dt>Approval expires</dt>
            <dd>
              {new Date(proposal.plan.expires_at * 1000).toLocaleTimeString()}
            </dd>
          </dl>
        )}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        {submission.current && (
          <p className="notice">
            The response was interrupted. Check recent operations or retry this
            same request; it retains its original request key.
          </p>
        )}
        <button disabled={!proposal || pending} onClick={() => void confirm()}>
          {pending
            ? "Submitting…"
            : submission.current
              ? "Retry same request"
              : `Approve and ${operation}`}
        </button>
      </dialog>
    </>
  );
}
function ContainerLogView({ resource }: { resource: Resource }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [logs, setLogs] = useState<ContainerLogs | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  async function refresh() {
    if (pending) return;
    setPending(true);
    setError("");
    setLogs(null);
    try {
      const value = await api.containerLogs(resource.id);
      if (value.resource !== resource.id)
        throw new Error("Log identity mismatch");
      if (alive.current) setLogs(value);
    } catch (e) {
      if (alive.current) setError(message(e));
    } finally {
      if (alive.current) setPending(false);
    }
  }
  return (
    <>
      <button
        className="quiet"
        onClick={() => {
          dialog.current?.showModal();
          void refresh();
        }}
      >
        Logs
      </button>
      <dialog
        ref={dialog}
        className="operation-dialog log-dialog"
        aria-labelledby={`logs-${resource.id}`}
        onKeyDown={containFocus}
      >
        <header className="page-heading">
          <h2 id={`logs-${resource.id}`}>Logs · {resource.name}</h2>
          <button
            autoFocus
            className="quiet"
            onClick={() => dialog.current?.close()}
          >
            Close logs
          </button>
        </header>
        <p>
          Last 100 lines. Common credentials and terminal controls are filtered.
        </p>
        {pending && <p role="status">Reading logs…</p>}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        {logs && (
          <>
            {logs.truncated && (
              <p className="notice">Output reached the size limit.</p>
            )}
            <pre className="container-logs">
              {logs.text || "No log output."}
            </pre>
          </>
        )}
        <button disabled={pending} onClick={() => void refresh()}>
          Refresh logs
        </button>
      </dialog>
    </>
  );
}
export function ContainerJobs({
  csrf,
  revision,
  expired,
}: {
  csrf: string;
  revision: number;
  expired: () => void;
}) {
  const [jobs, setJobs] = useState<RestartJob[]>([]);
  const [error, setError] = useState("");
  const [events, setEvents] = useState<{ id: string; labels: string[] } | null>(
    null,
  );
  useEffect(() => {
    let active = true,
      inFlight = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await api.restartJobs();
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
      await api.cancelRestart(id, csrf);
      setJobs(await api.restartJobs());
    } catch (e) {
      setError(message(e));
    }
  }
  async function details(id: string) {
    try {
      const progress = await api.restartProgress(id);
      setEvents({
        id,
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
    <section className="operations" aria-labelledby="operations-heading">
      <h2 id="operations-heading">Recent container operations</h2>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {jobs.slice(0, 8).map((job) => (
        <article key={job.id} className="job-row">
          <div>
            <strong>
              {labels[job.plan.operation ?? "restart"]} ·{" "}
              {job.plan.expected.resource.slice(10, 22)}
            </strong>
            <p role="status">{job.state.replaceAll("_", " ")}</p>
            {job.state === "needs_intervention" && (
              <p className="notice">
                The outcome needs inspection. The resource remains locked and
                the operation will not be repeated.
              </p>
            )}
          </div>
          <button className="quiet" onClick={() => void details(job.id)}>
            Progress
          </button>
          {job.state === "queued" && (
            <button className="quiet" onClick={() => void cancel(job.id)}>
              Cancel queued {job.plan.operation ?? "restart"}
            </button>
          )}
          {events?.id === job.id && (
            <ol className="job-events">
              {events.labels.map((label, i) => (
                <li key={i}>{label}</li>
              ))}
            </ol>
          )}
        </article>
      ))}
    </section>
  );
}
