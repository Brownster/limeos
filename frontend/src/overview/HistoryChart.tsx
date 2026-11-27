/**
 * A dependency-free SVG line chart, ported from the previous app's
 * performance history: gridlines, one or two series, a pointer/keyboard
 * cursor with a tooltip, and a now/min/avg/max summary. Series differ by
 * line style as well as colour, and the legend names them.
 */
import { useState, type KeyboardEvent, type PointerEvent } from "react";
import type { HistoryRange, MetricHistory } from "../../../contracts/generated/types";
import { Icon, type IconName } from "../ui/icons";
import { Card, CardTitle } from "../ui/primitives";
import { axisTimes, CHART_HEIGHT, CHART_WIDTH, domainFor, nearestIndex, seriesPaths, summarize, timeSpan, xFor, type SeriesKey } from "./chart-model";

export interface Series { key: SeriesKey; label: string; tone: "info" | "success" | "warning" | "accent"; dashed?: boolean }

const finite = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

function format(value: number | null | undefined, unit: string) {
  return finite(value) ? `${value.toFixed(1)}${unit}` : "—";
}

function axisLabel(at: number, range: HistoryRange) {
  const date = new Date(at * 1000);
  return range === "24h"
    ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString([], { day: "numeric", month: "short" });
}

function tooltipTime(at: number) {
  return new Date(at * 1000).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

const rangeWords: Record<HistoryRange, string> = { "24h": "the last 24 hours", "7d": "the last 7 days", "30d": "the last 30 days" };

export function HistoryChart({ title, icon, history, range, series, unit }: {
  title: string; icon: IconName; history: MetricHistory; range: HistoryRange; series: Series[]; unit: "%" | " °C";
}) {
  const points = history.points;
  const [active, setActive] = useState<number | null>(null);
  const [keyboard, setKeyboard] = useState(false);
  const span = timeSpan(points, history.from, history.to);
  const domain = domainFor(series[0].key, points);
  const point = active === null ? null : points[active] ?? null;
  const ratio = point ? xFor(point.at, span, 1) : 0;
  const summaries = series.map((s) => ({ ...s, summary: summarize(points, s.key) }));
  const description = `${title} over ${rangeWords[range]}. ` + summaries
    .map((s) => `${s.label}: now ${format(s.summary.current, unit)}, average ${format(s.summary.average, unit)}, highest ${format(s.summary.max, unit)}`)
    .join(". ") + ". Use the left and right arrow keys to read individual samples.";

  const move = (event: PointerEvent<SVGSVGElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    setKeyboard(false);
    setActive(nearestIndex(points, (event.clientX - bounds.left) / Math.max(bounds.width, 1), span));
  };
  const key = (event: KeyboardEvent<SVGSVGElement>) => {
    const step = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : event.key === "Home" ? -points.length : event.key === "End" ? points.length : 0;
    if (!step) return;
    event.preventDefault();
    setKeyboard(true);
    setActive((current) => Math.min(points.length - 1, Math.max(0, (current ?? points.length - 1) + step)));
  };
  const readout = point ? `${tooltipTime(point.at)}: ${series.map((s) => `${s.label} ${format(point[s.key], unit)}`).join(", ")}` : "";

  return (
    <Card as="section" className="chart-card" aria-label={title}>
      <CardTitle icon={icon} level={3} actions={
        <ul className="chart-legend" aria-label="Series">
          {series.map((s) => <li key={s.key}><span className={`legend-line series-${s.tone}${s.dashed ? " dashed" : ""}`} aria-hidden="true" />{s.label}</li>)}
        </ul>
      }>{title}</CardTitle>
      <div className="chart" data-testid={`history-chart-${series[0].key}`}>
        <div className="chart-y" aria-hidden="true">
          <span>{domain[1]}{unit}</span><span>{(domain[0] + domain[1]) / 2}{unit}</span><span>{domain[0]}{unit}</span>
        </div>
        <div className="chart-plot">
          <svg viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`} preserveAspectRatio="none" role="img" aria-label={description} tabIndex={0}
            onPointerMove={move} onPointerDown={move} onPointerLeave={() => setActive(null)} onKeyDown={key}
            onFocus={() => { setKeyboard(true); setActive(points.length - 1); }} onBlur={() => setActive(null)}>
            {[0, 0.25, 0.5, 0.75, 1].map((r) => (
              <line key={r} className="chart-grid" x1={0} x2={CHART_WIDTH} y1={r * CHART_HEIGHT} y2={r * CHART_HEIGHT} vectorEffect="non-scaling-stroke" />
            ))}
            {summaries.flatMap((s) => seriesPaths(points, s.key, domain, span, history.bucket_seconds).map((d, i) => (
              <path key={`${s.key}-${i}`} d={d} className={`chart-line series-${s.tone}${s.dashed ? " dashed" : ""}`} data-history-line={s.key} vectorEffect="non-scaling-stroke" />
            )))}
            {point && <line className="chart-cursor" x1={ratio * CHART_WIDTH} x2={ratio * CHART_WIDTH} y1={0} y2={CHART_HEIGHT} vectorEffect="non-scaling-stroke" />}
          </svg>
          {point && (
            <div className={`chart-tooltip${ratio < 0.25 ? " start" : ratio > 0.75 ? " end" : ""}`} style={{ left: `${ratio * 100}%` }} aria-hidden="true" data-history-tooltip="">
              <p className="chart-tooltip-time">{tooltipTime(point.at)}</p>
              {series.map((s) => (
                <p key={s.key}><span><span className={`legend-line series-${s.tone}${s.dashed ? " dashed" : ""}`} />{s.label}</span><span>{format(point[s.key], unit)}</span></p>
              ))}
            </div>
          )}
          <p className="sr-only" aria-live="polite">{keyboard ? readout : ""}</p>
        </div>
        <span aria-hidden="true" />
        <div className="chart-x" aria-hidden="true">
          {axisTimes(span).map((at, i) => <span key={i}>{axisLabel(at, range)}</span>)}
        </div>
      </div>
      <table className="chart-summary">
        <caption className="sr-only">{title} summary for {rangeWords[range]}</caption>
        <thead><tr><th scope="col">Series</th><th scope="col">Now</th><th scope="col">Min</th><th scope="col">Avg</th><th scope="col">Max</th></tr></thead>
        <tbody>
          {summaries.map((s) => (
            <tr key={s.key}>
              <th scope="row"><span className={`legend-line series-${s.tone}${s.dashed ? " dashed" : ""}`} aria-hidden="true" />{s.label}</th>
              <td>{format(s.summary.current, unit)}</td><td>{format(s.summary.min, unit)}</td><td>{format(s.summary.average, unit)}</td><td>{format(s.summary.max, unit)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {summaries.every((s) => s.summary.current === null) && <p className="card-note"><Icon name="info" size={14} /> No {title.toLowerCase()} samples were recorded in this range.</p>}
    </Card>
  );
}
