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
const modelURL = encode(await code("media-activity-model.ts"));
const clientURL = encode(await code("media-activity-client.ts"));
const controllerURL = encode((await code("media-activity-controller.ts")).replace('"./media-activity-client"', JSON.stringify(clientURL)).replace('"./media-activity-model"', JSON.stringify(modelURL)));
const viewURL = encode((await code("MediaActivityView.tsx")).replace('"./media-activity-model"', JSON.stringify(modelURL)));
const panelURL = encode((await code("MediaActivityPanel.tsx")).replace('"./MediaActivityView"', JSON.stringify(viewURL)).replace('"./media-activity-controller"', JSON.stringify(controllerURL)));
export const { activityExpired } = await import(modelURL);
export const { decodeMediaActivity, readMediaActivity, ActivityReadError } = await import(clientURL);
export const { MediaActivityController } = await import(controllerURL);
export const { MediaActivityView } = await import(viewURL);
export const { MediaActivityPanel } = await import(panelURL);
export const wire = JSON.parse(await readFile(new URL("../../../tests/fixtures/deployment/activity/wire.json", import.meta.url), "utf8"));
