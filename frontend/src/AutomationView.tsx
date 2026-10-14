import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { automationClient } from "./automation-client";
import { AutomationController } from "./automation-controller";
import { NotificationsView } from "./NotificationsView";
import type { AutomationState, PageKind } from "./automation-controller";
import type { AlertState, Page, ScheduleDefinition, ScheduleView } from "./automation-client";

export function AutomationView({ schedules, alerts, csrf, canManage, changed, moreSchedules, moreAlerts, failed, disabled = false }: {
  schedules: Page<ScheduleView> | null; alerts: Page<AlertState> | null; csrf: string; canManage: boolean;
  changed: () => void; moreSchedules: () => void; moreAlerts: () => void;
  failed?: (error: unknown) => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false); const [error, setError] = useState("");
  const [minutes, setMinutes] = useState(10); const [start, setStart] = useState(0); const [end, setEnd] = useState(0);
  const [windowEnabled, setWindowEnabled] = useState(false);
  const [missed, setMissed] = useState<ScheduleDefinition["missed"]>("coalesce_latest");
  const blocked = busy || disabled;
  const mutate = async (action: () => Promise<unknown>) => {
    if (blocked) return;
    setBusy(true); setError("");
    try { await action(); changed(); }
    catch (failure) { setError(failure instanceof Error ? failure.message : "Unable to save. Refresh to check the outcome."); failed?.(failure); }
    finally { setBusy(false); }
  };
  return <section aria-labelledby="automation-heading">
    <p className="eyebrow">AUTOMATION</p><h2 id="automation-heading">Schedules and alerts</h2>
    {(schedules?.partial || alerts?.partial) && <p className="notice" role="status">Some observations are unavailable. Missing observations keep active alerts open.</p>}
    {error && <p className="error" role="alert">{error}</p>}
    <div className="table-wrap"><table><caption>Observation schedules</caption><thead><tr><th scope="col">Schedule</th><th scope="col">Timing</th><th scope="col">Status</th><th scope="col">Action</th></tr></thead>
      <tbody>{schedules?.items.map(row => <tr key={row.schedule.id}><th scope="row">{row.schedule.id}<small>{row.schedule.definition.operation.replaceAll("_", " ")}</small></th>
        <td>Every {row.schedule.definition.interval_seconds / 60} minutes<small>{row.schedule.definition.window ? `UTC ${clock(row.schedule.definition.window.start_minute)}–${clock(row.schedule.definition.window.end_minute)}` : "Any time · UTC"}</small></td>
        <td><span className={`badge ${row.status === "blocked" ? "warning" : ""}`}>{row.status}</span>{row.status === "blocked" && <small>Operation adapter is unavailable.</small>}
          {row.last_occurrence && <small>Latest occurrence · {new Date(row.last_occurrence.key.due * 1000).toISOString()}<br />{row.last_occurrence.outcome.outcome === "job" ? `Job ${row.last_occurrence.outcome.id} · ${row.last_occurrence.outcome.state.replaceAll("_", " ")}` : row.last_occurrence.outcome.outcome}</small>}</td>
        <td>{canManage && <button disabled={blocked} onClick={() => void mutate(() => automationClient.pause(row, csrf))}>{row.schedule.paused ? "Resume" : "Pause"} {row.schedule.id}</button>}</td>
      </tr>)}{schedules?.items.length === 0 && <tr><td colSpan={4}>No schedules on this page.</td></tr>}{!schedules && <tr><td colSpan={4}>Schedule data is unavailable.</td></tr>}</tbody></table></div>
    {schedules?.next && <button onClick={moreSchedules} disabled={busy}>Next schedules</button>}
    {canManage && <form onSubmit={event => {
      event.preventDefault();
      if (blocked) return;
      if (windowEnabled && start === end) { setError("Choose different window start and end times."); return; }
      const definition: ScheduleDefinition = { operation: "observe_health", anchor: Math.floor(Date.now() / 1000), interval_seconds: minutes * 60,
        grace_seconds: 30, missed, window: windowEnabled ? { start_minute: start, end_minute: end } : null };
      void mutate(() => automationClient.create(definition, csrf));
    }}><h3>Create a health observation schedule</h3>
      <label htmlFor="observation-interval">Interval in minutes</label><input id="observation-interval" type="number" required min={1} max={44640} step={1} value={minutes} disabled={blocked} onChange={event => setMinutes(Number(event.target.value))} />
      <label htmlFor="missed-observations">Missed observations</label><select id="missed-observations" value={missed} disabled={blocked} onChange={event => setMissed(event.target.value as ScheduleDefinition["missed"])}><option value="coalesce_latest">Use the most recent due occurrence</option><option value="skip">Skip missed occurrences</option></select>
      <label style={{ display: "flex", alignItems: "center", gap: "0.75rem" }}><input style={{ width: "auto" }} type="checkbox" checked={windowEnabled} disabled={blocked} onChange={event => setWindowEnabled(event.target.checked)} /> Use a daily UTC maintenance window</label>
      {windowEnabled && <><label htmlFor="window-start">Window start (UTC)</label><input id="window-start" type="time" required value={clock(start)} disabled={blocked} onChange={event => setStart(minute(event.target.value))} />
        <label htmlFor="window-end">Window end (UTC)</label><input id="window-end" type="time" required value={clock(end)} disabled={blocked} onChange={event => setEnd(minute(event.target.value))} /></>}
      <button disabled={blocked}>Create observation schedule</button>
    </form>}
    <h3>Active alerts</h3><ul>{alerts?.items.filter(row => row.incident.phase === "open").map(row => <li key={row.key}><strong>{row.key}</strong>{row.incident.phase === "open" && <p>{row.incident.summary} <span className="badge warning">{row.incident.severity}</span></p>}</li>)}</ul>
    {!alerts && <p>Alert data is unavailable.</p>}
    {alerts?.items.every(row => row.incident.phase !== "open") && <p>No active alerts on this page.</p>}
    {alerts?.next && <button onClick={moreAlerts}>Next alerts</button>}
  </section>;
}
function clock(value: number): string { return `${Math.floor(value / 60).toString().padStart(2, "0")}:${(value % 60).toString().padStart(2, "0")}`; }
function minute(value: string): number { const [hours, minutes] = value.split(":").map(Number); return hours * 60 + minutes; }

