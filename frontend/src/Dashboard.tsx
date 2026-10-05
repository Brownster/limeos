import { useEffect, useState } from "react";
import type {
  Overview,
  ResourceKind,
  MetricHistory,
  HistoryRange,
} from "../../contracts/generated/types";
import { api, ApiError } from "./api";
import { page } from "./model";
import { ObservationView } from "./ObservationView";
const views: [string, ResourceKind | undefined][] = [
  ["All resources", undefined],
  ["Containers", "container"],
  ["Stacks", "stack"],
  ["Disks", "disk"],
  ["Partitions", "partition"],
  ["Pools", "pool"],
];
export function Dashboard({ expired }: { expired: () => void }) {
  const [value, setValue] = useState<Overview | null>(null);
  const [error, setError] = useState("");
  const [connected, setConnected] = useState(false);
  const [kind, setKind] = useState<ResourceKind | undefined>();
  const [offset, setOffset] = useState(0);
  const [range, setRange] = useState<HistoryRange>("24h");
  const [history, setHistory] = useState<MetricHistory | null>(null);
  const [historyError, setHistoryError] = useState("");
  useEffect(() => {
    let active = true;
    const fail = (e: unknown) => {
      if (!active) return;
      if (e instanceof ApiError && e.detail.code === "unauthenticated") {
        expired();
        return;
      }
      setError(
        e instanceof ApiError
          ? `${e.message} · ${e.detail.code} · reference ${e.detail.audit_id}`
          : "Unable to read host observations.",
      );
    };
    const refresh = () => {
      void api
        .overview()
        .then((v) => {
          if (active) {
            setValue(v);
            setError("");
          }
        })
        .catch(fail);
    };
    refresh();
    let stream: EventSource | null = null;
    const connect = () => {
      stream?.close();
      stream = new EventSource("/api/v1/observations/stream");
      stream.addEventListener("snapshot", (event) => {
        if (!active) return;
        try {
          const next = JSON.parse(
            (event as MessageEvent<string>).data,
          ) as Overview;
          setValue(next);
          setConnected(true);
          setError("");
        } catch {
          setError("The host returned an invalid observation.");
        }
      });
      stream.onerror = () => {
        if (active) {
          setConnected(false);
          refresh();
        }
      };
    };
    if (!document.hidden) connect();
    const visibility = () => {
      if (document.hidden) {
        stream?.close();
        setConnected(false);
      } else {
        refresh();
        connect();
      }
    };
    document.addEventListener("visibilitychange", visibility);
    // Also refresh freshness/session status when a proxy cannot carry SSE.
    const timer = window.setInterval(() => {
      if (!document.hidden) refresh();
    }, 30_000);
    return () => {
      active = false;
      stream?.close();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [expired]);
  useEffect(() => {
    let active = true;
    setHistory(null);
    setHistoryError("");
    void api
      .history(range)
      .then((h) => {
        if (active) setHistory(h);
      })
      .catch((e: unknown) => {
        if (active)
          setHistoryError(
            e instanceof ApiError
              ? `${e.message} · ${e.detail.code} · reference ${e.detail.audit_id}`
              : "History is unavailable.",
          );
      });
    return () => {
      active = false;
    };
  }, [range]);
  const count =
    value?.resources.filter((r) => kind === undefined || r.kind === kind)
      .length ?? 0;
  const safeOffset = offset < count ? offset : 0;
  return (
    <div className="dashboard">
      <header className="page-heading">
        <div>
          <p className="eyebrow">HOST OBSERVATIONS</p>
          <h1>At home.</h1>
          <p className="intro">Your services and storage, in one place.</p>
        </div>
        <span className="badge">
          {connected ? "Live updates" : "Reconnecting"}
        </span>
      </header>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {!value ? (
        <p role="status">
          {error
            ? "Observations are unavailable. Reconnecting to the host…"
            : "Loading host observations…"}
        </p>
      ) : (
        <>
          <nav className="resource-nav" aria-label="Resource views">
            {views.map(([label, k]) => (
              <button
                key={label}
                className={kind === k ? "selected" : ""}
                aria-pressed={kind === k}
                onClick={() => {
                  setKind(k);
                  setOffset(0);
                }}
              >
                {label}
              </button>
            ))}
          </nav>
          <ObservationView
            value={value}
            items={page(value.resources, kind, safeOffset)}
            history={history}
          />
          <div className="pager">
            <button
              disabled={safeOffset === 0}
              onClick={() => setOffset(Math.max(0, safeOffset - 20))}
            >
              Previous
            </button>
            <span>
              {count === 0
                ? "0 resources"
                : `${safeOffset + 1}–${Math.min(safeOffset + 20, count)} of ${count}`}
            </span>
            <button
              disabled={safeOffset + 20 >= count}
              onClick={() => setOffset(safeOffset + 20)}
            >
              Next
            </button>
          </div>
          <label className="range">
            History range
            <select
              value={range}
              onChange={(e) => setRange(e.target.value as HistoryRange)}
            >
              <option value="24h">24 hours</option>
              <option value="7d">7 days</option>
              <option value="30d">30 days</option>
            </select>
          </label>
          {!history && !historyError && <p role="status">Loading history…</p>}
          {historyError && (
            <p className="error" role="alert">
              {historyError}
            </p>
          )}
        </>
      )}
    </div>
  );
}
