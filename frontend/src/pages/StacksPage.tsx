import { useOverview } from "../app/overview-data";
import { ResourceTable } from "../ObservationView";
import { StackPanel } from "../StackManagement";
import { PageHeader } from "../ui/primitives";
import { workloadCounts } from "../overview/overview-model";
import { canManage, isAdmin, type PageProps } from "./Pages";

export function StacksPage({ session, expired }: PageProps) {
  const { value } = useOverview();
  const stacks = value ? value.resources.filter((r) => r.kind === "stack") : [];
  const counts = value ? workloadCounts(value.resources).stacks : null;
  return (
    <>
      <PageHeader
        title="Stacks"
        description={counts && counts.total > 0
          ? `${counts.running} running · ${counts.partial} partly running · ${counts.stopped} stopped`
          : "Compose projects and the services in them."}
      />
      <div className="panel">
        <StackPanel principal={session.principal.id} grantRevision={session.principal.grant_revision}
          csrf={session.csrf_token} expired={expired} admin={isAdmin(session)} manage={canManage(session)} />
      </div>
      {stacks.length > 0 && (
        <details className="panel-details">
          <summary>Stack resources reported by the host observer · {stacks.length}</summary>
          <ResourceTable items={stacks} caption="Observed stacks" />
        </details>
      )}
    </>
  );
}
