/**
 * The one shared source of host observations for every page: a single
 * EventSource on /api/v1/observations/stream with a 30 s GET /overview
 * fallback. Mounted once by the shell; pages read it with useOverview() and
 * never open their own stream or poll (a session may hold only two streams).
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Overview } from "../../../contracts/generated/types";
import { api, ApiError } from "../api";
import { liveState, pollNeeded, type LiveState } from "./live-status";

export interface OverviewData {
  value: Overview | null;
  /** The last failed read (an ApiError or other), cleared by the next success. */
  error: unknown;
  /** A snapshot that could not be parsed. */
  invalid: boolean;
  live: LiveState;
  /** When the shown observation was last confirmed (epoch seconds). */
  updatedAt: number | null;
  refreshing: boolean;
  /** Re-read now. Also bumps refreshKey so pages can re-read their own data. */
  refresh: () => void;
  /** Increments on every manual refresh. */
  refreshKey: number;
  /** A clock that ticks every 5 s, for ages and freshness (no network). */
  now: number;
}

const Context = createContext<OverviewData | null>(null);

export function useOverview(): OverviewData {
  const value = useContext(Context);
  if (!value) throw new Error("useOverview needs OverviewProvider");
  return value;
}

const seconds = () => Date.now() / 1000;

export function OverviewProvider({ expired, children }: { expired: () => void; children: ReactNode }) {
  const [value, setValue] = useState<Overview | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [invalid, setInvalid] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [now, setNow] = useState(seconds);
  const [marks, setMarks] = useState<{ snapshotAt: number | null; succeededAt: number | null; failedAt: number | null }>({ snapshotAt: null, succeededAt: null, failedAt: null });
  const expiry = useRef(expired);
  expiry.current = expired;
  const refreshRef = useRef<(manual: boolean) => void>(() => {});

  useEffect(() => {
    let active = true;
    let stream: EventSource | null = null;
    let snapshotAt: number | null = null;
    let connectedAt = 0;
    const mark = (patch: Partial<typeof marks>) => setMarks((m) => ({ ...m, ...patch }));
    const connect = () => {
      stream?.close();
      connectedAt = seconds();
      stream = new EventSource("/api/v1/observations/stream");
      stream.addEventListener("snapshot", (event) => {
        if (!active) return;
        try {
          const next = JSON.parse((event as MessageEvent<string>).data) as Overview;
          snapshotAt = seconds();
          setValue(next);
          setError(null);
          setInvalid(false);
          mark({ snapshotAt, succeededAt: snapshotAt });
        } catch {
          setInvalid(true);
        }
      });
      stream.onerror = () => {
        // The stream ends every 25 s and the browser reconnects by itself;
        // only a stream the browser gave up on needs the GET fallback now.
        if (active && stream?.readyState === EventSource.CLOSED) read(false);
      };
    };
    const read = (manual: boolean) => {
      if (manual) setRefreshing(true);
      void api
        .overview()
        .then((next) => {
          if (!active) return;
          setValue(next);
          setError(null);
          setInvalid(false);
          mark({ succeededAt: seconds() });
          // The host answers again: retry a stream the browser had given up
          // on, at most once per poll interval.
          if (!document.hidden && (!stream || stream.readyState === EventSource.CLOSED) && seconds() - connectedAt >= 30) connect();
        })
        .catch((e: unknown) => {
          if (!active) return;
          if (e instanceof ApiError && e.detail.code === "unauthenticated") {
            expiry.current();
            return;
          }
          setError(e);
          mark({ failedAt: seconds() });
        })
        .finally(() => {
          if (active && manual) setRefreshing(false);
        });
    };
    refreshRef.current = read;
    read(false);
    if (!document.hidden) connect();
    const visibility = () => {
      if (document.hidden) {
        stream?.close();
        stream = null;
      } else {
        read(false);
        connect();
      }
    };
    document.addEventListener("visibilitychange", visibility);
    // Freshness and session checks when a proxy cannot carry SSE; skipped
    // while the stream is delivering, which already proves both.
    const poll = window.setInterval(() => {
      if (!document.hidden && pollNeeded(snapshotAt, seconds())) read(false);
    }, 30_000);
    const clock = window.setInterval(() => setNow(seconds()), 5_000);
    return () => {
      active = false;
      stream?.close();
      window.clearInterval(poll);
      window.clearInterval(clock);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);

  const refresh = useCallback(() => {
    setRefreshKey((k) => k + 1);
    refreshRef.current(true);
  }, []);

  const data = useMemo<OverviewData>(() => {
    const live = liveState({ ...marks, now });
    return {
      value, error, invalid, live, refreshing, refresh, refreshKey, now,
      updatedAt: marks.succeededAt,
    };
  }, [value, error, invalid, marks, now, refreshing, refresh, refreshKey]);

  return <Context.Provider value={data}>{children}</Context.Provider>;
}
