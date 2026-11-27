import type {
  Overview,
  Resource,
  MetricHistory,
} from "../../contracts/generated/types";
import { percent, sampled, stale, partial } from "./model";
import { ContainerActions } from "./ContainerActions";

/** Said whenever any source is stale, unavailable or warns, or a resource is missing. */
export function PartialNotice({ value }: { value: Overview }) {
  return partial(value) ? (
    <p className="notice" role="status">
      Some observations are incomplete or out of date. Check the source
      details below.
    </p>
  ) : null;
}

export function SourceList({ value }: { value: Overview }) {
  return (
    <section className="source-list flush" aria-label="Observation sources">
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
      {value.sources.length === 0 && <p>No observation sources reported yet.</p>}
    </section>
  );
}

/** The observed-resources table, with container lifecycle controls for managers. */
export function ResourceTable({
  items,
  csrf = "",
  canManage = false,
  changed = () => {},
  caption = "Observed resources",
  empty = "No resources in this view. Source availability is shown on the Overview.",
}: {
  items: Resource[];
  csrf?: string;
  canManage?: boolean;
  changed?: () => void;
  caption?: string;
  empty?: string;
}) {
  return (
    <div className="table-wrap">
      <table>
        <caption>{caption}</caption>
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
                {r.kind === "container" && canManage && (
                  <ContainerActions
                    resource={r}
                    csrf={csrf}
                    changed={changed}
                  />
                )}
              </td>
            </tr>
          ))}
          {items.length === 0 && (
            <tr>
              <td colSpan={3}>{empty}</td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

/** The numbers behind the history charts, for screen readers and exact values. */
export function HistoryTable({ history, limit = 48 }: { history: MetricHistory; limit?: number }) {
  const rows = history.points.slice(-limit).reverse();
  return (
    <>
      <p className="notice">{history.legacy_history}</p>
      <p>
        {history.points.length} reporting intervals ·{" "}
        {history.bucket_seconds / 60} minute averages
      </p>
      <div className="table-wrap" tabIndex={0} role="region" aria-label="History values">
        <table>
          <caption>Recent history · latest {Math.min(limit, history.points.length)} intervals, newest first</caption>
          <thead>
            <tr>
              <th scope="col">Time</th>
              <th scope="col">CPU</th>
              <th scope="col">Memory</th>
              <th scope="col">Temperature</th>
              <th scope="col">Root disk</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr key={p.at}>
                <th scope="row">{new Date(p.at * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}</th>
                <td>{percent(p.cpu_percent)}</td>
                <td>{percent(p.memory_percent)}</td>
                <td>{p.temperature_celsius === null || p.temperature_celsius === undefined || !Number.isFinite(p.temperature_celsius) ? "Unknown" : `${p.temperature_celsius.toFixed(1)} °C`}</td>
                <td>{percent(p.disk_percent)}</td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={5}>No intervals recorded in this range yet.</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}
