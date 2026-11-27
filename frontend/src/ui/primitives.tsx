/**
 * The UI primitives every page builds on. Styling lives in primitives.css and
 * uses only the tokens in tokens.css. Tones are semantic; status is never
 * carried by colour alone (each badge has a text label, each banner an icon
 * and a heading).
 */
import type { HTMLAttributes, ReactNode } from "react";
import { Icon, type IconName } from "./icons";
import { describeError } from "./error-copy";
import { formatAge, formatClock } from "./format";
import type { LiveState } from "../app/live-status";

export type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

const join = (...names: (string | false | null | undefined)[]) => names.filter(Boolean).join(" ");

export function Card({ className, as: Tag = "div", ...props }: HTMLAttributes<HTMLElement> & { as?: "div" | "section" | "article" }) {
  return <Tag className={join("card", className)} {...props} />;
}

/** A card heading row: icon, title (h2 by default) and optional trailing actions. */
export function CardTitle({ icon, children, actions, id, level = 2 }: {
  icon?: IconName; children: ReactNode; actions?: ReactNode; id?: string; level?: 2 | 3;
}) {
  const Heading = level === 2 ? "h2" : "h3";
  return (
    <div className="card-title">
      <Heading id={id}>{icon && <Icon name={icon} size={16} className="tone-accent" />}{children}</Heading>
      {actions && <div className="card-actions">{actions}</div>}
    </div>
  );
}

export function Badge({ tone = "neutral", className, ...props }: HTMLAttributes<HTMLSpanElement> & { tone?: Tone }) {
  return <span className={join("ui-badge", `tone-${tone}`, className)} {...props} />;
}

/** A badge with a status dot; the label is the meaning, the dot is a cue. */
export function StatusBadge({ label, tone = "neutral", className }: { label: string; tone?: Tone; className?: string }) {
  return <Badge tone={tone} className={className}><span className="dot" aria-hidden="true" />{label}</Badge>;
}

/** A 0–100 bar. Unknown values draw an empty, hatched track and say so. */
export function MetricBar({ value, tone = "accent", label, valueText }: {
  value: number | null | undefined; tone?: Tone; label: string; valueText?: string;
}) {
  const known = typeof value === "number" && Number.isFinite(value);
  const width = known ? Math.min(100, Math.max(0, value)) : 0;
  return (
    <div className={join("metric-bar", !known && "unknown")} role="progressbar" aria-label={label}
      aria-valuemin={0} aria-valuemax={100} aria-valuenow={known ? Math.round(width) : undefined}
      aria-valuetext={valueText ?? (known ? `${width.toFixed(1)}%` : "Unknown")}>
      <div className={`fill tone-bg-${tone}`} style={{ width: `${width}%` }} />
    </div>
  );
}

/** Page title row. The h1 receives focus after navigation (see Shell). */
export function PageHeader({ title, description, status, actions, eyebrow }: {
  title: string; description?: ReactNode; status?: ReactNode; actions?: ReactNode; eyebrow?: string;
}) {
  return (
    <header className="page-header">
      <div className="page-header-text">
        {eyebrow && <p className="page-eyebrow">{eyebrow}</p>}
        <div className="page-title-row">
          <h1 tabIndex={-1} data-page-title="">{title}</h1>
          {status}
        </div>
        {description && <div className="page-description">{description}</div>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </header>
  );
}

/**
 * "Updated 12:04:31 · live" plus a refresh button. `live` reflects the shared
 * observation stream; the label stays steady across the stream's normal
 * 25-second rotation (see app/live-status.ts).
 */
export function RefreshControl({ updatedAt, live, refreshing = false, onRefresh, now }: {
  updatedAt: number | null; live: LiveState; refreshing?: boolean; onRefresh?: () => void; now?: number;
}) {
  const label = { connecting: "Connecting", live: "Live", polling: "Updating every 30 s", reconnecting: "Reconnecting", offline: "Offline" }[live];
  const tone: Tone = live === "live" ? "success" : live === "polling" || live === "connecting" ? "info" : live === "reconnecting" ? "warning" : "danger";
  return (
    <div className="refresh-control">
      <StatusBadge label={label} tone={tone} />
      <span className="refresh-time" title={updatedAt ? new Date(updatedAt * 1000).toLocaleString() : undefined}>
        {updatedAt === null ? "Waiting for first update" : <>Updated <time dateTime={new Date(updatedAt * 1000).toISOString()}>{formatClock(updatedAt)}</time><span className="sr-only">, {formatAge(updatedAt, now)}</span></>}
      </span>
      {onRefresh && (
        <button type="button" className="quiet icon-button" onClick={onRefresh} disabled={refreshing} aria-busy={refreshing} aria-label="Refresh now" title="Refresh now">
          <Icon name="refresh" size={16} className={refreshing ? "spin" : undefined} />
        </button>
      )}
    </div>
  );
}

export type StateKind = "loading" | "empty" | "error" | "unavailable";

/**
 * One way to say "loading", "nothing here", "couldn't read" and "not set up".
 * Errors get friendly copy and a retry; the server reference stays available
 * for support behind a disclosure.
 */
export function StateMessage({ kind, title, children, onRetry, error, subject = "This information", compact = false }: {
  kind: StateKind; title?: string; children?: ReactNode; onRetry?: () => void; error?: unknown; subject?: string; compact?: boolean;
}) {
  const friendly = kind === "error" && error !== undefined ? describeError(error, subject) : null;
  const icon: IconName = kind === "loading" ? "clock" : kind === "empty" ? "info" : kind === "unavailable" ? "info" : "warning";
  const heading = title ?? friendly?.title ?? (kind === "loading" ? "Loading…" : kind === "empty" ? "Nothing to show yet" : `${subject} can't be read right now`);
  return (
    <div className={join("state-message", `state-${kind}`, compact && "compact")} role={kind === "error" ? "alert" : "status"} aria-busy={kind === "loading" || undefined}>
      <Icon name={icon} size={20} className={kind === "loading" ? "pulse" : undefined} />
      <div className="state-text">
        <p className="state-title">{heading}</p>
        {(children ?? friendly?.detail) && <div className="state-detail">{children ?? friendly?.detail}</div>}
        {friendly && (friendly.reference || friendly.serverMessage) && (
          <details className="state-support">
            <summary>Details for support</summary>
            <p>{friendly.serverMessage}{friendly.code && <> · <code>{friendly.code}</code></>}{friendly.reference && <> · reference <code>{friendly.reference}</code></>}</p>
          </details>
        )}
        {onRetry && (friendly?.retry ?? true) && <button type="button" className="quiet" onClick={onRetry}>Try again</button>}
      </div>
    </div>
  );
}

/** A labelled number with optional badges beneath, used for counts. */
export function Stat({ label, value, icon, children }: { label: string; value: ReactNode; icon?: IconName; children?: ReactNode }) {
  return (
    <div className="stat">
      <div className="stat-head"><span className="stat-label">{icon && <Icon name={icon} size={16} />}{label}</span><span className="stat-value">{value}</span></div>
      {children && <div className="stat-badges">{children}</div>}
    </div>
  );
}
