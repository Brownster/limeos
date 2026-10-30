import { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { AutomationController } from "./automation-controller";
import { automationClient } from "./automation-client";
import { AutomationReadStatus } from "./AutomationView";
import { NotificationsView } from "./NotificationsView";
import type { AlertState, Page } from "./automation-client";

const useOwnerEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

export function HealthHistory({ principal, grantRevision, csrf, expired }: {
  principal: string; grantRevision: number; csrf: string; expired: () => void;
}) {
  return <OwnedHistory key={JSON.stringify([principal, grantRevision, csrf])} csrf={csrf} expired={expired} />;
}

function OwnedHistory({ csrf, expired }: { csrf: string; expired: () => void }) {
  const callback = useRef(expired); callback.current = expired;
  const controller = useMemo(() => new AutomationController(() => callback.current(), automationClient, "history"), []);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useOwnerEffect(() => {
    void controller.start();
    const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => {
      window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh);
      controller.dispose();
    };
  }, [controller]);
  if (state.expired) return null;
  const notes = state.notifications;
  const mutation = state.mutations.notifications;
  const fresh = notes.page !== null && !notes.loading && !notes.stale && !notes.error;
  return <div id="health-history-panel">
    <p className="notice">New automatic health observations are not available yet. This view shows recorded alerts and in-app notifications.</p>
    <button disabled={state.alerts.loading && notes.loading} onClick={() => { void controller.refresh(); }}>Refresh alert history</button>
    <AutomationReadStatus state={state} kinds={["alerts", "notifications"]} />
    <RecordedAlerts value={state.alerts.page} busy={state.alerts.loading} more={() => { void controller.next("alerts"); }} />
    {state.alerts.after && <button disabled={state.alerts.loading} onClick={() => { void controller.first("alerts"); }}>First alerts</button>}
    <NotificationsView value={notes.page} canAcknowledge={true} disabled={!fresh || mutation.denied}
      busy={mutation.pending} error={mutation.error}
      acknowledge={row => { void controller.mutate("notifications", signal => automationClient.acknowledge(row, csrf, signal)); }}
      more={() => { void controller.next("notifications"); }} />
    {notes.after && <button disabled={notes.loading} onClick={() => { void controller.first("notifications"); }}>First notifications</button>}
  </div>;
}

export function RecordedAlerts({ value, busy = false, more }: { value: Page<AlertState> | null; busy?: boolean; more: () => void }) {
  return <section aria-labelledby="recorded-alerts-heading">
    <p className="eyebrow">ALERT HISTORY</p><h2 id="recorded-alerts-heading">Recorded alerts</h2>
    {value?.partial && <p className="notice" role="status">Some observations are unavailable. Existing incidents stay open until a verified recovery.</p>}
    <div className="table-wrap" tabIndex={0} role="region" aria-label="Recorded alert states">
      <table className="recorded-alerts-table"><caption>Recorded alert states</caption><thead><tr><th scope="col">Resource</th><th scope="col">Recorded state</th></tr></thead>
        <tbody>{value?.items.map(row => <tr key={row.key}><th scope="row">{row.key}</th>
          <td>{row.incident.phase === "open" ? <><span className="badge warning">Open incident · {row.incident.severity}</span><p>{row.incident.summary}</p></> : <>No open incident{row.incident.failures > 0 && <small>{row.incident.failures} unsuccessful observation{row.incident.failures === 1 ? "" : "s"}</small>}</>}</td></tr>)}
          {value?.items.length === 0 && <tr><td colSpan={2}>No alert history on this page.</td></tr>}
          {!value && <tr><td colSpan={2}>Alert data is unavailable.</td></tr>}
        </tbody>
      </table>
    </div>
    {value?.next && <button disabled={busy} onClick={more}>Next alerts</button>}
  </section>;
}
