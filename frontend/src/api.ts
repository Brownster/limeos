import type {
  ErrorEnvelope,
  Login,
  SessionView,
  Overview,
  ResourcePage,
  ResourceKind,
  MetricHistory,
  HistoryRange,
  PlannedRestart,
  PlanApproval,
  RestartJob,
  JobProgress,
} from "../../contracts/generated/types";
export class ApiError extends Error {
  readonly detail: ErrorEnvelope;
  constructor(detail: ErrorEnvelope) {
    super(detail.message);
    this.detail = detail;
  }
}
function isErrorEnvelope(value: unknown): value is ErrorEnvelope {
  if (!value || typeof value !== "object") return false;
  const error = value as Record<string, unknown>;
  return (
    typeof error.code === "string" &&
    error.code.length <= 64 &&
    typeof error.message === "string" &&
    error.message.length > 0 &&
    error.message.length <= 512 &&
    typeof error.retry === "boolean" &&
    typeof error.audit_id === "string" &&
    error.audit_id.length <= 128
  );
}
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/v1${path}`, {
    ...init,
    credentials: "same-origin",
    signal: AbortSignal.timeout(12_000),
  });
  if (!response.ok) {
    const error: unknown = await response.json().catch(() => null);
    if (isErrorEnvelope(error)) throw new ApiError(error);
    throw new ApiError({
      code: "unavailable",
      message: "The host could not complete the request. Try again.",
      retry: true,
      audit_id: `http-${response.status}`,
    });
  }
  return response.status === 204
    ? (undefined as T)
    : ((await response.json()) as T);
}
export const api = {
  planRestart: (resource: string, csrf: string) =>
    request<PlannedRestart>("/container/restart/plans", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify({ resource }),
    }),
  approveRestart: (proposal: PlannedRestart, csrf: string) =>
    request<PlanApproval>(
      `/container/restart/plans/${proposal.plan.id}/approval`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
        body: JSON.stringify({ digest: proposal.digest }),
      },
    ),
  queueRestart: (
    proposal: PlannedRestart,
    approval: string,
    key: string,
    csrf: string,
  ) =>
    request<RestartJob>("/container/restart/jobs", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify({ proposal, approval, idempotency_key: key }),
    }),
  restartJobs: () => request<RestartJob[]>("/container/restart/jobs"),
  restartProgress: (id: string, after = 0) =>
    request<JobProgress>(`/container/restart/jobs/${id}?after=${after}`),
  cancelRestart: (id: string, csrf: string, plan = false) =>
    request<void>(
      `/container/restart/${plan ? "plans" : "jobs"}/${id}/cancel`,
      {
        method: "POST",
        headers: { "X-CSRF-Token": csrf },
      },
    ),
  overview: () => request<Overview>("/overview"),
  resources: (kind?: ResourceKind, offset = 0, revision?: number) =>
    request<ResourcePage>(
      `/resources?limit=20&offset=${offset}${kind ? `&kind=${kind}` : ""}${revision === undefined ? "" : `&revision=${revision}`}`,
    ),
  history: (range: HistoryRange) =>
    request<MetricHistory>(`/system/history?range=${range}`),
  session: () => request<SessionView>("/auth/session"),
  login: (input: Login) =>
    request<SessionView>("/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(input),
    }),
  logout: (csrf: string) =>
    request<void>("/auth/logout", {
      method: "POST",
      headers: { "X-CSRF-Token": csrf },
    }),
};
