import { activityExpired } from "./media-activity-model";
import type { ActivitySnapshot, ServiceState } from "./media-activity-model";

const states: Record<ServiceState, string> = { ready: "Observed", partial: "Partial observation", stopped: "Service stopped", stale: "Previous observation", future: "Sample clock mismatch", identity_mismatch: "Container identity changed", unavailable: "Unavailable", credential_unavailable: "Service credential unavailable", timeout: "Service timed out", invalid_response: "Unsupported or invalid response", unsupported: "Unsupported provider" };
const known = (n: number | null) => n !== null && Number.isFinite(n) && n >= 0;
const count = (n: number | null) => known(n) ? String(n) : "Unknown";
const rate = (n: number | null) => known(n) ? `${(n! / 1048576).toFixed(1)} MiB/s` : "Unknown rate";
const size = (n: number | null) => known(n) ? `${(n! / 1073741824).toFixed(1)} GiB` : "Unknown size";
const sample = (n: number | null) => n === null ? "No sample" : new Date(n * 1000).toISOString();

/** Presentational leaf. Parent owns authenticated cache reads, admission, session
 * clearing and a one-shot expiry rerender; `now` must advance without polling. */
export function MediaActivityView({ value, now, loading = false, stale = false, error = "", refresh }: {
  value: ActivitySnapshot | null; now: number; loading?: boolean; stale?: boolean; error?: string; refresh: () => void;
}) {
  const previous = value !== null && (stale || activityExpired(value, now));
  const source = (resource: string) => value?.services.find(row => row.target.container.resource === resource)?.target.name ?? "Unknown service";
  const totals = value && !previous && value.complete ? value.totals : null;
  return <section aria-labelledby="media-activity-heading" aria-busy={loading}>
    <p className="eyebrow">MEDIA · ACTIVITY</p><h2 id="media-activity-heading">What’s happening</h2>
    <p>Streams, transfers and queue problems reported by your media services.</p>
    <button onClick={refresh} disabled={loading}>Refresh activity</button>
    {loading && <p role="status">Reading activity…</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {!value && !loading && <p>Media activity is unavailable.</p>}
    {value && <>
      <p><small>Sample · {sample(value.sampled_at)}</small></p>
      {previous && <p className="notice" role="status">Previous observation. Current activity and totals are unknown.</p>}
      {!previous && !value.complete && <p className="notice" role="status">Some activity could not be confirmed. Available services are shown; aggregate totals are unknown.</p>}
      <dl className="source-list" aria-label="Activity totals">
        <div><dt>Streams / transcodes</dt><dd>{count(totals?.streams ?? null)} / {count(totals?.transcodes ?? null)}</dd></div>
        <div><dt>Transfers</dt><dd>{count(totals?.downloads ?? null)} · {rate(totals?.download_speed ?? null)}</dd></div>
        <div><dt>Queued / problems</dt><dd>{count(totals?.queued ?? null)} / {count(totals?.queue_problems ?? null)}</dd></div>
      </dl>
      <h3>{previous ? "Previously observed streams" : "Streams"}</h3>
      {value.streams.length === 0 && <p>{!previous && value.complete ? "No streams observed." : "Current stream activity could not be confirmed."}</p>}
      {value.streams.map((row, index) => <article key={`${row.source}:${index}`}>
        <h4>{row.data.title} <span className="badge warning">{previous ? "Previous" : row.data.paused ? "Paused" : "Playing"}</span></h4>
        <p>{row.data.subtitle}<br /><small>{source(row.source)} · {row.data.user || "Unknown viewer"} · {row.data.client || "Unknown client"} / {row.data.device || "Unknown device"}</small></p>
        <Progress title={row.data.title} percent={previous ? null : row.data.progress_percent} />
        <p>{row.data.transcoding ? "Transcoding" : "Direct playback"}{row.data.transcode_reason && ` · ${row.data.transcode_reason}`}{previous && " · previous observation"}</p>
      </article>)}
      {value.omitted_streams > 0 && <p>{value.omitted_streams} additional observed streams are outside this view.</p>}
      <h3>{previous ? "Previously observed transfers" : "Transfers"}</h3>
      {value.downloads.length === 0 && <p>{!previous && value.complete ? "No transfers observed." : "Current transfer activity could not be confirmed."}</p>}
      {value.downloads.length > 0 && <div className="table-wrap"><table><caption>Media transfers · {previous ? "previous observation" : "service observations"}</caption>
        <thead><tr><th scope="col">Item</th><th scope="col">State</th><th scope="col">Progress</th><th scope="col">Rate / remaining</th></tr></thead>
        <tbody>{value.downloads.map((row, index) => <tr key={`${row.source}:${index}`}><th scope="row">{row.data.name}<small>{source(row.source)}</small></th><td>{previous ? "Previously " : ""}{row.data.state}</td>
          <td><Progress title={row.data.name} percent={previous ? null : row.data.progress_percent} /></td><td>{rate(previous ? null : row.data.speed_bytes)}<small>{size(previous ? null : row.data.remaining_bytes)} · {previous ? "Unknown ETA" : row.data.eta ?? "Unknown ETA"}</small></td></tr>)}</tbody>
      </table></div>}
      {value.omitted_downloads > 0 && <p>{value.omitted_downloads} additional observed transfers are outside this view.</p>}
      <h3>Service queues</h3>
      {value.queues.map(row => <article key={row.source}><h4>{source(row.source)}</h4>
        <p>{previous ? "Current queue counts are unknown." : `${row.data.total} queued · ${row.data.problems} problems in ${row.data.observed} observed items${row.data.complete ? "" : " · partial page"}`}</p>
        {row.data.observed > row.data.items.length && <p><small>Showing {row.data.items.length} of {row.data.observed} inspected items.</small></p>}
        <ul>{row.data.items.map((item, index) => <li key={index}>{item.title} · {item.state} / {item.status}{previous && " · previous observation"}</li>)}</ul>
      </article>)}
      <details open={value.services.some(row => row.state !== "ready")}><summary>Service observations · {value.services.length}</summary>
        <ul>{value.services.map(row => <li key={row.target.container.resource}>{row.target.name} · {previous ? "Previous observation" : states[row.state]}<small>{sample(row.sampled_at)}</small></li>)}</ul>
        {value.services.length === 0 && <p>{!previous && value.complete ? "No supported media services were discovered." : "Media service discovery could not be confirmed."}</p>}
      </details>
    </>}
  </section>;
}
function Progress({ title, percent }: { title: string; percent: number | null }) {
  return known(percent) && percent! <= 100 ? <span><progress aria-label={`${title} progress`} max={100} value={percent!} /> {percent!.toFixed(1)}%</span> : <span>Progress unknown</span>;
}
