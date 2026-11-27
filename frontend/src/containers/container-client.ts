/**
 * Reads the Containers page makes besides the shared overview: a container's
 * log tail and the storage mounts every container declares. Both responses
 * are decoded strictly (unknown fields, wrong types or values beyond the
 * host's limits are refused), as the other feature clients do.
 */
import type {
  ContainerLogs,
  ContainerStorageConsumer,
  ContainerStorageInventoryView,
  ContainerStorageMount,
  ContainerStorageSource,
} from "../../../contracts/generated/types";

export class DecodeError extends Error {
  constructor(what: string) {
    super(`The host sent ${what} LimeOS couldn't read.`);
  }
}

type Fetch = <T>(path: string) => Promise<T>;

const isObject = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
function exact(value: unknown, keys: string[], what: string): Record<string, unknown> {
  if (!isObject(value)) throw new DecodeError(what);
  const actual = Object.keys(value);
  if (actual.length !== keys.length || !actual.every((key) => keys.includes(key))) throw new DecodeError(what);
  return value;
}
const string = (value: unknown, max: number, what: string): string => {
  if (typeof value !== "string" || value.length > max) throw new DecodeError(what);
  return value;
};
const boolean = (value: unknown, what: string): boolean => {
  if (typeof value !== "boolean") throw new DecodeError(what);
  return value;
};
const integer = (value: unknown, what: string): number => {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new DecodeError(what);
  return value;
};

const CONTAINER = /^container:[0-9a-f]{64}$/;
const IMAGE_ID = /^sha256:[0-9a-f]{64}$/;
const path = (value: unknown, what: string) => {
  const text = string(value, 512, what);
  if (!text.startsWith("/") || /[\u0000-\u001f\u007f]/.test(text)) throw new DecodeError(what);
  return text;
};
const name = (value: unknown, max: number, what: string) => {
  const text = string(value, max, what);
  if (!/^[A-Za-z0-9_.-]+$/.test(text)) throw new DecodeError(what);
  return text;
};

/** The log tail as the host answers it; `resource` must be the container asked for. */
export function decodeLogs(value: unknown, resource: string, maxBytes = 24 * 1024): ContainerLogs {
  const what = "a log answer";
  const v = exact(value, ["resource", "text", "truncated"], what);
  if (v.resource !== resource) throw new DecodeError(what);
  // The host counts UTF-8 bytes; a JavaScript string is never longer in code units.
  return { resource, text: string(v.text, maxBytes, what), truncated: boolean(v.truncated, what) };
}

function decodeSource(value: unknown): ContainerStorageSource {
  const what = "a storage mount";
  if (isObject(value) && value.kind === "bind") {
    const v = exact(value, ["kind", "path"], what);
    return { kind: "bind", path: path(v.path, what) };
  }
  const v = exact(value, ["kind", "path", "name", "driver"], what);
  if (v.kind !== "volume") throw new DecodeError(what);
  return { kind: "volume", path: path(v.path, what), name: name(v.name, 255, what), driver: name(v.driver, 64, what) };
}

function decodeMount(value: unknown): ContainerStorageMount {
  const what = "a storage mount";
  const v = exact(value, ["source", "destination", "writable"], what);
  return { source: decodeSource(v.source), destination: path(v.destination, what), writable: boolean(v.writable, what) };
}

function decodeConsumer(value: unknown): ContainerStorageConsumer {
  const what = "a container's storage";
  const v = exact(value, ["container", "mounts"], what);
  const c = exact(v.container, ["resource", "image", "started_at", "running"], what);
  const resource = string(c.resource, 80, what);
  const image = string(c.image, 80, what);
  if (!CONTAINER.test(resource) || !IMAGE_ID.test(image)) throw new DecodeError(what);
  if (!Array.isArray(v.mounts) || v.mounts.length > 64) throw new DecodeError(what);
  return {
    container: { resource, image, started_at: string(c.started_at, 64, what), running: boolean(c.running, what) },
    mounts: v.mounts.map(decodeMount),
  };
}

/** `/storage/container-dependencies`: at most 64 containers, 64 mounts each, 256 in all. */
export function decodeContainerStorage(value: unknown): ContainerStorageInventoryView {
  const what = "a storage inventory";
  const v = exact(value, ["inventory", "digest"], what);
  const inv = exact(v.inventory, ["version", "engine_id", "observed_at", "containers"], what);
  if (inv.version !== 1 || !Array.isArray(inv.containers) || inv.containers.length > 64) throw new DecodeError(what);
  const containers = inv.containers.map(decodeConsumer);
  if (containers.reduce((sum, c) => sum + c.mounts.length, 0) > 256) throw new DecodeError(what);
  if (new Set(containers.map((c) => c.container.resource)).size !== containers.length) throw new DecodeError(what);
  return {
    inventory: {
      version: 1,
      engine_id: string(inv.engine_id, 128, what),
      observed_at: integer(inv.observed_at, what),
      containers,
    },
    digest: string(v.digest, 128, what),
  };
}

export function containerClient(fetch: Fetch) {
  return {
    logs: async (resource: string, tail: number) =>
      decodeLogs(await fetch<unknown>(`/containers/${encodeURIComponent(resource.slice(10))}/logs?tail=${tail}`), resource),
    storage: async () => decodeContainerStorage(await fetch<unknown>("/storage/container-dependencies")),
  };
}
