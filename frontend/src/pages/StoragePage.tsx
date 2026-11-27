import { useState } from "react";
import type { ResourceKind } from "../../../contracts/generated/types";
import { useOverview } from "../app/overview-data";
import { PartialNotice, ResourceTable } from "../ObservationView";
import { StorageMounts } from "../storage/StorageMounts";
import { PageHeader } from "../ui/primitives";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import { isAdmin, type PageProps } from "./Pages";

const views: [string, ResourceKind | undefined][] = [
  ["All storage", undefined],
  ["Disks", "disk"],
  ["Partitions", "partition"],
  ["Pools", "pool"],
];
const storageKinds: ResourceKind[] = ["disk", "partition", "pool"];

export function StoragePage({ session, expired }: PageProps) {
  const { value } = useOverview();
  const [kind, setKind] = useState<ResourceKind | undefined>();
  const storage = value ? value.resources.filter((r) => storageKinds.includes(r.kind)) : [];
  const items = storage.filter((r) => kind === undefined || r.kind === kind);
  const count = (k: ResourceKind) => storage.filter((r) => r.kind === k).length;
  return (
    <>
      <PageHeader
        title="Storage"
        description={value ? `${count("disk")} disks · ${count("partition")} partitions · ${count("pool")} pools` : "Disks, partitions and pools on this host."}
        actions={<OverviewRefresh />}
      />
      <OverviewPending />
      {value && (
        <div className="panel">
          <PartialNotice value={value} />
          <div className="segmented resource-nav" role="group" aria-label="Storage views">
            {views.map(([label, k]) => (
              <button key={label} type="button" aria-pressed={kind === k} onClick={() => setKind(k)}>{label}</button>
            ))}
          </div>
          <ResourceTable items={items} caption="Observed storage" empty="No storage of this kind is reported. Source availability is shown on the Overview." />
        </div>
      )}
      {isAdmin(session) && (
        <div className="panel">
          <StorageMounts csrf={session.csrf_token} expired={expired} />
        </div>
      )}
    </>
  );
}
