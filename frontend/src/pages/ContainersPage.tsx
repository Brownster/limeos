import { useCallback, useState } from "react";
import { useOverview } from "../app/overview-data";
import { ContainerJobs } from "../ContainerActions";
import { page } from "../model";
import { PartialNotice, ResourceTable } from "../ObservationView";
import { PageHeader } from "../ui/primitives";
import { workloadCounts } from "../overview/overview-model";
import { OverviewPending, OverviewRefresh } from "./ObservationState";
import { canManage, isAdmin, type PageProps } from "./Pages";

const PAGE = 20;

export function ContainersPage({ session, expired }: PageProps) {
  const { value } = useOverview();
  const [offset, setOffset] = useState(0);
  const [operationsRevision, setOperationsRevision] = useState(0);
  const changed = useCallback(() => setOperationsRevision((v) => v + 1), []);
  const manage = canManage(session);
  const counts = value ? workloadCounts(value.resources).containers : null;
  const total = counts?.total ?? 0;
  const safeOffset = offset < total ? offset : 0;
  return (
    <>
      <PageHeader
        title="Containers"
        description={counts ? `${counts.running} running · ${counts.stopped} stopped · ${counts.unhealthy} unhealthy · ${counts.total} in total` : "Every container the host reports."}
        actions={<OverviewRefresh />}
      />
      <OverviewPending />
      {value && (
        <div className="panel">
          <PartialNotice value={value} />
          <ResourceTable
            items={page(value.resources, "container", safeOffset)}
            csrf={session.csrf_token}
            canManage={manage}
            changed={changed}
            caption="Containers"
            empty="No containers are reported. If you expected some, check the docker source on the Overview."
          />
          {total > PAGE && (
            <div className="pager">
              <button className="quiet" disabled={safeOffset === 0} onClick={() => setOffset(Math.max(0, safeOffset - PAGE))}>Previous</button>
              <span>{`${safeOffset + 1}–${Math.min(safeOffset + PAGE, total)} of ${total}`}</span>
              <button className="quiet" disabled={safeOffset + PAGE >= total} onClick={() => setOffset(safeOffset + PAGE)}>Next</button>
            </div>
          )}
        </div>
      )}
      {manage && (
        <div className="panel">
          <ContainerJobs csrf={session.csrf_token} revision={operationsRevision} expired={expired} admin={isAdmin(session)} changed={changed} />
        </div>
      )}
    </>
  );
}
