import type { ResolveJobInput } from "../../contracts/generated/types";
import {
  isJobId,
  type RawResponse,
  type ResolutionClient,
} from "./resolution-controller";

/**
 * Plain fetch: the resolution answers carry meaning in 202 and 409 bodies that
 * the shared request helper would flatten into errors.
 */
async function call(path: string, init: RequestInit, ms: number): Promise<RawResponse> {
  const response = await fetch(`/api/v1${path}`, {
    ...init,
    credentials: "same-origin",
    signal: AbortSignal.timeout(ms),
  });
  const body: unknown = await response.json().catch(() => null);
  return { status: response.status, body };
}
function path(job: string) {
  if (!isJobId(job)) throw new Error("Invalid job identifier");
  return `/jobs/${job}/resolution`;
}
export const resolutionClient: ResolutionClient = {
  read: (job) => call(path(job), {}, 12_000),
  submit: (job, input: ResolveJobInput, csrf) =>
    call(
      path(job),
      {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
        body: JSON.stringify(input),
      },
      30_000,
    ),
  session: async () => {
    try {
      const { status } = await call("/auth/session", {}, 12_000);
      return status === 200 ? "valid" : status === 401 ? "expired" : "unknown";
    } catch {
      return "unknown";
    }
  },
};
