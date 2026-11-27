import { readFile } from "node:fs/promises";
import { loader } from "./modules.mjs";

// The real inventory client, controller, flow controller and views, with the
// HTTP client replaced by a test-controlled global and the dialog helpers
// and resolution panel stubbed. Relative imports are followed by modules.mjs.
const modules = loader({
  "api.ts": `export class ApiError extends Error { constructor(detail) { super(detail.message); this.detail = detail; } }
export const api = new Proxy({}, { get: (_, name) => (...args) => globalThis.__stackApi[name](...args) });`,
  "operation-ui.ts": `export const message = error => error instanceof Error ? error.message : "unavailable";
export const containFocus = () => {};`,
  "ResolutionPanel.tsx": "export const NeedsAttention = () => null; export const mergeAttention = (own) => own;",
});
export const { decodeStackInventory, readStackInventory, inventoryStale, StackReadError } = await modules.load("stack-inventory-client.ts");
export const { StackInventoryController } = await modules.load("stack-inventory-controller.ts");
export const { StackManagement, StackPanel } = await modules.load("StackManagement.tsx");
export const { StackActionController, stackTargets } = await modules.load("stack-action-controller.ts");
export const { stackAvailability, StackJobRow, StackJobStrip, StackReceiptView, isInterrupted, partlyDone } = await modules.load("StackActions.tsx");
export const { ApiError } = await modules.load("api.ts");
export const wire = JSON.parse(await readFile(new URL("../../../tests/fixtures/deployment/discovery/wire.json", import.meta.url), "utf8"));
