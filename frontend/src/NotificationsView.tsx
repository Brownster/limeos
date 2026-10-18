import { deliveryLabel } from "./automation-client";
import type { Notification, Page } from "./automation-client";

export function NotificationsView({ value, canAcknowledge, acknowledge, more, error, busy = false, disabled = false }: {
  value: Page<Notification> | null; canAcknowledge: boolean; acknowledge?: (row: Notification) => void; more: () => void;
  error?: string | null; busy?: boolean; disabled?: boolean;
}) {
  return <section aria-labelledby="notifications-heading"><p className="eyebrow">NOTIFICATIONS</p><h2 id="notifications-heading">Delivery and acknowledgment</h2>
    {value?.partial && <p className="notice" role="status">Some notification state is unavailable. Refresh before retrying a delivery.</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {busy && <p role="status">Saving acknowledgment…</p>}
    <div className="table-wrap"><table><caption>Notification delivery</caption><thead><tr><th scope="col">Event</th><th scope="col">Delivery</th><th scope="col">Acknowledgment</th></tr></thead><tbody>
      {value?.items.map(row => <tr key={row.id}><th scope="row">{row.event.event === "incident" ? "Incident opened" : "Recovered"}<small>{row.event.key}</small><p>{row.event.summary}</p></th>
        <td><span className={`badge ${["failed", "ambiguous", "blocked"].includes(row.delivery.state) ? "warning" : ""}`}>{deliveryLabel(row)}</span><small>{row.channel.replaceAll("_", " ")} · {row.attempts} of 4 attempts</small></td>
        <td>{row.acknowledged_at !== null ? "Acknowledged" : canAcknowledge ? <button disabled={busy || disabled || !acknowledge} onClick={() => { if (!busy && !disabled) acknowledge?.(row); }}>Acknowledge {row.id}</button> : "Unacknowledged"}</td></tr>)}
      {value?.items.length === 0 && <tr><td colSpan={3}>No notifications on this page.</td></tr>}
      {!value && <tr><td colSpan={3}>Notification data is unavailable.</td></tr>}
    </tbody></table></div>{value?.next && <button disabled={busy} onClick={more}>Next notifications</button>}
  </section>;
}
