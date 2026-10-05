import type {
  Overview,
  Resource,
  MetricHistory,
} from "../../contracts/generated/types";
import { percent, sampled, stale, partial } from "./model";
export function ObservationView({
  value,
  items,
  history,
}: {
  value: Overview;
  items: Resource[];
  history: MetricHistory | null;
}) {
  const metrics = value.host;
  return (
    <>
      {partial(value) && (
        <p className="notice" role="status">
          Some observations are incomplete or out of date. Check the source
          details below.
        </p>
      )}
      <dl className="metrics">
        {[
          ["CPU", percent(metrics?.cpu_percent ?? null)],
          ["Memory", percent(metrics?.memory_percent ?? null)],
          ["Root disk", percent(metrics?.disk_percent ?? null)],
          [
            "Temperature",
            metrics?.temperature_celsius === null ||
            metrics?.temperature_celsius === undefined
              ? "Unknown"
              : `${metrics.temperature_celsius.toFixed(1)} °C`,
          ],
        ].map(([name, result]) => (
          <div key={name}>
            <dt>{name}</dt>
            <dd>{result}</dd>
          </div>
        ))}
      </dl>
      <section className="source-list" aria-label="Observation sources">
        {value.sources.map((s) => (
          <details key={s.source}>
            <summary>
              <span>{s.source}</span>
              <span className={`badge ${stale(s) ? "warning" : ""}`}>
                {stale(s) && s.state === "fresh" ? "stale" : s.state}
              </span>
            </summary>
            <p>
              Sampled {sampled(s.sampled_at)} · freshness limit{" "}
              {s.max_age_seconds}s
            </p>
            {s.warnings.map((w, i) => (
              <p className="notice" key={i}>
                {w}
              </p>
            ))}
          </details>
        ))}
      </section>
      <div className="table-wrap">
        <table>
          <caption>Observed resources</caption>
          <thead>
            <tr>
              <th scope="col">Resource</th>
              <th scope="col">Status</th>
              <th scope="col">Details</th>
            </tr>
          </thead>
          <tbody>
            {items.map((r) => (
              <tr key={r.id}>
                <th scope="row">
                  {r.name}
                  <small>{r.kind}</small>
                </th>
                <td>
                  <span
                    className={`badge ${["missing", "degraded", "dead"].includes(r.status) ? "warning" : ""}`}
                  >
                    {r.status}
                  </span>
                  {r.health && <small>{r.health}</small>}
                </td>
                <td>
                  {r.image ?? r.mountpoint ?? r.filesystem ?? "—"}
                  <small>
                    {r.identity ?? r.id}
                    {r.kind === "container" &&
                      ` · CPU ${percent(r.cpu_percent)} · memory ${percent(r.memory_percent)}`}
                  </small>
                </td>
              </tr>
            ))}
            {items.length === 0 && (
              <tr>
                <td colSpan={3}>
                  No resources in this view. Source availability is shown above.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {history && (
        <section aria-labelledby="history-title">
          <h2 id="history-title">System history</h2>
          <p className="notice">{history.legacy_history}</p>
          <p>
            {history.points.length} reporting intervals ·{" "}
            {history.bucket_seconds / 60} minute averages
          </p>
          <div className="table-wrap">
            <table>
              <caption>Recent history · latest 12 intervals</caption>
              <thead>
                <tr>
                  <th scope="col">Time</th>
                  <th scope="col">CPU</th>
                  <th scope="col">Memory</th>
                  <th scope="col">Root disk</th>
                </tr>
              </thead>
              <tbody>
                {history.points.slice(-12).map((p) => (
                  <tr key={p.at}>
                    <th scope="row">{sampled(p.at)}</th>
                    <td>{percent(p.cpu_percent)}</td>
                    <td>{percent(p.memory_percent)}</td>
                    <td>{percent(p.disk_percent)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}
    </>
  );
}
