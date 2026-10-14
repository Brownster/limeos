import { useState } from "react";
import { automationClient, deliveryLabel } from "./automation-client";
import type { Notification, Page } from "./automation-client";

export function NotificationsView({ value, csrf, canAcknowledge, changed, more }: {
  value: Page<Notification>; csrf: string; canAcknowledge: boolean; changed: () => void; more: () => void;
}) {
  const [busy, setBusy] = useState(false); const [error, setError] = useState("");
  const acknowledge = async (row: Notification) => {
    setBusy(true); setError("");
    try { await automationClient.acknowledge(row, csrf); changed(); }
    catch (failure) { setError(failure instanceof Error ? failure.message : "Acknowledgment failed. Refresh to check the outcome."); }
    finally { setBusy(false); }
  };
  return <section aria-labelledby="notifications-heading"><p className="eyebrow">NOTIFICATIONS</p><h2 id="notifications-heading">Delivery and acknowledgment</h2>
    {value.partial && <p className="notice" role="status">Some notification state is unavailable. Refresh before retrying a delivery.</p>}
    {error && <p className="error" role="alert">{error}</p>}
    <div className="table-wrap"><table><caption>Notification delivery</caption><thead><tr><th scope="col">Event</th><th scope="col">Delivery</th><th scope="col">Acknowledgment</th></tr></thead><tbody>
      {value.items.map(row => <tr key={row.id}><th scope="row">{row.event.event === "incident" ? "Incident opened" : "Recovered"}<small>{row.event.key}</small><p>{row.event.summary}</p></th>
        <td><span className={`badge ${["failed", "ambiguous", "blocked"].includes(row.delivery.state) ? "warning" : ""}`}>{deliveryLabel(row)}</span><small>{row.channel.replaceAll("_", " ")} · {row.attempts} of 4 attempts</small></td>
        <td>{row.acknowledged_at !== null ? "Acknowledged" : canAcknowledge ? <button disabled={busy} onClick={() => void acknowledge(row)}>Acknowledge {row.id}</button> : "Unacknowledged"}</td></tr>)}
      {value.items.length === 0 && <tr><td colSpan={3}>No notifications on this page.</td></tr>}
    </tbody></table></div>{value.next && <button disabled={busy} onClick={more}>Next notifications</button>}
  </section>;
}
