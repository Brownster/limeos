import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { PlannedStorageMount, StorageMountJob, StorageDevice } from "../../../contracts/generated/types";
import { ApiError } from "../api";
import { mountApi } from "./client";

function message(error: unknown): string {
  return error instanceof ApiError
    ? `${error.message} · reference ${error.detail.audit_id}`
    : "Mount operations are unavailable. Check recent jobs before retrying.";
}
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
  const [target, setTarget] = useState("");
  const [targets, setTargets] = useState<StorageDevice[]>([]);
  const [catalogError, setCatalogError] = useState("");
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [proposal, setProposal] = useState<PlannedStorageMount | null>(null);
  const [jobs, setJobs] = useState<StorageMountJob[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [historyError, setHistoryError] = useState("");
  const [canceling, setCanceling] = useState<string | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const alive = useRef(true);
  const submission = useRef<{ proposal: PlannedStorageMount; approval: string; key: string } | null>(null);
  const refresh = useCallback(async () => {
    try {
      const current = await mountApi.jobs();
      if (alive.current) { setJobs(current); setHistoryError(""); }
    } catch (failure) {
      if (!alive.current) return;
      setJobs([]);
      if (failure instanceof ApiError && failure.detail.code === "unauthenticated") { expired(); return; }
      setHistoryError(message(failure));
    }
  }, [expired]);
  const refreshTargets = useCallback(async () => {
    setCatalogLoading(true); setCatalogError("");
    try {
      const catalog = await mountApi.targets();
      if (!alive.current) return;
      setTargets(catalog.targets);
      setTarget((previous) => catalog.targets.some((item) => item.id === previous) ? previous : (catalog.targets[0]?.id ?? ""));
    } catch (failure) {
      if (!alive.current) return;
      setTargets([]); setTarget(""); setCatalogError(message(failure));
      if (failure instanceof ApiError && failure.detail.code === "unauthenticated") expired();
    } finally { if (alive.current) setCatalogLoading(false); }
  }, [expired]);
  useEffect(() => { void refreshTargets(); }, [refreshTargets]);
  useEffect(() => {
    alive.current = true;
    void refresh();
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 3000);
    return () => { alive.current = false; window.clearInterval(timer); };
  }, [refresh]);
  async function preview() {
    if (pending) return;
    setPending(true); setError(""); setProposal(null); submission.current = null;
    dialog.current?.showModal();
    try {
      const current = await mountApi.plan(target, csrf);
      if (current.plan.effect.device.id !== target || current.plan.effect.operation !== "storage.mount") {
        throw new Error("Mount plan identity mismatch");
      }
      if (alive.current) setProposal(current);
    } catch (failure) { if (alive.current) setError(message(failure)); }
    finally { if (alive.current) setPending(false); }
  }
  async function confirm() {
    if (!proposal || pending) return;
    setPending(true); setError("");
    try {
      if (!submission.current) {
        const approval = await mountApi.approve(proposal, csrf);
        submission.current = { proposal, approval: approval.token, key: crypto.randomUUID() };
      }
      const saved = submission.current;
      await mountApi.queue(saved.proposal, saved.approval, saved.key, csrf);
      submission.current = null;
      if (alive.current) dialog.current?.close();
      await refresh();
    } catch (failure) { if (alive.current) setError(message(failure)); await refresh(); }
    finally { if (alive.current) setPending(false); }
  }
  async function cancel(id: string) {
    if (canceling) return;
    setCanceling(id); setHistoryError("");
    try { await mountApi.cancel(id, csrf); await refresh(); }
    catch (failure) { if (alive.current) setHistoryError(message(failure)); }
    finally { if (alive.current) setCanceling(null); }
  }
  return (
    <section className="operations" aria-labelledby="mount-heading">
      <h2 id="mount-heading">Mount a managed disk</h2>
      <p>This mounts an approved ext4 data disk until restart. Review its identity and destination before approving.</p>
      <form onSubmit={(event) => { event.preventDefault(); void preview(); }}>
        <label>Managed disk
          <select value={target} onChange={(event) => setTarget(event.target.value)} required disabled={pending || catalogLoading || targets.length === 0}>
            {targets.length === 0 && <option value="">{catalogLoading ? "Loading managed disks…" : "No managed ext4 data disks"}</option>}
            {targets.map((item) => <option key={item.id} value={item.id}>{item.id} · {item.mountpoint}</option>)}
          </select>
        </label>
        {targets.filter((item) => item.id === target).map((item) => <p key={item.id}>UUID: {item.filesystem_uuid} · {item.mountpoint}</p>)}
        <button ref={trigger} type="submit" disabled={pending || catalogLoading || !target}>Review mount</button>
        <button type="button" className="quiet" disabled={pending || catalogLoading} onClick={() => void refreshTargets()}>Refresh managed disks</button>
      </form>
      {catalogError && <p className="error" role="alert">{catalogError}</p>}
      <dialog ref={dialog} className="operation-dialog" aria-labelledby="mount-review-heading"
        onKeyDown={containFocus} onClose={() => trigger.current?.focus()}>
        <header className="page-heading">
          <h2 id="mount-review-heading">Review nonpersistent mount</h2>
          <button className="quiet" autoFocus onClick={() => dialog.current?.close()}>Close</button>
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
        <button onClick={() => void confirm()} disabled={!proposal || pending}>
          {pending ? "Submitting…" : submission.current ? "Retry the same request" : "Approve and mount"}
        </button>
      </dialog>
      <h3>Recent mount jobs</h3>
      {historyError && <p className="error" role="alert">{historyError}</p>}
      {!historyError && jobs.length === 0 && <p>No recent mount jobs.</p>}
      {jobs.map((job) => <article className="job-row" key={job.id}>
        <div><strong>{job.plan.effect.device.id}</strong><p>{job.plan.effect.device.mountpoint}</p>
          <span className="badge">{job.state.replaceAll("_", " ")}</span>
          {job.state === "needs_intervention" && <p className="notice">The outcome needs reconciliation. Its resource claims remain held.</p>}
        </div>
        {job.state === "queued" && <button className="quiet" disabled={canceling !== null}
          onClick={() => void cancel(job.id)}>{canceling === job.id ? "Canceling…" : "Cancel before dispatch"}</button>}
      </article>)}
    </section>
  );
}
