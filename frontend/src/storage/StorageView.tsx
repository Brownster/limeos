/**
 * The Storage page's parts: summary band, disk cards with their partitions,
 * pool cards, and the administrator-only inventory details behind each
 * disk's disclosure. Data comes from the shared overview; the inventory is
 * read once when the page opens and on a manual refresh. Styles: storage.css.
 */
import { useEffect, useRef, useState } from "react";
import type { HostMetrics, Resource, StorageInventory } from "../../../contracts/generated/types";
import { metricTone, toneWord } from "../overview/overview-model";
import { Badge, MetricBar, StatusBadge } from "../ui/primitives";
import { CopyValue } from "../ui/copy";
import { formatBytes, formatPercent } from "../ui/format";
import { Icon } from "../ui/icons";
import { readStorageInventory, StorageInventoryError } from "./inventory-client";
import {
  diskSerial, filesystemStatus, mountPersistence, serialOf, uuidOf,
  type DiskNode, type FilesystemFacts, type FilesystemNode, type StorageSummary,
} from "./storage-model";

export type InventoryStatus = "off" | "loading" | "ready" | "denied" | "unavailable";
export interface InventoryState { status: InventoryStatus; value: StorageInventory | null }

/**
 * The storage inventory for an administrator, read when the page mounts and
 * when `refreshKey` changes (the page's refresh control), never on a timer.
 * A refusal (403) hides the details quietly for the rest of the visit.
 */
export function useStorageInventory(enabled: boolean, refreshKey: number, expired: () => void): InventoryState {
  const [state, setState] = useState<InventoryState>({ status: enabled ? "loading" : "off", value: null });
  const denied = useRef(false);
  const expiry = useRef(expired);
  expiry.current = expired;
  useEffect(() => {
    if (!enabled || denied.current) return;
    const controller = new AbortController();
    setState((s) => ({ status: s.value ? "ready" : "loading", value: s.value }));
    readStorageInventory(controller.signal).then(
      (value) => { if (!controller.signal.aborted) setState({ status: "ready", value }); },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        const status = error instanceof StorageInventoryError ? error.status : 0;
        if (status === 401) { expiry.current(); return; }
        if (status === 403) denied.current = true;
        setState({ status: status === 403 ? "denied" : "unavailable", value: null });
      },
    );
    return () => controller.abort();
  }, [enabled, refreshKey]);
  return enabled ? state : { status: "off", value: null };
}

export function StorageSummaryBand({ summary }: { summary: StorageSummary }) {
  const rootTone = metricTone("disk", summary.rootPercent);
  const other = summary.filesystems - summary.mounted - summary.notMounted;
  return (
    <div className="summary-band" role="group" aria-label="Storage summary">
      <div className="summary-cell">
        <p className="summary-label">Disks</p>
        <p className="summary-value">{summary.disks}</p>
        <p className="summary-note">
          {summary.capacity === null ? "total size unknown" : `${formatBytes(summary.capacity)} in total`}
          {summary.sizeUnknown > 0 && summary.capacity !== null ? ` · size unknown for ${summary.sizeUnknown}` : ""}
        </p>
      </div>
      <div className="summary-cell">
        <p className="summary-label">Filesystems mounted</p>
        <p className="summary-value">{summary.mounted} <span className="summary-of">of {summary.filesystems}</span></p>
        <p className="summary-note">{summary.notMounted} not mounted{other > 0 ? ` · ${other} other` : ""}</p>
      </div>
      <div className="summary-cell">
        <p className="summary-label">Pools</p>
        <p className="summary-value">{summary.pools}</p>
        <p className="summary-note">{summary.pools === 0 ? "no mergerfs pool" : `${summary.poolsMounted} mounted`}</p>
      </div>
      <div className="summary-cell">
        <p className="summary-label">Root filesystem</p>
        <p className={`summary-value tone-text-${rootTone}`}>{summary.rootPercent === null ? "Unknown" : formatPercent(summary.rootPercent)}</p>
        <p className="summary-note">{summary.rootPercent === null ? "use not reported" : `used · ${toneWord[rootTone]}`}</p>
        <MetricBar value={summary.rootPercent} tone={rootTone === "neutral" ? "neutral" : rootTone} label="Root filesystem used"
          valueText={summary.rootPercent === null ? "Unknown" : `${formatPercent(summary.rootPercent)} used, ${toneWord[rootTone]}`} />
      </div>
    </div>
  );
}

