/**
 * Everything LimeOS knows about one container, in a side drawer that the URL
 * can open (`?container=<name>`): identity, image, stack, status, health,
 * CPU and memory from the shared overview, the principal's recent operations
 * on it, and, for accounts that may read storage, its declared mounts. The
 * mounts read inspects every container on the host, so it runs once when the
 * drawer opens (and on request), never for the list.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { ContainerAction, ContainerStorageConsumer, RestartJob } from "../../../contracts/generated/types";
import { ApiError, request } from "../api";
import { Link } from "../app/router";
import { containFocus } from "../operation-ui";
import { Icon } from "../ui/icons";
import { MetricBar, StateMessage, StatusBadge } from "../ui/primitives";
import { formatAge, formatClock, formatPercent } from "../ui/format";
import { metricBarValue, metricTone, toneWord } from "../overview/overview-model";
import { containerClient } from "./container-client";
import { actionStates, containerId, jobBadge, STARTABLE, type DockerSource, type Row } from "./container-model";
import { CopyButton } from "./CopyButton";
import { CIcon } from "./icons";
import { healthCue, statusCue } from "./ContainerList";
import { jobAnchor } from "./OperationsList";

const client = containerClient(request);

type Mounts =
  | { state: "idle" | "loading" }
  | { state: "ready"; consumer: ContainerStorageConsumer | null; observedAt: number }
  | { state: "forbidden" }
  | { state: "error"; error: unknown };

function started(value: string): string {
  const at = Date.parse(value);
  if (!Number.isFinite(at)) return value;
  return `${new Date(at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })} (${formatAge(at / 1000)})`;
}

function Metric({ label, metric, value, current }: { label: string; metric: "cpu" | "memory"; value: number | null; current: boolean }) {
  const tone = metricTone(metric, value);
  const text = current ? formatPercent(value) : "Unknown";
  return (
    <div className="detail-metric">
      <div className="detail-metric-head">
        <span>{label}</span>
        <span className={`tone-text-${tone}`}>{text}{value !== null && tone !== "neutral" ? ` · ${toneWord[tone]}` : ""}</span>
      </div>
      <MetricBar label={label} value={metricBarValue(metric, value)} tone={tone} valueText={value === null ? "Unknown" : `${text}, ${toneWord[tone]}`} />
      {!current && <p className="detail-note">Docker readings are not current, so this is unknown.</p>}
    </div>
  );
}

export function ContainerDetail({ name, row, docker, now, jobs, canManage, canReadStorage, onAction, onLogs, onClose, onShowOperation }: {
  /** The requested container name; null when closed. */
  name: string | null;
  row: Row | null;
  docker: DockerSource;
  now: number;
  /** The principal's jobs for this container, newest first. */
  jobs: RestartJob[];
  canManage: boolean;
  canReadStorage: boolean;
  onAction: (action: ContainerAction, opener: HTMLElement) => void;
  onLogs: (opener: HTMLElement) => void;
  onClose: () => void;
  onShowOperation: (id: string) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [mounts, setMounts] = useState<Mounts>({ state: "idle" });
  const open = name !== null;
  const resourceId = row?.resource.id ?? null;
  const idBox = useRef<HTMLElement>(null);
  const imageBox = useRef<HTMLElement>(null);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    if (!open && element.open) element.close();
  }, [open]);

  const loadMounts = useCallback(() => {
    if (!canReadStorage) return;
    setMounts({ state: "loading" });
    client
      .storage()
      .then((view) => {
        const consumer = view.inventory.containers.find((c) => c.container.resource === resourceId) ?? null;
        setMounts({ state: "ready", consumer, observedAt: view.inventory.observed_at });
      })
      .catch((error: unknown) => {
        if (error instanceof ApiError && error.detail.code === "forbidden") setMounts({ state: "forbidden" });
        else setMounts({ state: "error", error });
      });
  }, [canReadStorage, resourceId]);

  // One read per opening of a container (and on request).
  useEffect(() => {
    if (open && resourceId) loadMounts();
    else setMounts({ state: "idle" });
  }, [open, resourceId, loadMounts]);

  const current = docker.state === "fresh";
  const status = row ? statusCue(row) : null;
  const health = row ? healthCue(row.resource.health) : null;
  const badge = row ? jobBadge(row.job, now) : null;
  const ready = mounts.state === "ready" ? mounts : null;

  return (
    <dialog ref={dialog} className="drawer detail-drawer" aria-labelledby="detail-title" onClose={onClose} onKeyDown={containFocus}>
      <header className="drawer-head">
        <div className="drawer-title">
          <p className="drawer-eyebrow">Container</p>
          <h2 id="detail-title">{row?.name ?? name ?? ""}</h2>
          {row && status && health && (
            <div className="detail-badges">
              <StatusBadge tone={status.tone} label={status.label} />
              <span className={`health-cue tone-text-${health.tone}`}>{health.icon && <CIcon name={health.icon} size={14} />}{health.icon ? health.label : "no healthcheck defined"}</span>
              {badge && <StatusBadge tone={badge.tone} label={badge.label} />}
            </div>
          )}
        </div>
        <button type="button" className="quiet icon-button" autoFocus onClick={() => dialog.current?.close()} aria-label="Close container details" title="Close">
          <CIcon name="close" size={18} />
        </button>
      </header>
      {!row && open && (
        <StateMessage kind="empty" title={`No container named “${name}” is reported`}>
          It may have been removed or renamed, or the overview has not arrived yet.
        </StateMessage>
      )}
      {row && (
        <div className="drawer-body">
          {row.attention && (
            <p className="detail-attention"><Icon name="warning" size={16} /><span><strong>Needs attention:</strong> {row.attention}.</span></p>
          )}
          {canManage && (
            <div className="detail-actions" role="group" aria-label={`Actions for ${row.name}`}>
              {actionStates(row).filter((state) => (state.action === "start") !== row.running && (row.running || STARTABLE.has(row.resource.status))).map((state) => (
                // Relevant actions stay while an operation runs (aria-disabled), so focus is not lost.
                <button key={state.action} type="button" className="quiet" aria-disabled={state.disabled ? true : undefined}
                  aria-describedby={state.disabled ? "detail-why" : undefined}
                  onClick={(event) => !state.disabled && onAction(state.action, event.currentTarget)}>
                  <CIcon name={state.action === "start" ? "play" : state.action === "stop" ? "square" : "rotate"} />
                  {state.label}
                </button>
              ))}
              <button type="button" className="quiet" onClick={(event) => onLogs(event.currentTarget)}>
                <CIcon name="logs" />Logs
              </button>
              {actionStates(row).every((state) => state.disabled) && (
                <p className="detail-note detail-why" id="detail-why">Start, stop and restart are unavailable: {actionStates(row)[0].disabled!.replace(/^./, (c) => c.toLowerCase())}.</p>
              )}
            </div>
          )}

          <section className="detail-section" aria-labelledby="detail-usage">
            <h3 id="detail-usage">Usage</h3>
            <Metric label="CPU" metric="cpu" value={row.cpu} current={current} />
            <Metric label="Memory" metric="memory" value={row.memory} current={current} />
            <p className="detail-note">
              {docker.sampledAt === null ? "Docker has not reported yet." : `Docker reading from ${formatClock(docker.sampledAt)} (${formatAge(docker.sampledAt, now)}).`}
              {" "}CPU can exceed 100% on a multi-core host; memory is a share of the container's limit.
            </p>
          </section>

          <section className="detail-section" aria-labelledby="detail-identity">
            <h3 id="detail-identity">Identity</h3>
            <dl className="detail-facts">
              <dt>Stack</dt>
              <dd>{row.stack ? <Link to="/stacks">{row.stack}</Link> : "Not in a stack"}</dd>
              <dt>Image</dt>
              <dd>
                {row.image.kind === "reference" ? (
                  <dl className="image-parts">
                    <dt>Registry</dt><dd>{row.image.registry ?? "Docker Hub"}</dd>
                    <dt>Repository</dt><dd>{row.image.repository}</dd>
                    <dt>Tag</dt><dd>{row.image.tag ?? "none given (Docker uses latest)"}</dd>
                    {row.image.digest && <><dt>Digest</dt><dd><code className="mono-wrap">{row.image.digest}</code></dd></>}
                  </dl>
                ) : row.image.kind === "id" ? (
                  <span>Only an image ID is reported: the tag it was started from has moved or been removed.</span>
                ) : "Unknown"}
                {row.image.raw && (
                  <div className="copy-row">
                    <code ref={imageBox} className="mono-wrap">{row.image.raw}</code>
                    <CopyButton value={row.image.raw} label="Copy image reference" target={() => imageBox.current} compact />
                  </div>
                )}
              </dd>
              <dt>Container ID</dt>
              <dd>
                <div className="copy-row">
                  <code ref={idBox} className="mono-wrap">{containerId(row.resource)}</code>
                  <CopyButton value={containerId(row.resource)} label="Copy container ID" target={() => idBox.current} compact />
                </div>
              </dd>
              {ready?.consumer && (
                <>
                  <dt>Started</dt>
                  <dd>{started(ready.consumer.container.started_at)}</dd>
                  <dt>Image ID</dt>
                  <dd><code className="mono-wrap">{ready.consumer.container.image}</code></dd>
                </>
              )}
            </dl>
          </section>

          {canManage && (
            <section className="detail-section" aria-labelledby="detail-operations">
              <h3 id="detail-operations">Your recent operations</h3>
              {jobs.length === 0 ? <p className="detail-note">None recently.</p> : (
                <ul className="detail-jobs">
                  {jobs.slice(0, 5).map((job) => (
                    <li key={job.id}>
                      <span>{(job.plan.operation ?? "restart").replace(/^./, (c) => c.toUpperCase())}</span>
                      <span className="job-meta">{formatClock(job.plan.created_at)}</span>
                      <span className="detail-job-state">{job.state.replaceAll("_", " ")}</span>
                      <a href={`#${jobAnchor(job.id)}`} onClick={(event) => { event.preventDefault(); onShowOperation(job.id); }}>Details</a>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}

          <section className="detail-section" aria-labelledby="detail-mounts">
            <h3 id="detail-mounts">Storage mounts</h3>
            {!canReadStorage && <p className="detail-note">Storage mounts are shown to accounts that may manage storage (administrators).</p>}
            {mounts.state === "loading" && <p className="detail-note" role="status">Reading mounts from the host…</p>}
            {mounts.state === "forbidden" && <p className="detail-note">Your account isn't allowed to read storage mounts. An administrator can see them here.</p>}
            {mounts.state === "error" && <StateMessage kind="error" compact error={mounts.error} subject="Storage mounts" onRetry={loadMounts} />}
            {ready && !ready.consumer && <p className="detail-note">The host's mount read did not include this container (it reads at most 64).</p>}
            {ready?.consumer && (ready.consumer.mounts.length === 0 ? <p className="detail-note">No bind mounts or volumes.</p> : (
              <ul className="mount-list">
                {ready.consumer.mounts.map((mount) => (
                  <li key={mount.destination}>
                    <span className="mount-path"><span className="mono-wrap">{mount.source.path}</span><span aria-hidden="true"> → </span><span className="sr-only"> mounted at </span><span className="mono-wrap">{mount.destination}</span></span>
                    <span className="mount-meta">
                      {mount.source.kind === "volume" ? `volume ${mount.source.name}` : "bind mount"} · {mount.writable ? "read-write" : "read-only"}
                    </span>
                  </li>
                ))}
              </ul>
            ))}
            {(ready || mounts.state === "error") && (
              <button type="button" className="quiet" onClick={loadMounts}>Read mounts again</button>
            )}
          </section>
        </div>
      )}
    </dialog>
  );
}
