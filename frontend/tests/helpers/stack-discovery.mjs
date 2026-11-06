import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import ts from "typescript";

const require = createRequire(import.meta.url);
const encode = source => `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
async function code(file) {
  return ts.transpileModule(await readFile(new URL(`../../src/${file}`, import.meta.url), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX },
  }).outputText
    .replace('"react/jsx-runtime"', JSON.stringify(pathToFileURL(require.resolve("react/jsx-runtime")).href))
    .replace('"react"', JSON.stringify(pathToFileURL(require.resolve("react")).href));
}
const clientURL = encode(await code("stack-inventory-client.ts"));
const controllerURL = encode((await code("stack-inventory-controller.ts")).replace('"./stack-inventory-client"', JSON.stringify(clientURL)));
// Whole-stack actions: the real flow controller and view, with the HTTP client
// replaced by a test-controlled global and the dialog helpers stubbed.
const apiURL = encode(`export class ApiError extends Error { constructor(detail) { super(detail.message); this.detail = detail; } }
export const api = new Proxy({}, { get: (_, name) => (...args) => globalThis.__stackApi[name](...args) });`);
const uiURL = encode(`export const message = error => error instanceof Error ? error.message : "unavailable";
export const containFocus = () => {};`);
const panelURL = encode("export const NeedsAttention = () => null; export const mergeAttention = (own) => own;");
const flowURL = encode(await code("stack-action-controller.ts"));
const actionsURL = encode((await code("StackActions.tsx"))
  .replace('"./api"', JSON.stringify(apiURL)).replace('"./operation-ui"', JSON.stringify(uiURL)).replace('"./ResolutionPanel"', JSON.stringify(panelURL))
  .replace('"./stack-action-controller"', JSON.stringify(flowURL)));
const viewURL = encode((await code("StackManagement.tsx")).replace('"./stack-inventory-controller"', JSON.stringify(controllerURL))
  .replace('"./StackActions"', JSON.stringify(actionsURL)));
export const { decodeStackInventory, readStackInventory, inventoryStale, StackReadError } = await import(clientURL);
export const { StackInventoryController } = await import(controllerURL);
export const { StackManagement, StackPanel } = await import(viewURL);
export const { StackActionController, stackTargets } = await import(flowURL);
export const { stackAvailability, StackJobRow, StackReceiptView, isInterrupted, partlyDone } = await import(actionsURL);
export const { ApiError } = await import(apiURL);
export const wire = JSON.parse(await readFile(new URL("../../../tests/fixtures/deployment/discovery/wire.json", import.meta.url), "utf8"));
