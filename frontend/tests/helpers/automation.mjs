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
const apiURL = encode(await code("api.ts"));
const clientURL = encode((await code("automation-client.ts")).replace('"./api"', JSON.stringify(apiURL)));
const controllerURL = encode((await code("automation-controller.ts")).replace('"./api"', JSON.stringify(apiURL)).replace('"./automation-client"', JSON.stringify(clientURL)));
const notificationsURL = encode((await code("NotificationsView.tsx")).replace('"./automation-client"', JSON.stringify(clientURL)));
const viewURL = encode((await code("AutomationView.tsx")).replace('"./automation-client"', JSON.stringify(clientURL)).replace('"./automation-controller"', JSON.stringify(controllerURL)).replace('"./NotificationsView"', JSON.stringify(notificationsURL)));
export const { ApiError } = await import(apiURL);
export const { automationClient, deliveryLabel } = await import(clientURL);
export const { AutomationController } = await import(controllerURL);
export const { NotificationsView } = await import(notificationsURL);
export const { AutomationView, AutomationPanel, AutomationReadStatus } = await import(viewURL);
