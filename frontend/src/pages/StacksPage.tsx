import { useOverview } from "../app/overview-data";
import { StackPanel } from "../StackManagement";
import { PageHeader } from "../ui/primitives";
import { canManage, isAdmin, type PageProps } from "./Pages";
import "../ui/parts.css";
import "../stacks/stacks.css";

/**
 * The stack inventory is read by its own controller while this page is open;
 * the shared overview only names containers and stands in when the
 * inventory can't be read.
 */
export function StacksPage({ session, expired }: PageProps) {
  const { value, now } = useOverview();
  return (
    <>
      <PageHeader title="Stacks" description="Your Compose stacks, the services in them and how each container is doing." />
      <StackPanel principal={session.principal.id} grantRevision={session.principal.grant_revision}
        csrf={session.csrf_token} expired={expired} admin={isAdmin(session)} manage={canManage(session)}
        resources={value?.resources} sources={value?.sources} now={now} />
    </>
  );
}
