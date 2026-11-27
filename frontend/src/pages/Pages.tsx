/**
 * Route → page. Every page receives the same session props; host
 * observations come from useOverview(), never from a page's own stream.
 */
import type { SessionView } from "../../../contracts/generated/types";
import type { RouteId } from "../app/routes";
import { OverviewPage } from "./OverviewPage";
import { ContainersPage } from "./ContainersPage";
import { StacksPage } from "./StacksPage";
import { StoragePage } from "./StoragePage";
import { AlertsPage } from "./AlertsPage";
import { NotFoundPage } from "./NotFoundPage";

export interface PageProps {
  session: SessionView;
  expired: () => void;
  reauthenticate: () => void;
}

export const canManage = (session: SessionView) => ["operator", "administrator"].includes(session.principal.role);
export const isAdmin = (session: SessionView) => session.principal.role === "administrator";

export function Pages({ route, ...props }: PageProps & { route: RouteId }) {
  switch (route) {
    case "overview": return <OverviewPage {...props} />;
    case "containers": return <ContainersPage {...props} />;
    case "stacks": return <StacksPage {...props} />;
    case "storage": return <StoragePage {...props} />;
    case "alerts":
    case "alerts-schedules":
    case "alerts-policy": return <AlertsPage {...props} tab={route} />;
    default: return <NotFoundPage />;
  }
}
