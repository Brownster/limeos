import { useState } from "react";
import { automationClient } from "./automation-client";
import type { AlertState, Page, ScheduleDefinition, ScheduleView } from "./automation-client";

export function AutomationView({ schedules, alerts, csrf, canManage, changed, moreSchedules, moreAlerts }: {
  schedules: Page<ScheduleView>; alerts: Page<AlertState>; csrf: string; canManage: boolean;
  changed: () => void; moreSchedules: () => void; moreAlerts: () => void;
}) {
  const [busy, setBusy] = useState(false); const [error, setError] = useState("");
  const [minutes, setMinutes] = useState(10); const [start, setStart] = useState(0); const [end, setEnd] = useState(0);
  const [windowEnabled, setWindowEnabled] = useState(false);
  const [missed, setMissed] = useState<ScheduleDefinition["missed"]>("coalesce_latest");
  const mutate = async (action: () => Promise<unknown>) => {
    setBusy(true); setError("");
    try { await action(); changed(); }
    catch (failure) { setError(failure instanceof Error ? failure.message : "Unable to save. Refresh to check the outcome."); }
    finally { setBusy(false); }
  };
  return <section aria-labelledby="automation-heading">
    <p className="eyebrow">AUTOMATION</p><h2 id="automation-heading">Schedules and alerts</h2>
    {(schedules.partial || alerts.partial) && <p className="notice" role="status">Some observations are unavailable. Missing observations keep active alerts open.</p>}
    {error && <p className="error" role="alert">{error}</p>}
    <div className="table-wrap"><table><caption>Observation schedules</caption><thead><tr><th scope="col">Schedule</th><th scope="col">Timing</th><th scope="col">Status</th><th scope="col">Action</th></tr></thead>
      <tbody>{schedules.items.map(row => <tr key={row.schedule.id}><th scope="row">{row.schedule.id}<small>{row.schedule.definition.operation.replaceAll("_", " ")}</small></th>
        <td>Every {row.schedule.definition.interval_seconds / 60} minutes<small>{row.schedule.definition.window ? `UTC ${clock(row.schedule.definition.window.start_minute)}–${clock(row.schedule.definition.window.end_minute)}` : "Any time · UTC"}</small></td>
        <td><span className={`badge ${row.status === "blocked" ? "warning" : ""}`}>{row.status}</span>{row.status === "blocked" && <small>Operation adapter is unavailable.</small>}
          {row.last_occurrence && <small>Latest occurrence · {new Date(row.last_occurrence.key.due * 1000).toISOString()}<br />{row.last_occurrence.outcome.outcome === "job" ? `Job ${row.last_occurrence.outcome.id} · ${row.last_occurrence.outcome.state.replaceAll("_", " ")}` : row.last_occurrence.outcome.outcome}</small>}</td>
        <td>{canManage && <button disabled={busy} onClick={() => void mutate(() => automationClient.pause(row, csrf))}>{row.schedule.paused ? "Resume" : "Pause"} {row.schedule.id}</button>}</td>
      </tr>)}{schedules.items.length === 0 && <tr><td colSpan={4}>No schedules on this page.</td></tr>}</tbody></table></div>
    {schedules.next && <button onClick={moreSchedules} disabled={busy}>Next schedules</button>}
    {canManage && <form onSubmit={event => {
      event.preventDefault();
      if (windowEnabled && start === end) { setError("Choose different window start and end times."); return; }
      const definition: ScheduleDefinition = { operation: "observe_health", anchor: Math.floor(Date.now() / 1000), interval_seconds: minutes * 60,
        grace_seconds: 30, missed, window: windowEnabled ? { start_minute: start, end_minute: end } : null };
      void mutate(() => automationClient.create(definition, csrf));
    }}><h3>Create a health observation schedule</h3>
      <label htmlFor="observation-interval">Interval in minutes</label><input id="observation-interval" type="number" required min={1} max={44640} step={1} value={minutes} disabled={busy} onChange={event => setMinutes(Number(event.target.value))} />
      <label htmlFor="missed-observations">Missed observations</label><select id="missed-observations" value={missed} disabled={busy} onChange={event => setMissed(event.target.value as ScheduleDefinition["missed"])}><option value="coalesce_latest">Use the most recent due occurrence</option><option value="skip">Skip missed occurrences</option></select>
      <label style={{ display: "flex", alignItems: "center", gap: "0.75rem" }}><input style={{ width: "auto" }} type="checkbox" checked={windowEnabled} disabled={busy} onChange={event => setWindowEnabled(event.target.checked)} /> Use a daily UTC maintenance window</label>
      {windowEnabled && <><label htmlFor="window-start">Window start (UTC)</label><input id="window-start" type="time" required value={clock(start)} disabled={busy} onChange={event => setStart(minute(event.target.value))} />
        <label htmlFor="window-end">Window end (UTC)</label><input id="window-end" type="time" required value={clock(end)} disabled={busy} onChange={event => setEnd(minute(event.target.value))} /></>}
      <button disabled={busy}>Create observation schedule</button>
    </form>}
    <h3>Active alerts</h3><ul>{alerts.items.filter(row => row.incident.phase === "open").map(row => <li key={row.key}><strong>{row.key}</strong>{row.incident.phase === "open" && <p>{row.incident.summary} <span className="badge warning">{row.incident.severity}</span></p>}</li>)}</ul>
    {alerts.items.every(row => row.incident.phase !== "open") && <p>No active alerts on this page.</p>}
    {alerts.next && <button onClick={moreAlerts}>Next alerts</button>}
  </section>;
}
function clock(value: number): string { return `${Math.floor(value / 60).toString().padStart(2, "0")}:${(value % 60).toString().padStart(2, "0")}`; }
function minute(value: string): number { const [hours, minutes] = value.split(":").map(Number); return hours * 60 + minutes; }
