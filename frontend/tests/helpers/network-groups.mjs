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
const clientURL = encode(await code("network-inventory-client.ts"));
const controllerURL = encode((await code("network-inventory-controller.ts")).replaceAll('"./network-inventory-client"', JSON.stringify(clientURL)));
const viewURL = encode((await code("NetworkGroups.tsx")).replaceAll('"./network-inventory-client"', JSON.stringify(clientURL)).replaceAll('"./network-inventory-controller"', JSON.stringify(controllerURL)));
export const { decodeNetworkInventory, readNetworkInventory, networkInventoryExpired, NetworkReadError } = await import(clientURL);
export const { NetworkInventoryController } = await import(controllerURL);
export const { NetworkGroups, NetworkGroupsPanel } = await import(viewURL);
export const wire = JSON.parse(await readFile(new URL("../../../tests/fixtures/deployment/networks/wire.json", import.meta.url), "utf8"));
