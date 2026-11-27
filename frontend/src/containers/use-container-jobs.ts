/**
 * The principal's container jobs, shared by the rows (an operation in flight
 * shows on its container) and "Recent container operations". Read on mount,
 * on `revision` and every 2 s while a job is queued, running or verifying,
 * exactly as the operations list always polled; nothing is read for viewers.
 * When a job settles, `settled` runs once so the page can re-read the overview
 * and the row shows the result without a manual refresh.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { NeedsAttentionJob, RestartJob } from "../../../contracts/generated/types";
import { api, ApiError } from "../api";
import { jobInFlight, settledJobs } from "./container-model";

export interface ContainerJobsState {
  jobs: RestartJob[];
  /** Administrators: every principal's stuck jobs, for the resolution banner. */
  listed: NeedsAttentionJob[];
  error: unknown;
  loaded: boolean;
  reload: () => void;
}

export function useContainerJobs({ enabled, admin, revision, expired, settled }: {
  enabled: boolean;
  admin: boolean;
  revision: number;
  expired: () => void;
  settled: () => void;
}): ContainerJobsState {
  const [jobs, setJobs] = useState<RestartJob[]>([]);
  const [listed, setListed] = useState<NeedsAttentionJob[]>([]);
  const [error, setError] = useState<unknown>(null);
  const [loaded, setLoaded] = useState(false);
  const [nonce, setNonce] = useState(0);
  const previous = useRef<RestartJob[]>([]);
  const onSettled = useRef(settled);
  onSettled.current = settled;
  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (!enabled) return;
    let active = true, inFlight = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function read() {
      if (inFlight) return;
      inFlight = true;
      try {
        const next = await api.restartJobs();
        if (!active) return;
        if (settledJobs(previous.current, next).length) onSettled.current();
        previous.current = next;
        setJobs(next);
        setLoaded(true);
        setError(null);
        // Read-only and best effort: the banner falls back to the own jobs.
        if (admin)
          api.needsAttention()
            .then((all) => active && setListed(all))
            .catch(() => active && setListed([]));
        if (next.some(jobInFlight)) timer = setTimeout(() => void read(), 2000);
      } catch (e) {
        if (!active) return;
        if (e instanceof ApiError && e.detail.code === "unauthenticated") expired();
        else setError(e);
        setLoaded(true);
      } finally {
        inFlight = false;
      }
    }
    void read();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [enabled, admin, revision, nonce, expired]);

  return { jobs, listed, error, loaded, reload };
}
