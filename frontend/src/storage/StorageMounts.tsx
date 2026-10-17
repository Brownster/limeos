import { useEffect, useMemo, useRef, useSyncExternalStore, type KeyboardEvent } from "react";
import { StorageMountController } from "./controller";
function containFocus(event: KeyboardEvent<HTMLDialogElement>) {
  if (event.key !== "Tab") return;
  const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>(
    "button:not(:disabled), input:not(:disabled), [tabindex='0']",
  )).filter((element) => element.getClientRects().length > 0);
  const first = controls[0], last = controls.at(-1);
  if (!first || !last) { event.preventDefault(); event.currentTarget.focus(); return; }
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
}

export function StorageMounts({ csrf, expired }: { csrf: string; expired: () => void }) {
  return <SessionStorageMounts key={csrf} csrf={csrf} expired={expired} />;
}
function SessionStorageMounts({ csrf, expired }: { csrf: string; expired: () => void }) {
  const callback = useRef(expired); callback.current = expired;
  const controller = useMemo(() => new StorageMountController(csrf, () => callback.current()), [csrf]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  const { target, targets, catalogError, catalogLoading, proposal, jobs, pending, error, historyError, historyLoading, canceling, retryAvailable } = state;
  const unavailable = state.expired || state.denied;
  const dialog = useRef<HTMLDialogElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    void controller.start();
    const timer = window.setInterval(() => { if (!document.hidden) void controller.refreshJobs(); }, 3000);
    return () => { window.clearInterval(timer); controller.dispose(); };
  }, [controller]);
  useEffect(() => {
    if (state.reviewOpen && !dialog.current?.open) dialog.current?.showModal();
    if (!state.reviewOpen && dialog.current?.open) dialog.current.close();
  }, [state.reviewOpen]);
  return (
    <section className="operations" aria-labelledby="mount-heading">
      <h2 id="mount-heading">Mount a managed disk</h2>
      <p>This mounts an approved ext4 data disk until restart. Review its identity and destination before approving.</p>
      <form onSubmit={(event) => { event.preventDefault(); void controller.preview(); }}>
        <label>Managed disk
          <select value={target} onChange={(event) => controller.setTarget(event.target.value)} required disabled={unavailable || pending || catalogLoading || targets.length === 0}>
            {targets.length === 0 && <option value="">{catalogLoading ? "Loading managed disks…" : "No managed ext4 data disks"}</option>}
            {targets.map((item) => <option key={item.id} value={item.id}>{item.id} · {item.mountpoint}</option>)}
          </select>
        </label>
        {targets.filter((item) => item.id === target).map((item) => <p key={item.id}>UUID: {item.filesystem_uuid} · {item.mountpoint}</p>)}
        <button ref={trigger} type="submit" disabled={unavailable || pending || catalogLoading || !target}>Review mount</button>
        <button type="button" className="quiet" disabled={unavailable || pending || catalogLoading} onClick={() => void controller.refreshTargets()}>Refresh managed disks</button>
      </form>
      {retryAvailable && !state.reviewOpen && <button className="quiet" onClick={() => controller.reopenReview()}>Return to pending mount request</button>}
      {catalogError && <p className="error" role="alert">{catalogError}</p>}
      {unavailable && <p className="error" role="alert">{error}</p>}
      <dialog ref={dialog} className="operation-dialog" aria-labelledby="mount-review-heading"
        onKeyDown={containFocus} onCancel={(event) => { if (pending) event.preventDefault(); else controller.closeReview(); }} onClose={() => { controller.closeReview(); trigger.current?.focus(); }}>
        <header className="page-heading">
          <h2 id="mount-review-heading">Review nonpersistent mount</h2>
          <button className="quiet" autoFocus disabled={pending} onClick={() => controller.closeReview()}>Close</button>
        </header>
        {pending && !proposal && <p role="status">Checking the managed disk and its dependencies…</p>}
        {proposal && <>
          <dl className="plan-details">
            <dt>Disk</dt><dd>{proposal.plan.effect.device.id}</dd>
            <dt>Filesystem UUID</dt><dd>{proposal.plan.effect.device.filesystem_uuid}</dd>
            <dt>Mount point</dt><dd>{proposal.plan.effect.device.mountpoint}</dd>
            <dt>Filesystem</dt><dd>ext4 · read/write</dd>
            <dt>Approval expires</dt><dd>{new Date(proposal.plan.effect.expires_at * 1000).toLocaleString()}</dd>
          </dl>
          <p className="notice">This mount ends at restart. Approval applies to this disk and destination.</p>
        </>}
        {error && <p className="error" role="alert">{error}</p>}
        <button onClick={() => void controller.confirm()} disabled={unavailable || !proposal || pending}>
          {pending ? "Submitting…" : retryAvailable ? "Retry the same request" : "Approve and mount"}
        </button>
      </dialog>
      <h3>Recent mount jobs</h3>
      {historyError && <p className="error" role="alert">{historyError}</p>}
      {historyLoading && <p role="status">Reading recent mount jobs…</p>}
      {!historyError && !historyLoading && !unavailable && jobs.length === 0 && <p>No recent mount jobs.</p>}
      {jobs.map((job) => <article className="job-row" key={job.id}>
        <div><strong>{job.plan.effect.device.id}</strong><p>{job.plan.effect.device.mountpoint}</p>
          <span className="badge">{job.state.replaceAll("_", " ")}</span>
          {job.state === "needs_intervention" && <p className="notice">The outcome needs reconciliation. Its resource claims remain held.</p>}
        </div>
        {job.state === "queued" && <button className="quiet" disabled={pending || canceling !== null}
          onClick={() => void controller.cancel(job.id)}>{canceling === job.id ? "Canceling…" : "Cancel before dispatch"}</button>}
      </article>)}
    </section>
  );
}
