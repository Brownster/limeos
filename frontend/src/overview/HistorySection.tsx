/**
 * Performance history from GET /api/v1/system/history: read once per range
 * and on manual refresh (never polled).
 */
import { useEffect, useState } from "react";
import type { HistoryRange, MetricHistory } from "../../../contracts/generated/types";
import { api, ApiError } from "../api";
import { HistoryTable } from "../ObservationView";
import { StateMessage } from "../ui/primitives";
import { HistoryChart } from "./HistoryChart";

const ranges: { value: HistoryRange; label: string; long: string }[] = [
  { value: "24h", label: "24h", long: "24 hours" },
  { value: "7d", label: "7d", long: "7 days" },
  { value: "30d", label: "30d", long: "30 days" },
];

export function HistorySection({ refreshKey, expired }: { refreshKey: number; expired: () => void }) {
  const [range, setRange] = useState<HistoryRange>("24h");
  const [history, setHistory] = useState<MetricHistory | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    void api
      .history(range)
      .then((next) => { if (active) setHistory(next); })
      .catch((e: unknown) => {
        if (!active) return;
        if (e instanceof ApiError && e.detail.code === "unauthenticated") expired();
        else setError(e);
      })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [range, refreshKey, attempt, expired]);
  const shown = history && (history.range ?? range) === range ? history : null;
  const minutes = shown ? shown.bucket_seconds / 60 : null;
  return (
    <section className="history" aria-labelledby="history-title">
      <div className="section-head">
        <div>
          <h2 id="history-title">Performance history</h2>
          <p className="section-note">
            {shown ? `${shown.points.length} samples · ${minutes! >= 60 ? `${minutes! / 60} hour` : `${minutes} minute`} averages` : loading ? "Loading samples…" : "No samples"}
          </p>
        </div>
        <div className="segmented" role="group" aria-label="History range">
          {ranges.map((r) => (
            <button key={r.value} type="button" aria-pressed={range === r.value} title={r.long} onClick={() => setRange(r.value)}>{r.label}</button>
          ))}
        </div>
      </div>
      {error ? (
        <StateMessage kind="error" error={error} subject="Performance history" onRetry={() => setAttempt((a) => a + 1)} />
      ) : !shown ? (
        <StateMessage kind="loading" title="Loading performance history…" />
      ) : shown.points.length === 0 ? (
        <StateMessage kind="empty" title="No history yet">
          <p>History appears after the first samples are collected.</p>
          {shown.legacy_history && <p>{shown.legacy_history}</p>}
        </StateMessage>
      ) : (
        <HistoryBody history={shown} range={range} />
      )}
    </section>
  );
}

function HistoryBody({ history, range }: { history: MetricHistory; range: HistoryRange }) {
  return (
    <>
      <div className="chart-grid-layout">
        <HistoryChart title="CPU and memory" icon="cpu" history={history} range={range} unit="%"
          series={[{ key: "cpu_percent", label: "CPU", tone: "info" }, { key: "memory_percent", label: "Memory", tone: "success", dashed: true }]} />
        <HistoryChart title="Temperature" icon="temperature" history={history} range={range} unit=" °C"
          series={[{ key: "temperature_celsius", label: "Temperature", tone: "warning" }]} />
        <div className="chart-wide">
          <HistoryChart title="Root disk" icon="disk" history={history} range={range} unit="%"
            series={[{ key: "disk_percent", label: "Root disk", tone: "accent" }]} />
        </div>
      </div>
      <details className="history-table">
        <summary>Show the numbers</summary>
        <HistoryTable history={history} />
      </details>
    </>
  );
}
