/**
 * Friendly copy for a failed read. The raw server message, code and audit
 * reference are kept for support (shown behind a disclosure), never as the
 * headline. Pure and dependency-free so it is unit tested directly.
 */
export interface FriendlyError {
  title: string;
  detail: string;
  /** The server's audit reference, when it sent a well-formed one. */
  reference: string | null;
  /** The server's own message, kept for support. */
  serverMessage: string | null;
  code: string | null;
  retry: boolean;
}

interface Envelope { code: string; message: string; retry: boolean; audit_id: string }

function envelope(error: unknown): Envelope | null {
  const detail = (error as { detail?: unknown } | null)?.detail as Record<string, unknown> | undefined;
  if (!detail || typeof detail !== "object") return null;
  return typeof detail.code === "string" && typeof detail.message === "string" && typeof detail.audit_id === "string"
    ? { code: detail.code, message: detail.message, retry: detail.retry === true, audit_id: detail.audit_id }
    : null;
}

const copy: Record<string, (subject: string) => [string, string]> = {
  unavailable: (s) => [`${s} can't be read right now`, "The host didn't answer this time. It usually recovers by itself; try again in a moment."],
  overloaded: (s) => [`${s} is busy`, "The host is handling other requests. Wait a few seconds and try again."],
  rate_limited: (s) => [`${s} is busy`, "Too many requests in a short time. Wait a few seconds and try again."],
  forbidden: (s) => [`${s} isn't available to your account`, "Your role doesn't include this. Ask an administrator if you need it."],
  unauthenticated: () => ["Your session has ended", "Sign in again to continue."],
  not_found: (s) => [`${s} wasn't found`, "It may have been removed or renamed since the page loaded."],
  expired: (s) => [`${s} has expired`, "Load it again to see the current state."],
  conflict: (s) => [`${s} changed while you were looking`, "Refresh to see the current state."],
  corrupt_configuration: (s) => [`${s} can't be trusted right now`, "The host's protected configuration could not be verified. An administrator should check the host."],
  state_not_durable: (s) => [`${s} couldn't be recorded safely`, "Nothing was changed. Try again shortly."],
};

export function describeError(error: unknown, subject: string): FriendlyError {
  const value = envelope(error);
  if (!value) {
    return {
      title: `${subject} can't be read right now`,
      detail: "LimeOS couldn't reach the host. Check that it is switched on and connected, then try again.",
      reference: null, serverMessage: null, code: null, retry: true,
    };
  }
  const [title, detail] = (copy[value.code] ?? ((s: string) => [`${s} can't be read right now`, "Something went wrong on the host. Try again; if it keeps happening, quote the reference below."]))(subject);
  return {
    title, detail,
    reference: /^[A-Za-z0-9_-]{1,128}$/.test(value.audit_id) ? value.audit_id : null,
    serverMessage: value.message.slice(0, 512),
    code: value.code,
    retry: value.code !== "forbidden" && value.code !== "unauthenticated",
  };
}