function FilesystemRow({ node, host }: { node: FilesystemNode; host: HostMetrics | null }) {
  const r = node.resource;
  const status = filesystemStatus(r);
  const percent = node.root ? host?.disk_percent ?? null : null;
  const tone = metricTone("disk", percent);
  return (
    <li className={`fs-row depth-${node.depth}`}>
      <div className="fs-line">
        <span className="fs-name">{r.name}</span>
        {r.filesystem && <Badge tone="info">{r.filesystem}</Badge>}
        <span className="fs-size">{r.size_bytes === null ? "size unknown" : formatBytes(r.size_bytes)}</span>
      </div>
      <div className="fs-line">
        {r.mountpoint ? <span className="fs-mount">{r.mountpoint}</span> : <span className="fs-unmounted">no mount point</span>}
        <StatusBadge label={status.label} tone={status.tone} />
      </div>
      {node.root && (
        <div className="fs-usage">
          <div className="fs-line"><span className="fs-usage-label">Root filesystem</span><span className={`fs-usage-value tone-text-${tone}`}>{percent === null ? "use unknown" : `${formatPercent(percent)} used`}</span></div>
          <MetricBar value={percent} tone={tone === "neutral" ? "neutral" : tone} label={`${r.name} used`} valueText={percent === null ? "Unknown" : `${formatPercent(percent)} used, ${toneWord[tone]}`} />
        </div>
      )}
    </li>
  );
}

function InventoryNote({ inventory, admin }: { inventory: InventoryState; admin: boolean }) {
  if (inventory.status === "ready") return null;
  if (!admin || inventory.status === "denied" || inventory.status === "off") return <p className="disk-details-note">Mount persistence and every mount point are shown to administrators who manage storage.</p>;
  if (inventory.status === "loading") return <p className="disk-details-note" role="status">Reading serial numbers and /etc/fstab…</p>;
  return <p className="disk-details-note">The storage reader didn't answer, so /etc/fstab details aren't shown. Use the refresh button to try again.</p>;
}

function FilesystemDetails({ node, facts }: { node: FilesystemNode; facts: Map<string, FilesystemFacts> | null }) {
  const r = node.resource;
  const uuid = uuidOf(r);
  const fact = uuid && facts ? facts.get(uuid) : undefined;
  const persistence = mountPersistence(r.status === "mounted", fact);
  return (
    <div className="fs-details">
      <p className="fs-details-name">{r.name}{fact?.boot && <Badge tone="accent">system disk</Badge>}{fact?.swap && <Badge tone="info">swap</Badge>}</p>
      <dl className="disk-facts">
        {uuid ? <><dt>Filesystem UUID</dt><dd><CopyValue value={uuid} label={`${r.name} filesystem UUID`} /></dd></>
          : <><dt>Identifier</dt><dd><CopyValue value={r.id} label={`${r.name} identifier`} /></dd></>}
        {persistence && <><dt>At boot</dt><dd>{persistence}</dd></>}
        {fact && fact.mounts.length > 0 && <><dt>{fact.mounts.length === 1 ? "Mount" : "Mounts"}</dt>
          <dd>{fact.mounts.map((m) => <span key={`${m.mountpoint}:${m.root}`} className="disk-mount"><code>{m.mountpoint}</code> {m.writable ? "read/write" : "read-only"}{m.root !== "/" ? ` · subfolder ${m.root}` : ""}</span>)}</dd></>}
      </dl>
    </div>
  );
}

