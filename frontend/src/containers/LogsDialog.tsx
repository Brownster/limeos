/**
 * A container's recent log lines. Reads only while open: once on opening, on
 * "Refresh logs", on a change of length, and every 5 s while the user has
 * switched auto-refresh on and the tab is visible. Closing the dialog or
 * hiding the tab stops it. The host caps a read at 200 lines and 24 KiB
 * (newest lines kept) and filters credentials and terminal controls; the
 * text is rendered as text, never as HTML.
 */
import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { ContainerLogs, Resource } from "../../../contracts/generated/types";
import { request } from "../api";
import { containFocus } from "../operation-ui";
import { restoreFocus } from "../ResolutionPanel";
import { StateMessage } from "../ui/primitives";
import { formatClock } from "../ui/format";
import { containerClient } from "./container-client";
import { CopyButton } from "./CopyButton";
import { LOG_REFRESH_MS, LOG_TAILS, logNote, type LogTail } from "./container-model";

export interface LogsTarget {
  resource: Resource;
  opener: HTMLElement | null;
  fallback?: () => HTMLElement | null;
  nonce: number;
}

const client = containerClient(request);

export function LogsDialog({ target, onClosed }: { target: LogsTarget | null; onClosed: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const output = useRef<HTMLPreElement>(null);
  const [tail, setTail] = useState<LogTail>(100);
  const [logs, setLogs] = useState<ContainerLogs | null>(null);
  const [readAt, setReadAt] = useState<number | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [auto, setAuto] = useState(false);
  const [wrap, setWrap] = useState(true);
  const [open, setOpen] = useState(false);
  const [hidden, setHidden] = useState(() => typeof document !== "undefined" && document.hidden);
  const current = useRef<LogsTarget | null>(null);
  const inFlight = useRef(false);
  const stick = useRef(true);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const read = useCallback(async (lines: number) => {
    const target = current.current;
    if (!target || inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    try {
      const value = await client.logs(target.resource.id, lines);
      if (!alive.current || current.current !== target) return;
      const box = output.current;
      stick.current = !box || box.scrollHeight - box.scrollTop - box.clientHeight < 24;
      setLogs(value);
      setReadAt(Date.now() / 1000);
      setError(null);
    } catch (e) {
      if (alive.current && current.current === target) setError(e);
    } finally {
      inFlight.current = false;
      if (alive.current) setPending(false);
    }
  }, []);

  // Open on a new target: reset and read once.
  useEffect(() => {
    if (!target) return;
    current.current = target;
    stick.current = true;
    setLogs(null);
    setError(null);
    setReadAt(null);
    setAuto(false);
    setOpen(true);
    if (!dialog.current?.open) dialog.current?.showModal();
    void read(tail);
    // `tail` is read at opening only; later changes go through changeTail.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, read]);

  // Keep the newest line in view, unless the user scrolled up to read.
  useEffect(() => {
    const box = output.current;
    if (box && stick.current) box.scrollTop = box.scrollHeight;
  }, [logs]);

  useEffect(() => {
    const visibility = () => setHidden(document.hidden);
    document.addEventListener("visibilitychange", visibility);
    return () => document.removeEventListener("visibilitychange", visibility);
  }, []);

  useEffect(() => {
    if (!open || !auto || hidden) return;
    const timer = window.setInterval(() => void read(tail), LOG_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [open, auto, hidden, tail, read]);

  function changeTail(next: LogTail) {
    setTail(next);
    stick.current = true;
    void read(next);
  }

  function closed() {
    const was = current.current;
    current.current = null;
    setOpen(false);
    setAuto(false);
    onClosed();
    if (was) restoreFocus(was.opener, was.fallback ?? (() => null));
  }

  const name = target?.resource.name ?? current.current?.resource.name ?? "";
  return (
    <dialog ref={dialog} className="operation-dialog log-dialog" aria-labelledby={titleId} onClose={closed} onKeyDown={containFocus}>
      <header className="page-heading">
        <h2 id={titleId}>Logs · {name}</h2>
        <button autoFocus className="quiet" onClick={() => dialog.current?.close()}>
          Close logs
        </button>
      </header>
      <div className="log-toolbar">
        <label className="log-tail">
          <span>Lines</span>
          <select aria-label="Lines to show" value={tail} onChange={(event) => changeTail(Number(event.target.value) as LogTail)}>
            {LOG_TAILS.map((value) => <option key={value} value={value}>Last {value}</option>)}
          </select>
        </label>
        <button className="quiet" disabled={pending} aria-busy={pending} onClick={() => void read(tail)}>
          Refresh logs
        </button>
        <button className="quiet" aria-pressed={auto} onClick={() => setAuto((on) => !on)}>
          Auto-refresh {auto ? "on" : "off"}
        </button>
        <button className="quiet" aria-pressed={wrap} onClick={() => setWrap((on) => !on)}>
          Wrap lines
        </button>
        <CopyButton value={logs?.text ?? ""} label="Copy log lines" target={() => output.current} />
      </div>
      <p className="log-meta">
        <span role="status">{logs ? logNote(tail, logs.text, logs.truncated) : pending ? "Reading logs…" : ""}</span>
        {readAt !== null && <span className="log-read"> Read at {formatClock(readAt)}{auto ? (hidden ? "; auto-refresh paused while this tab is hidden" : "; refreshing every 5 s") : ""}.</span>}
      </p>
      {logs?.truncated && (
        <p className="notice">
          <strong>Output reached the size limit.</strong> The host sends at most 24 KiB, keeping the newest lines; ask for fewer lines to see whole ones.
        </p>
      )}
      {error !== null && <StateMessage kind="error" compact error={error} subject="The logs" onRetry={() => void read(tail)} />}
      {logs && (
        <pre ref={output} className={wrap ? "container-logs" : "container-logs nowrap"} tabIndex={0} aria-label={`Log lines of ${name}`}>
          {logs.text || "No log output."}
        </pre>
      )}
      <p className="log-foot">Common credentials and terminal controls are filtered by the host.</p>
    </dialog>
  );
}
