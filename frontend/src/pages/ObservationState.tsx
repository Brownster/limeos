/**
 * What a page shows while the shared overview is loading or failed, and the
 * header control every observation-backed page uses.
 */
import { useOverview } from "../app/overview-data";
import { RefreshControl, StateMessage } from "../ui/primitives";

export function OverviewRefresh() {
  const { updatedAt, live, refreshing, refresh, now } = useOverview();
  return <RefreshControl updatedAt={updatedAt} live={live} refreshing={refreshing} onRefresh={refresh} now={now} />;
}

/** Null when an observation is available; otherwise loading or friendly error. */
export function OverviewPending() {
  const { value, error, invalid, refresh } = useOverview();
  if (value) {
    // Keep showing the last good observation; say that it is no longer current.
    if (error) return <StateMessage kind="error" compact error={error} subject="The latest observation" onRetry={refresh}>
      Showing the last observation received. It may be out of date.
    </StateMessage>;
    if (invalid) return <StateMessage kind="error" compact title="The host sent an observation LimeOS couldn't read">
      Showing the last good observation. The next update usually fixes this.
    </StateMessage>;
    return null;
  }
  if (error) return <StateMessage kind="error" error={error} subject="Host observations" onRetry={refresh} />;
  return <StateMessage kind="loading" title="Loading host observations…">Reading the latest from your host.</StateMessage>;
}
