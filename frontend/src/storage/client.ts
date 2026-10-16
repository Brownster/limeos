import type {
  PlanApproval,
  PlannedStorageMount,
  StorageMountJob,
  StorageMountCatalog,
} from "../../../contracts/generated/types";
import { request } from "../api";

const root = "/storage/mounts";
const post = (csrf: string, body?: unknown): RequestInit => ({
  method: "POST",
  headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
  ...(body === undefined ? {} : { body: JSON.stringify(body) }),
});
export const mountApi = {
  targets: () => request<StorageMountCatalog>(`${root}/targets`),
  plan: (target_id: string, csrf: string) =>
    request<PlannedStorageMount>(`${root}/plans`, post(csrf, { target_id })),
  approve: (proposal: PlannedStorageMount, csrf: string) =>
    request<PlanApproval>(`${root}/plans/${proposal.plan.id}/approval`, post(csrf, { digest: proposal.digest })),
  queue: (proposal: PlannedStorageMount, approval: string, idempotency_key: string, csrf: string) =>
    request<StorageMountJob>(`${root}/jobs`, post(csrf, { proposal, approval, idempotency_key })),
  jobs: () => request<StorageMountJob[]>(`${root}/jobs`),
  cancel: (id: string, csrf: string) => request<void>(`${root}/jobs/${id}/cancel`, post(csrf)),
};
