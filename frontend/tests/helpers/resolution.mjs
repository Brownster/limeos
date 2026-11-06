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
// The real controller and panel; the HTTP client and dialog helpers are stubbed.
const controllerURL = encode(await code("resolution-controller.ts"));
const clientURL = encode("export const resolutionClient = {};");
const uiURL = encode("export const containFocus = () => {};");
const panelURL = encode((await code("ResolutionPanel.tsx"))
  .replace('"./resolution-controller"', JSON.stringify(controllerURL))
  .replace('"./resolution-client"', JSON.stringify(clientURL))
  .replace('"./operation-ui"', JSON.stringify(uiURL)));
export const { ResolutionController, validateReason, disabledReason, asView, isJobId, declarations } = await import(controllerURL);
export const { NeedsAttention, ResolutionBody, restoreFocus } = await import(panelURL);
