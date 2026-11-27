/**
 * GET /api/v1/storage/inventory: serial numbers, filesystem UUIDs, mounts and
 * /etc/fstab entries, read on demand through the storage reader. The server
 * allows it to administrators holding storage_manage only; each call is one
 * reader exchange, so the Storage page reads it once when it opens and again
 * only on a manual refresh. The response is decoded strictly, mirroring
 * StorageInventory::validate on the server.
 */
import type { StorageInventory, StorageInventoryView } from "../../../contracts/generated/types";

export class StorageInventoryError extends Error {
  readonly status: number;
  readonly code: string;
  // Plain fields (no parameter properties), so node tests load this file as is.
  constructor(message: string, status = 0, code = "unavailable") { super(message); this.status = status; this.code = code; }
}
const invalid = (): never => { throw new StorageInventoryError("The host returned a storage inventory LimeOS couldn't read.", 0, "invalid"); };
const record = (v: unknown): Record<string, unknown> => v !== null && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : invalid();
const list = (v: unknown, max: number): unknown[] => Array.isArray(v) && v.length <= max ? v : invalid();
const opaque = (v: unknown): string => typeof v === "string" && /^[0-9a-f]{64}$/.test(v) ? v : invalid();
const word = (v: unknown, max: number, extra: string): string => {
  if (typeof v !== "string" || v.length === 0 || v.length > max) return invalid();
  for (const c of v) if (!/[A-Za-z0-9]/.test(c) && !extra.includes(c)) invalid();
  return v as string;
};
const bool = (v: unknown): boolean => typeof v === "boolean" ? v : invalid();
const uint = (v: unknown, min: number, max: number): number => typeof v === "number" && Number.isSafeInteger(v) && v >= min && v <= max ? v : invalid();
/** "/" or an absolute path without empty, "." or ".." segments, NUL or control characters. */
function path(v: unknown): string {
  if (typeof v !== "string" || v.length === 0 || new TextEncoder().encode(v).length > 512 || /\p{Cc}/u.test(v)) return invalid();
  const p = v as string;
  if (p === "/") return p;
  if (!p.startsWith("/") || p.endsWith("/")) return invalid();
  for (const part of p.slice(1).split("/")) if (part === "" || part === "." || part === "..") invalid();
  return p;
}
function device(v: unknown): { major: number; minor: number } {
  const d = record(v);
  return { major: uint(d.major, 0, 0xffffffff), minor: uint(d.minor, 0, 0xffffffff) };
}

export function decodeStorageInventory(value: unknown): StorageInventoryView {
  const view = record(value);
  opaque(view.digest);
  const inv = record(view.inventory);
  uint(inv.host_root_mount_id, 1, 2 ** 53 - 1);
  opaque(inv.topology_digest); opaque(inv.mounts_digest); opaque(inv.fstab_digest);
  const numbers = new Set<string>();
  for (const item of list(inv.devices, 128)) {
    const d = record(item);
    const n = device(d.device);
    if (n.major === 0 || numbers.has(`${n.major}:${n.minor}`)) invalid();
    numbers.add(`${n.major}:${n.minor}`);
    word(d.filesystem_uuid, 128, "._-");
    word(d.filesystem, 64, "_-");
    if (d.serial !== null) {
      if (typeof d.serial !== "string" || d.serial.length === 0 || d.serial.length > 128 || d.serial.trim() !== d.serial || /\p{Cc}/u.test(d.serial)) invalid();
    }
    bool(d.boot_backing);
    if (d.in_use_as_swap !== undefined) bool(d.in_use_as_swap);
    for (const m of list(d.mounts, 32)) {
      const mount = record(m);
      path(mount.mountpoint); path(mount.filesystem_root);
      uint(mount.mount_id, 1, 2 ** 53 - 1);
      word(mount.filesystem, 64, "._-");
      bool(mount.writable);
    }
  }
  for (const item of list(inv.fstab_entries, 128)) {
    const e = record(item);
    path(e.mountpoint);
    if (e.filesystem_uuid !== null) word(e.filesystem_uuid, 128, "._-");
    if (e.device !== null) device(e.device);
    bool(e.managed);
  }
  return value as StorageInventoryView;
}

const LIMIT = 64 * 1024;
async function body(response: Response, signal: AbortSignal): Promise<unknown> {
  const reader = response.body?.getReader() ?? invalid();
  const buffer = new Uint8Array(LIMIT); let size = 0;
  const cancel = () => { void reader.cancel().catch(() => {}); };
  signal.addEventListener("abort", cancel, { once: true }); if (signal.aborted) cancel();
  try {
    for (;;) {
      if (signal.aborted) invalid();
      const chunk = await reader.read(); if (signal.aborted) invalid(); if (chunk.done) break;
      if (chunk.value.length > buffer.length - size) { cancel(); invalid(); }
      buffer.set(chunk.value, size); size += chunk.value.length;
    }
  } finally { signal.removeEventListener("abort", cancel); reader.releaseLock(); }
  try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, size))) as unknown; }
  catch { return invalid(); }
}

/** One GET. 401 and 403 are told apart so the page can hide the details quietly. */
export async function readStorageInventory(signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<StorageInventory> {
  const controller = new AbortController(); const abort = () => controller.abort();
  signal.addEventListener("abort", abort, { once: true }); if (signal.aborted) abort();
  const timer = setTimeout(abort, 12_000);
  try {
    if (controller.signal.aborted) invalid();
    const response = await fetcher("/api/v1/storage/inventory", { method: "GET", credentials: "same-origin", cache: "no-store", redirect: "error", headers: { Accept: "application/json" }, signal: controller.signal });
    if (!response.ok) {
      void response.body?.cancel().catch(() => {});
      const code = response.status === 401 ? "unauthenticated" : response.status === 403 ? "forbidden" : "unavailable";
      throw new StorageInventoryError("The storage inventory is unavailable.", response.status, code);
    }
    return decodeStorageInventory(await body(response, controller.signal)).inventory;
  } finally { clearTimeout(timer); signal.removeEventListener("abort", abort); }
}
