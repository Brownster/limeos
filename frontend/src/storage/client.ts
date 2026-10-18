import type {
  PlanApproval,
  PlannedStorageMount,
  StorageMountJob,
  StorageMountCatalog,
} from "../../../contracts/generated/types";
import { ApiError } from "../api";

const root = "/storage/mounts";
const post = (csrf: string, body?: unknown): RequestInit => ({
  method: "POST",
  headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
  ...(body === undefined ? {} : { body: JSON.stringify(body) }),
});
const unavailable = () => new ApiError({ code: "unavailable", message: "Mount operations are unavailable. Check recent jobs before retrying.", retry: true, audit_id: "mount-transport" });
async function body(response: Response, limit: number, signal: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader(); if (!reader) throw unavailable();
  const buffer = new Uint8Array(limit); let size = 0;
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener("abort", cancel, { once: true }); if (signal.aborted) cancel();
  try {
    for (;;) {
      if (signal.aborted) throw unavailable();
      const chunk = await reader.read(); if (signal.aborted) throw unavailable(); if (chunk.done) break;
      if (chunk.value.length > buffer.length - size) { cancel(); throw unavailable(); }
      buffer.set(chunk.value, size); size += chunk.value.length;
    }
  } finally { signal.removeEventListener("abort", cancel); reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, size))) as unknown; }
  catch { throw unavailable(); }
}
/** Storage-owned transport: the shared request helper cannot compose cancellation. */
export function createMountApi(fetcher: typeof fetch = fetch) {
  async function request<T>(path: string, init: RequestInit = {}, signal?: AbortSignal, limit = 32768): Promise<T> {
    const controller = new AbortController(); const abort = () => controller.abort();
    signal?.addEventListener("abort", abort, { once: true }); if (signal?.aborted) abort();
    const timer = setTimeout(abort, 12000);
    try {
      if (controller.signal.aborted || typeof init.body === "string" && new TextEncoder().encode(init.body).length > 32768) throw unavailable();
      const response = await fetcher(`/api/v1${path}`, { ...init, credentials: "same-origin", cache: "no-store", redirect: "error", signal: controller.signal });
      if (controller.signal.aborted) { void response.body?.cancel().catch(() => {}); throw unavailable(); }
      if (response.status === 401 || response.status === 403) {
        void response.body?.cancel().catch(() => {});
        throw new ApiError({ code: response.status === 401 ? "unauthenticated" : "forbidden", message: response.status === 401 ? "Your session has expired." : "You no longer have access to mount operations.", retry: false, audit_id: `http-${response.status}` });
      }
      if (!response.ok) {
        const detail = await body(response, 4096, controller.signal).catch(() => null) as Record<string, unknown> | null;
        if (detail && typeof detail.code === "string" && detail.code.length <= 64 && typeof detail.message === "string" && detail.message.length > 0 && detail.message.length <= 512 && typeof detail.retry === "boolean" && typeof detail.audit_id === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(detail.audit_id)) throw new ApiError(detail as unknown as ConstructorParameters<typeof ApiError>[0]);
        throw unavailable();
      }
      return response.status === 204 ? undefined as T : await body(response, limit, controller.signal) as T;
    } finally { clearTimeout(timer); signal?.removeEventListener("abort", abort); }
  }
  return {
    targets: (signal?: AbortSignal) => request<StorageMountCatalog>(`${root}/targets`, {}, signal),
    plan: (target_id: string, csrf: string, signal?: AbortSignal) => request<PlannedStorageMount>(`${root}/plans`, post(csrf, { target_id }), signal),
    approve: (proposal: PlannedStorageMount, csrf: string, signal?: AbortSignal) => request<PlanApproval>(`${root}/plans/${encodeURIComponent(proposal.plan.id)}/approval`, post(csrf, { digest: proposal.digest }), signal, 4096),
    queue: (proposal: PlannedStorageMount, approval: string, idempotency_key: string, csrf: string, signal?: AbortSignal) => request<StorageMountJob>(`${root}/jobs`, post(csrf, { proposal, approval, idempotency_key }), signal),
    jobs: (signal?: AbortSignal) => request<StorageMountJob[]>(`${root}/jobs`, {}, signal, 1024 * 1024),
    cancel: (id: string, csrf: string, signal?: AbortSignal) => request<void>(`${root}/jobs/${encodeURIComponent(id)}/cancel`, post(csrf), signal),
  };
}
export const mountApi = createMountApi();
export type MountApi = ReturnType<typeof createMountApi>;
