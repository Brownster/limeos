/**
 * How current the shared observations are, for the "Live / Reconnecting"
 * badge. Pure and unit tested.
 *
 * The stream sends a snapshot at least every 5 s and ends every 25 s by
 * design; the browser reconnects a few seconds later. A failed or ended
 * connection is therefore not news: the badge only changes when no snapshot
 * has arrived for longer than STREAM_GRACE_SECONDS, so the normal rotation
 * never shows "Reconnecting".
 */
export type LiveState = "connecting" | "live" | "polling" | "reconnecting" | "offline";

export const STREAM_GRACE_SECONDS = 20;
/** The fallback poll runs every 30 s; allow one missed tick. */
export const POLL_GRACE_SECONDS = 75;

export interface LiveInputs {
  /** When the stream last delivered a snapshot (epoch seconds). */
  snapshotAt: number | null;
  /** When any overview read (stream or GET) last succeeded. */
  succeededAt: number | null;
  /** When an overview GET last failed. */
  failedAt: number | null;
  now: number;
}

export function liveState({ snapshotAt, succeededAt, failedAt, now }: LiveInputs): LiveState {
  if (snapshotAt !== null && now - snapshotAt <= STREAM_GRACE_SECONDS) return "live";
  if (failedAt !== null && (succeededAt === null || failedAt >= succeededAt)) return "offline";
  if (succeededAt !== null && now - succeededAt <= POLL_GRACE_SECONDS) return "polling";
  if (succeededAt === null) return "connecting";
  return "reconnecting";
}

/** Whether the 30 s fallback poll is redundant because the stream is current. */
export function pollNeeded(snapshotAt: number | null, now: number): boolean {
  return snapshotAt === null || now - snapshotAt > 30;
}
