import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { MediaActivityView } from "./MediaActivityView";
import { MediaActivityController } from "./media-activity-controller";

/** Mount inside the shared authenticated session boundary; remove on logout. */
export function MediaActivityPanel({ principal, grantRevision, expired }: { principal: string | null; grantRevision: number; expired: () => void }) {
  if (!principal || !Number.isSafeInteger(grantRevision) || grantRevision < 0) return null;
  return <AuthenticatedActivity principal={principal} grantRevision={grantRevision} expired={expired} />;
}
function AuthenticatedActivity({ principal, grantRevision, expired }: { principal: string; grantRevision: number; expired: () => void }) {
  const expiry = useRef(expired); expiry.current = expired;
  const controller = useMemo(() => new MediaActivityController(() => expiry.current()), [principal, grantRevision]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => {
    void controller.start(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  return <MediaActivityView value={state.value} now={Math.floor(Date.now() / 1000)} loading={state.loading} stale={state.stale} error={state.error} refresh={state.denied ? undefined : () => { void controller.refresh(); }} />;
}