/** Integrator mounts this with the existing authenticated session and current permissions. */
export function AutomationPanel({ principal, grantRevision, csrf, canManage, canAcknowledge, expired }: {
  principal: string; grantRevision: number; csrf: string; canManage: boolean; canAcknowledge: boolean; expired: () => void;
}) {
  const expiry = useRef(expired); expiry.current = expired;
  const controller = useMemo(() => new AutomationController(() => expiry.current()), [principal, grantRevision, csrf]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => {
    void controller.start();
    const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  const changed = () => { void controller.refresh(); };
  const fresh = (kind: PageKind) => state[kind].page !== null && !state[kind].loading && !state[kind].stale && !state[kind].error;
  return <>
    <button onClick={changed} disabled={state.schedules.loading && state.alerts.loading && state.notifications.loading}>Refresh automation</button>
    <AutomationReadStatus state={state} />
    <AutomationView schedules={state.schedules.page} alerts={state.alerts.page} csrf={csrf} canManage={canManage} disabled={!fresh("schedules")} changed={() => { void controller.committed("schedules"); }}
      moreSchedules={() => { void controller.next("schedules"); }} moreAlerts={() => { void controller.next("alerts"); }} failed={error => controller.mutationFailed("schedules", error)} />
    {state.schedules.after && <button disabled={state.schedules.loading} onClick={() => { void controller.first("schedules"); }}>First schedules</button>}
    {state.alerts.after && <button disabled={state.alerts.loading} onClick={() => { void controller.first("alerts"); }}>First alerts</button>}
    <NotificationsView value={state.notifications.page} csrf={csrf} canAcknowledge={canAcknowledge} disabled={!fresh("notifications")} changed={() => { void controller.committed("notifications"); }}
      more={() => { void controller.next("notifications"); }} failed={error => controller.mutationFailed("notifications", error)} />
    {state.notifications.after && <button disabled={state.notifications.loading} onClick={() => { void controller.first("notifications"); }}>First notifications</button>}
  </>;
}
export function AutomationReadStatus({ state }: { state: AutomationState }) {
  return <div aria-label="Automation connection status" aria-live="polite">
    {(["schedules", "alerts", "notifications"] as const).map(kind => <div key={kind}>
      {state[kind].loading && <p role="status">Loading {kind}…</p>}
      {state[kind].stale && <p className="notice">Previously read {kind} may be out of date. Changes are paused until a successful refresh.</p>}
      {state[kind].error && <p className="error" role="alert">{kind}: {state[kind].error}</p>}
    </div>)}
  </div>;
}
