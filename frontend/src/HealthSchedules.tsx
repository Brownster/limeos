import { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { AutomationController } from "./automation-controller";
import { automationClient } from "./automation-client";
import { AutomationReadStatus, AutomationView } from "./AutomationView";
import { HealthEnablementController } from "./health-enablement-controller";

const useOwnerEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;
export function HealthSchedules({ principal, grantRevision, csrf, expired, reauthenticate }: {
  principal: string; grantRevision: number; csrf: string; expired: () => void; reauthenticate: () => void;
}) {
  return <OwnedHealthSchedules key={JSON.stringify([principal, grantRevision, csrf])} csrf={csrf} expired={expired} reauthenticate={reauthenticate} />;
}
function OwnedHealthSchedules({ csrf, expired, reauthenticate }: { csrf: string; expired: () => void; reauthenticate: () => void }) {
  const callbacks = useRef({ expired, reauthenticate }); callbacks.current = { expired, reauthenticate };
  const controllers = useMemo(() => {
    const schedules = new AutomationController(() => {
      enablement.dispose(); callbacks.current.expired();
    }, automationClient, "schedules");
    const enablement = new HealthEnablementController(result => {
      if (result?.reauthenticate) {
        schedules.dispose(); enablement.dispose(); callbacks.current.reauthenticate();
      } else void schedules.start();
    }, () => { schedules.dispose(); callbacks.current.expired(); });
    return { schedules, enablement };
  }, []);
  const { schedules, enablement } = controllers;
  const state = useSyncExternalStore(schedules.subscribe, schedules.getSnapshot, schedules.getSnapshot);
  const permission = useSyncExternalStore(enablement.subscribe, enablement.getSnapshot, enablement.getSnapshot);
  useOwnerEffect(() => {
    enablement.start(); void schedules.start();
    const refresh = () => { if (!enablement.getSnapshot().pending) void schedules.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); schedules.dispose(); enablement.dispose(); };
  }, [schedules, enablement]);
  if (state.expired) return null;
  const fresh = state.schedules.page !== null && !state.schedules.loading && !state.schedules.stale && !state.schedules.error;
  const saving = state.mutations.schedules.pending;
  return <div id="health-schedules-panel">
    <p className="notice">You can save or pause schedules. Automatic health observations are not running yet.</p>
    <button disabled={state.schedules.loading || permission.pending} onClick={() => { void schedules.refresh(); }}>Refresh schedules</button>
    <AutomationReadStatus state={state} kinds={["schedules"]} />
    <AutomationView showAlerts={false} schedules={state.schedules.page} alerts={null} canManage={true}
      busy={saving} disabled={!fresh || permission.pending || state.mutations.schedules.denied} error={state.mutations.schedules.error}
      create={input => { if (!enablement.getSnapshot().pending) void schedules.mutate("schedules", signal => automationClient.create(input, csrf, signal)); }}
      pause={row => { if (!enablement.getSnapshot().pending) void schedules.mutate("schedules", signal => automationClient.pause(row, csrf, signal)); }}
      moreSchedules={() => { void schedules.next("schedules"); }} moreAlerts={() => {}} />
    {state.schedules.after && <button disabled={state.schedules.loading || permission.pending} onClick={() => { void schedules.first("schedules"); }}>First schedules</button>}
    <section aria-labelledby="schedule-permission-heading">
      <h2 id="schedule-permission-heading">Scheduling permission</h2>
      <p>Use your current password to enable health scheduling for your account. Enabling a new permission requires signing in again.</p>
      {permission.error && <p className="error" role="alert">{permission.error}</p>}
      {permission.confirmed ? <p role="status">Scheduling permission confirmed.</p> : <form onSubmit={event => {
        event.preventDefault();
        if (schedules.getSnapshot().mutations.schedules.pending || enablement.getSnapshot().pending) return;
        const form = event.currentTarget; const password = String(new FormData(form).get("scheduling-password") ?? ""); form.reset();
        schedules.dispose(); void enablement.confirm(password, csrf);
      }} aria-busy={permission.pending}>
        <label htmlFor="scheduling-password">Current password</label>
        <input id="scheduling-password" name="scheduling-password" type="password" autoComplete="current-password" required maxLength={1024} disabled={permission.pending || saving} />
        <button disabled={permission.pending || saving}>{permission.pending ? "Confirming permission…" : "Enable health scheduling"}</button>
      </form>}
    </section>
  </div>;
}
