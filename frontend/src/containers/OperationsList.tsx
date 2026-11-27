/**
 * "Recent container operations": the principal's last eight container jobs
 * with their state, history and a cancel for queued ones, and the banner of
 * jobs that need an administrator's resolution. Selectors other suites use
 * stay: the section's `operations-heading`, `.job-row` articles holding the
 * container's 12-character ID and the state word, "Progress" and
 * "Cancel queued <action>".
 */
import { useState } from "react";
import type { EventKind, RestartJob } from "../../../contracts/generated/types";
import { api } from "../api";
import { mergeAttention, NeedsAttention } from "../ResolutionPanel";
import { StateMessage, StatusBadge, type Tone } from "../ui/primitives";
import { formatClock } from "../ui/format";
import { actionLabel } from "./container-model";
import type { ContainerJobsState } from "./use-container-jobs";

const stateTone: Record<string, Tone> = {
  queued: "info", running: "info", verifying: "info", succeeded: "success", resolved: "success",
  failed: "danger", needs_intervention: "danger", outcome_unknown: "danger",
  canceled: "neutral", precondition_changed: "warning", planned: "neutral", waiting_approval: "neutral",
};

/** One line of a job's history: its state, or what happened to it. */
function eventLabel(event: EventKind): string {
  if (event.kind === "job_transition") return event.state.replaceAll("_", " ");
  if (event.kind === "executor_budget_spent") {
    const until = event.frees_at === undefined ? "" : ` until ${new Date(event.frees_at * 1000).toLocaleString()}`;
    return `refused: the effect budget of a container it acts on is spent${until}`;
  }
  return event.kind.replaceAll("_", " ");
}

export const jobAnchor = (id: string) => `job-${id.slice(0, 16)}`;

export function ContainerJobs({ state, names, csrf, expired, admin = false, changed }: {
  state: ContainerJobsState;
  /** Container resource id → name, from the overview. */
  names: Map<string, string>;
  csrf: string;
  expired: () => void;
  /** Only administrators may resolve; others see the banner as information. */
  admin?: boolean;
  changed: () => void;
}) {
  const { jobs, listed, error } = state;
  const [actionError, setActionError] = useState<unknown>(null);
  const [events, setEvents] = useState<{ id: string; labels: string[] } | null>(null);
  const title = (job: RestartJob) => {
    const resource = job.plan.expected.resource;
    const name = names.get(resource);
    const short = resource.slice(10, 22);
    return name ? `${actionLabel(job.plan.operation ?? "restart")} · ${name} · ${short}` : `${actionLabel(job.plan.operation ?? "restart")} · ${short}`;
  };
  async function cancel(id: string) {
    try {
      await api.cancelRestart(id, csrf);
      setActionError(null);
      changed();
    } catch (e) {
      setActionError(e);
    }
  }
  async function details(id: string) {
    if (events?.id === id) {
      setEvents(null);
      return;
    }
    try {
      const progress = await api.restartProgress(id);
      setEvents({ id, labels: progress.events.map((e) => eventLabel(e.event)) });
      setActionError(null);
    } catch (e) {
      setActionError(e);
    }
  }
  const attention = mergeAttention(
    jobs.filter((job) => job.state === "needs_intervention").map((job) => ({ id: job.id, title: title(job) })),
    listed,
    "container",
  );
  if (!jobs.length && !attention.length && !error) return null;
  return (
    <section className="operations" aria-labelledby="operations-heading">
      <h2 id="operations-heading" tabIndex={-1}>Recent container operations</h2>
      <p className="operations-intro">Your latest starts, stops and restarts. Each runs once, in order, after you approve it.</p>
      {error !== null && <StateMessage kind="error" compact error={error} subject="Recent operations" onRetry={state.reload} />}
      {actionError !== null && <StateMessage kind="error" compact error={actionError} subject="That operation" />}
      <NeedsAttention jobs={attention} admin={admin} csrf={csrf} expired={expired} changed={changed} />
      <div className="job-list">
        {jobs.slice(0, 8).map((job) => {
          const operation = job.plan.operation ?? "restart";
          return (
            <article key={job.id} id={jobAnchor(job.id)} className="job-row" tabIndex={-1} aria-label={title(job)}>
              <div className="job-main">
                <strong className="job-title">{title(job)}</strong>
                <span className="job-meta">requested <time dateTime={new Date(job.plan.created_at * 1000).toISOString()}>{formatClock(job.plan.created_at)}</time></span>
                {job.state === "needs_intervention" && (
                  <p className="notice">The outcome needs inspection. The resource remains locked and the operation will not be repeated.</p>
                )}
              </div>
              <span className="job-state" role="status"><StatusBadge tone={stateTone[job.state] ?? "neutral"} label={job.state.replaceAll("_", " ")} /></span>
              <div className="job-actions">
                <button className="quiet" aria-expanded={events?.id === job.id} onClick={() => void details(job.id)}>Progress</button>
                {job.state === "queued" && (
                  <button className="quiet" onClick={() => void cancel(job.id)}>Cancel queued {operation}</button>
                )}
              </div>
              {events?.id === job.id && (
                <ol className="job-events">
                  {events.labels.map((label, i) => <li key={i}>{label}</li>)}
                </ol>
              )}
            </article>
          );
        })}
      </div>
    </section>
  );
}
