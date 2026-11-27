/**
 * Start, stop or restart one container: the host plans it (inspecting the
 * container), the user approves that exact plan, and the approved plan is
 * queued as a durable job. A lost queue response keeps its approval and
 * request key so "Retry same request" can never queue a second job.
 * Ported from pi-health@80593 container-list's Restart control; one dialog
 * serves the whole page and gives focus back to whatever opened it.
 */
import { useEffect, useId, useRef, useState } from "react";
import type { ContainerAction, PlannedRestart, Resource } from "../../../contracts/generated/types";
import { api } from "../api";
import { containFocus } from "../operation-ui";
import { restoreFocus } from "../ResolutionPanel";
import { StateMessage } from "../ui/primitives";
import { actionLabel, parseImage, imageLabel } from "./container-model";

export interface LifecycleTarget {
  resource: Resource;
  action: ContainerAction;
  opener: HTMLElement | null;
  /** Fallback focus when the opener has left the page. */
  fallback?: () => HTMLElement | null;
  /** Distinguishes two openings of the same container and action. */
  nonce: number;
}

const explanation: Record<ContainerAction, string> = {
  start: "This starts the selected service.",
  stop: "This stops the selected service until it is started again.",
  restart: "This briefly interrupts the service.",
};

function started(value: string): string {
  const at = Date.parse(value);
  return Number.isFinite(at) ? new Date(at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : value;
}

export function LifecycleDialog({ target, csrf, changed, onClosed }: {
  target: LifecycleTarget | null;
  csrf: string;
  changed: () => void;
  onClosed: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [proposal, setProposal] = useState<PlannedRestart | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<{ value: unknown; stage: "plan" | "queue" } | null>(null);
  const submission = useRef<{ proposal: PlannedRestart; approval: string; key: string } | null>(null);
  const current = useRef<LifecycleTarget | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  useEffect(() => {
    if (!target) return;
    current.current = target;
    setPending(true);
    setError(null);
    setProposal(null);
    submission.current = null;
    if (!dialog.current?.open) dialog.current?.showModal();
    const { resource, action } = target;
    api
      .planContainer(resource.id, action, csrf)
      .then((next) => {
        if (next.plan.expected.resource !== resource.id || (next.plan.operation ?? "restart") !== action) throw new Error("Plan identity mismatch");
        if (alive.current && current.current === target) setProposal(next);
      })
      .catch((e: unknown) => {
        if (alive.current && current.current === target) setError({ value: e, stage: "plan" });
      })
      .finally(() => {
        if (alive.current && current.current === target) setPending(false);
      });
  }, [target, csrf]);

  async function confirm() {
    if (!proposal || pending) return;
    setPending(true);
    setError(null);
    try {
      if (!submission.current) {
        const approval = await api.approveRestart(proposal, csrf);
        submission.current = { proposal, approval: approval.token, key: crypto.randomUUID() };
      }
      const { proposal: approved, approval, key } = submission.current;
      await api.queueRestart(approved, approval, key, csrf);
      submission.current = null;
      changed();
      if (alive.current) dialog.current?.close();
    } catch (e) {
      if (alive.current) setError({ value: e, stage: "queue" });
      changed();
    } finally {
      if (alive.current) setPending(false);
    }
  }

  function closed() {
    const was = current.current;
    current.current = null;
    setError(null);
    onClosed();
    if (was) restoreFocus(was.opener, was.fallback ?? (() => null));
  }

  const resource = target?.resource;
  const action = target?.action ?? "restart";
  const label = actionLabel(action);
  return (
    <dialog ref={dialog} className="operation-dialog lifecycle-dialog" aria-labelledby={titleId} onClose={closed} onKeyDown={containFocus}>
      <header className="page-heading">
        <h2 id={titleId}>{label} {resource?.name}</h2>
        <button className="quiet" autoFocus onClick={() => dialog.current?.close()} aria-label={`Close ${action} preview`}>
          Close
        </button>
      </header>
      <p>{explanation[action]} Review the selected container before approving.</p>
      {pending && !proposal && <p role="status" className="dialog-status">Inspecting the container…</p>}
      {proposal && resource && (
        <dl className="plan-details">
          <dt>Container</dt>
          <dd><span className="plan-name">{resource.name}</span><span className="mono-dim">{proposal.plan.expected.resource.slice(10)}</span></dd>
          <dt>Image</dt>
          <dd><span className="plan-name">{imageLabel(parseImage(resource.image))}</span><span className="mono-dim">{proposal.plan.expected.image}</span></dd>
          <dt>Started</dt>
          <dd>{started(proposal.plan.expected.started_at)}</dd>
          <dt>Approval expires</dt>
          <dd>{new Date(proposal.plan.expires_at * 1000).toLocaleTimeString()}</dd>
        </dl>
      )}
      {error && (
        <StateMessage kind="error" compact error={error.value} subject={error.stage === "plan" ? `The ${action} preview` : `The ${action} request`}
          title={error.stage === "plan" ? `LimeOS couldn't prepare the ${action}` : `The ${action} wasn't confirmed`}>
          {error.stage === "queue" ? "Check Recent container operations before trying again." : undefined}
        </StateMessage>
      )}
      {submission.current && (
        <p className="notice">
          The response was interrupted. Check recent operations or retry this same request; it retains its original request key.
        </p>
      )}
      <button disabled={!proposal || pending} onClick={() => void confirm()}>
        {pending && proposal ? "Submitting…" : submission.current ? "Retry same request" : `Approve and ${action}`}
      </button>
    </dialog>
  );
}