export function DiskCard({ node, host, inventory, facts, admin }: {
  node: DiskNode; host: HostMetrics | null; inventory: InventoryState; facts: Map<string, FilesystemFacts> | null; admin: boolean;
}) {
  const { disk } = node;
  const serial = diskSerial(node, facts);
  const boot = facts ? node.filesystems.some((f) => { const u = uuidOf(f.resource); return !!u && !!facts.get(u)?.boot; }) : false;
  return (
    <article className={`disk-card tone-${node.status.tone}`} aria-label={`Disk ${disk.name}`}>
      <header className="disk-card-head">
        <div className="disk-card-name">
          <h3><Icon name="storage" size={16} className="tone-accent" />{disk.name}</h3>
          <p className="disk-card-sub">{serial ? `Serial ${serial}` : "Serial not reported"}</p>
        </div>
        <div className="disk-card-badges">
          {boot && <Badge tone="accent">system disk</Badge>}
          <StatusBadge label={node.status.label} tone={node.status.tone} />
        </div>
      </header>
      <p className="disk-card-meta">
        <span><span className="dim">Size </span>{formatBytes(disk.size_bytes)}</span>
        <span><span className="dim">Mounted </span>{node.mounted} of {node.filesystems.length}</span>
      </p>
      {node.filesystems.length > 0
        ? <ul className="fs-list" aria-label={`Filesystems on ${disk.name}`}>{node.filesystems.map((f) => <FilesystemRow key={f.resource.id} node={f} host={host} />)}</ul>
        : <p className="fs-unmounted">No partitions or filesystems are reported on this disk.</p>}
      <details className="disk-details card-disclosure">
        <summary><Icon name="chevron" size={14} className="disclosure-chevron" />Identifiers and mounts</summary>
        <dl className="disk-facts">
          {!serialOf(disk) && <><dt>Disk identifier</dt><dd><CopyValue value={disk.id} label={`${disk.name} identifier`} /></dd></>}
          {serial && <><dt>Serial number</dt><dd><CopyValue value={serial} label={`${disk.name} serial number`} /></dd></>}
        </dl>
        {node.filesystems.map((f) => <FilesystemDetails key={f.resource.id} node={f} facts={facts} />)}
        <InventoryNote inventory={inventory} admin={admin} />
      </details>
    </article>
  );
}

/** Filesystems whose disk isn't reported (or isn't a disk the observer lists). */
export function OrphanCard({ nodes, host, facts }: { nodes: FilesystemNode[]; host: HostMetrics | null; facts: Map<string, FilesystemFacts> | null }) {
  return (
    <article className="disk-card" aria-label="Other filesystems">
      <header className="disk-card-head">
        <div className="disk-card-name">
          <h3><Icon name="disk" size={16} className="tone-accent" />Other filesystems</h3>
          <p className="disk-card-sub">Their disk isn't in the latest observation</p>
        </div>
      </header>
      <ul className="fs-list">{nodes.map((f) => <FilesystemRow key={f.resource.id} node={f} host={host} />)}</ul>
      <details className="disk-details card-disclosure">
        <summary><Icon name="chevron" size={14} className="disclosure-chevron" />Identifiers and mounts</summary>
        {nodes.map((f) => <FilesystemDetails key={f.resource.id} node={f} facts={facts} />)}
      </details>
    </article>
  );
}

export function PoolCard({ pool }: { pool: Resource }) {
  const status = filesystemStatus(pool);
  return (
    <article className={`disk-card pool-card tone-${status.tone}`} aria-label={`Pool ${pool.name}`}>
      <header className="disk-card-head">
        <div className="disk-card-name">
          <h3><Icon name="pool" size={16} className="tone-accent" />{pool.name}</h3>
          <p className="disk-card-sub">{pool.filesystem === "fuse.mergerfs" || pool.filesystem === "mergerfs" ? "mergerfs pool" : pool.filesystem ?? "pool"}</p>
        </div>
        <div className="disk-card-badges"><StatusBadge label={status.label} tone={status.tone} /></div>
      </header>
      <p className="disk-card-meta">
        <span><span className="dim">Mount point </span><span className="fs-mount">{pool.mountpoint ?? "unknown"}</span></span>
        {pool.size_bytes !== null && <span><span className="dim">Size </span>{formatBytes(pool.size_bytes)}</span>}
      </p>
    </article>
  );
}
