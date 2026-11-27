/**
 * Alerts: history (everyone), schedules and policy (operators and
 * administrators). Each part is its own route and mounts only while shown,
 * so its reads happen on demand exactly as the old toggle buttons did.
 */
import { Link } from "../app/router";
import { HealthHistory } from "../HealthHistory";
import { HealthPolicy } from "../HealthPolicy";
import { HealthSchedules } from "../HealthSchedules";
import { PageHeader, StateMessage } from "../ui/primitives";
import { canManage, type PageProps } from "./Pages";

type Tab = "alerts" | "alerts-schedules" | "alerts-policy";

const tabs: { id: Tab; path: string; label: string; manage: boolean }[] = [
  { id: "alerts", path: "/alerts", label: "History", manage: false },
  { id: "alerts-schedules", path: "/alerts/schedules", label: "Schedules", manage: true },
  { id: "alerts-policy", path: "/alerts/policy", label: "Policy", manage: true },
];

const descriptions: Record<Tab, string> = {
  alerts: "Recorded alerts and in-app notifications from the health checks.",
  "alerts-schedules": "When LimeOS checks container health by itself.",
  "alerts-policy": "Which checks raise alerts, how many failures count, and silences.",
};

export function AlertsPage({ session, expired, reauthenticate, tab }: PageProps & { tab: Tab }) {
  const manage = canManage(session);
  const visible = tabs.filter((t) => manage || !t.manage);
  const ids = { principal: session.principal.id, grantRevision: session.principal.grant_revision, csrf: session.csrf_token, expired };
  const allowed = manage || tab === "alerts";
  return (
    <>
      <PageHeader title="Alerts" description={descriptions[tab]} />
      {visible.length > 1 && (
        <nav className="subnav" aria-label="Alerts sections">
          {visible.map((t) => <Link key={t.id} to={t.path} current={t.id === tab}>{t.label}</Link>)}
        </nav>
      )}
      {!allowed ? (
        <StateMessage kind="unavailable" title="Only operators and administrators can change health checks">
          <p>Your account can read alert history. <Link to="/alerts">Open alert history</Link></p>
        </StateMessage>
      ) : (
        <div className="panel">
          {tab === "alerts" && <HealthHistory {...ids} />}
          {tab === "alerts-schedules" && <HealthSchedules {...ids} reauthenticate={reauthenticate} />}
          {tab === "alerts-policy" && <HealthPolicy {...ids} />}
        </div>
      )}
    </>
  );
}
