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
const viewURL = encode((await code("StackManagement.tsx")).replace('"./stack-inventory-controller"', JSON.stringify(controllerURL)));
export const { decodeStackInventory, readStackInventory, inventoryStale, StackReadError } = await import(clientURL);
export const { StackInventoryController } = await import(controllerURL);
export const { StackManagement, StackPanel } = await import(viewURL);
export const wire = JSON.parse(await readFile(new URL("../../../tests/fixtures/deployment/discovery/wire.json", import.meta.url), "utf8"));
