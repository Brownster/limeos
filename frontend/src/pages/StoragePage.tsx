import { useMemo } from "react";
import { useOverview } from "../app/overview-data";
import { sourceStale } from "../overview/overview-model";
import { StorageMounts } from "../storage/StorageMounts";
import { DiskCard, OrphanCard, PoolCard, StorageSummaryBand, useStorageInventory } from "../storage/StorageView";
import { inventoryFacts, storageSummary, storageTree, unmatchedFstab } from "../storage/storage-model";
import { PageHeader, StateMessage } from "../ui/primitives";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import { isAdmin, type PageProps } from "./Pages";
import "../ui/parts.css";
import "../storage/storage.css";

export function StoragePage({ session, expired }: PageProps) {
  const { value, now, refreshKey } = useOverview();
  const admin = isAdmin(session);
  const inventory = useStorageInventory(admin, refreshKey, expired);
  const tree = useMemo(() => storageTree(value?.resources), [value]);
  const summary = storageSummary(tree, value?.host);
  const facts = useMemo(() => (inventory.value ? inventoryFacts(inventory.value) : null), [inventory.value]);
  const missing = inventory.value ? unmatchedFstab(inventory.value) : [];
  const stale = value ? value.sources.filter((s) => (s.source === "storage" || s.source === "pools") && sourceStale(s, now)) : [];
  return (
    <>
      <PageHeader
        title="Storage"
        description={value ? `${summary.disks} ${summary.disks === 1 ? "disk" : "disks"} · ${summary.filesystems} ${summary.filesystems === 1 ? "filesystem" : "filesystems"} · ${summary.pools} ${summary.pools === 1 ? "pool" : "pools"}` : "Disks, partitions and pools on this host."}
        actions={<OverviewRefresh />}
      />
      <div className="storage-page">
        <OverviewPending />
        {value && <>
          {stale.length > 0 && <p className="storage-banner" role="status">
            {stale.map((s) => s.source === "pools" ? "Pool" : "Disk").join(" and ")} readings are out of date, so what is mounted may have changed since.
          </p>}
          <StorageSummaryBand summary={summary} />
          {missing.length > 0 && <StateMessage kind="unavailable" compact title={`${missing.length === 1 ? "An /etc/fstab entry has" : `${missing.length} /etc/fstab entries have`} no matching disk`}>
            {missing.map((m) => m.mountpoint).join(", ")}: the disk may be unplugged or failing. It was listed in /etc/fstab{missing.some((m) => m.managed) ? " (by LimeOS)" : ""} but isn't present now.
          </StateMessage>}
          <section className="storage-section" aria-labelledby="disks-heading">
            <h2 id="disks-heading">Disks</h2>
            {tree.disks.length === 0 && tree.orphans.length === 0
              ? <StateMessage kind="empty" title="No disks are reported">The host observer lists no disks. If you expected some, check the storage source on the Overview.</StateMessage>
              : <div className="disk-grid">
                {tree.disks.map((node) => <DiskCard key={node.disk.id} node={node} host={value.host} inventory={inventory} facts={facts} admin={admin} />)}
                {tree.orphans.length > 0 && <OrphanCard nodes={tree.orphans} host={value.host} facts={facts} />}
              </div>}
          </section>
          <section className="storage-section" aria-labelledby="pools-heading">
            <h2 id="pools-heading">Pools</h2>
            {tree.pools.length === 0
              ? <p className="storage-note">No mergerfs pools are mounted.</p>
              : <div className="disk-grid">{tree.pools.map((pool) => <PoolCard key={pool.id} pool={pool} />)}</div>}
          </section>
          <p className="storage-note">Free space is reported only for the root filesystem, so other disks and pools show their size without a usage bar.</p>
        </>}
      </div>
      {admin && (
        <div className="panel storage-mounts-panel">
          <StorageMounts csrf={session.csrf_token} expired={expired} />
        </div>
      )}
    </>
  );
}
