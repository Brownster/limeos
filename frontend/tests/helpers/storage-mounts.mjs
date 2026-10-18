import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import ts from "typescript";

const require = createRequire(import.meta.url);
const encode = source => `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
async function code(file) {
  return ts.transpileModule(await readFile(new URL(`../../src/${file}`, import.meta.url), "utf8"), {
    fileName: file,
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX },
  }).outputText.replaceAll('"react/jsx-runtime"', JSON.stringify(pathToFileURL(require.resolve("react/jsx-runtime")).href)).replaceAll('"react"', JSON.stringify(pathToFileURL(require.resolve("react")).href));
}
const api = encode(await code("api.ts"));
const client = encode((await code("storage/client.ts")).replaceAll('"../api"', JSON.stringify(api)));
const controller = encode((await code("storage/controller.ts")).replaceAll('"../api"', JSON.stringify(api)).replaceAll('"./client"', JSON.stringify(client)));
const view = encode((await code("storage/StorageMounts.tsx")).replaceAll('"./controller"', JSON.stringify(controller)));
export const { ApiError } = await import(api);
export const { createMountApi } = await import(client);
export const { StorageMountController } = await import(controller);
export const { StorageMounts } = await import(view);

const hash = char => char.repeat(64);
export const disk = { id: "data", role: "data", filesystem_uuid: "uuid-data", filesystem: "ext4", mountpoint: "/mnt/data", serial: "disk-data" };
const identity = inode => ({ major: 8, minor: 1, mount_id: 10, inode, mode: 16877, uid: 0, gid: 0 });
export function proposal() {
  return { plan: { id: hash("1"), version: 1, principal: "fixture-admin", grant_revision: 1,
    effect: { version: 1, operation: "storage.mount", action: hash("2"), device: structuredClone(disk),
      expected: { host_root_mount_id: 10, topology_digest: hash("a"), mounts_digest: hash("b"), fstab_digest: hash("c"), fstab_entries: [], devices: [{ device: { major: 8, minor: 16 }, filesystem_uuid: disk.filesystem_uuid, filesystem: "ext4", serial: disk.serial, boot_backing: false, in_use_as_swap: false, mounts: [] }] },
      target: { mountpoint: disk.mountpoint, parent: identity(2), existing: identity(9) }, physical_devices: [{ major: 8, minor: 16 }], consumer_digest: hash("d"), created_at: 100, expires_at: 400 } }, digest: hash("e") };
}
export const approval = () => ({ token: hash("f"), expires_at: 400 });
export const job = (state = "queued", p = proposal()) => ({ id: p.plan.effect.action, state, plan: structuredClone(p.plan) });
export const apiError = code => new ApiError({ code, message: "fixture-private-diagnostic", retry: false, audit_id: "fixture-audit" });
export function fakeApi(overrides = {}) {
  return { targets: async () => ({ targets: [structuredClone(disk)] }), jobs: async () => [], plan: async () => proposal(), approve: async () => approval(), queue: async p => job("queued", p), cancel: async () => {}, ...overrides };
}
