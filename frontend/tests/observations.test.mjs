import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { percent, partial, page } from "../src/model.ts";
import { api, ApiError } from "../src/api.ts";

const require = createRequire(import.meta.url);
function moduleCode(source) {
  return ts
    .transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
        jsx: ts.JsxEmit.ReactJSX,
      },
    })
    .outputText.replace(
      '"react/jsx-runtime"',
      JSON.stringify(pathToFileURL(require.resolve("react/jsx-runtime")).href),
    )
    .replace(
      '"react"',
      JSON.stringify(pathToFileURL(require.resolve("react")).href),
    );
}
const actions = moduleCode(
  await readFile(
    new URL("../src/ContainerActions.tsx", import.meta.url),
    "utf8",
  ),
).replace(
  '"./api"',
  JSON.stringify(new URL("../src/api.ts", import.meta.url).href),
).replace(
  '"./ResolutionPanel"',
  JSON.stringify(
    `data:text/javascript,${encodeURIComponent("export const NeedsAttention = () => null; export const mergeAttention = (own) => own;")}`,
  ),
);
const actionsURL = `data:text/javascript;base64,${Buffer.from(actions).toString("base64")}`;
const source = await readFile(
  new URL("../src/ObservationView.tsx", import.meta.url),
  "utf8",
);
let code = moduleCode(source);
code = code
  .replace(
    '"react/jsx-runtime"',
    JSON.stringify(pathToFileURL(require.resolve("react/jsx-runtime")).href),
  )
  .replace(
    '"./model"',
    JSON.stringify(new URL("../src/model.ts", import.meta.url).href),
  )
  .replace('"./ContainerActions"', JSON.stringify(actionsURL));
const { PartialNotice, SourceList, ResourceTable, HistoryTable } = await import(
  `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`
);
const resource = {
  id: "uuid:data",
  name: "<script>alert(1)</script>",
  kind: "partition",
  status: "missing",
  source: "storage",
  parent: null,
  identity: "uuid:data",
  filesystem: "ext4",
  mountpoint: "/mnt/data",
  size_bytes: 100,
  image: null,
  health: null,
  cpu_percent: null,
  memory_percent: null,
};
const overview = {
  revision: 1,
  generated_at: Date.now() / 1000,
  host: null,
  resources: [resource],
  sources: [
    {
      source: "storage",
      state: "unavailable",
      sampled_at: 100,
      max_age_seconds: 90,
      warnings: ["Read failed"],
    },
  ],
};

test("missing evidence is never displayed as zero or a complete inventory", () => {
  // The old single ObservationView became PartialNotice, SourceList and
  // ResourceTable (Overview, Containers and Storage pages); host metrics moved
  // to the Overview cards, whose "Unknown" is covered in overview-model.test.
  const html = renderToStaticMarkup(
    React.createElement(React.Fragment, null,
      React.createElement(PartialNotice, { value: overview }),
      React.createElement(SourceList, { value: overview }),
      React.createElement(ResourceTable, { items: [resource] }),
    ),
  );
  assert.match(html, /incomplete or out of date/);
  assert.match(html, /unavailable/);
  assert.match(html, /missing/);
  assert.match(html, /Read failed/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
  const container = { ...resource, kind: "container", id: "container:x", image: "img", source: "docker", status: "running" };
  assert.match(renderToStaticMarkup(React.createElement(ResourceTable, { items: [container] })), /CPU Unknown · memory Unknown/);
});
test("pagination, stale timestamps and legitimate zero metrics remain distinct", () => {
  assert.equal(percent(null), "Unknown");
  assert.equal(percent(0), "0.0%");
  assert.equal(percent(NaN), "Unknown");
  assert.equal(
    partial(
      {
        ...overview,
        sources: [
          {
            ...overview.sources[0],
            state: "fresh",
            warnings: [],
            sampled_at: 100,
          },
        ],
      },
      200,
    ),
    true,
  );
  const resources = Array.from({ length: 25 }, (_, i) => ({
    ...resource,
    id: String(i),
  }));
  assert.equal(page(resources, "partition", 20).length, 5);
  assert.equal(page(resources, "container", 0).length, 0);
});
test("history explicitly preserves the separate legacy view", () => {
  const history = {
    range: "24h",
    from: 1,
    to: 2,
    bucket_seconds: 300,
    points: [{ at: 1, cpu_percent: 0, memory_percent: null, temperature_celsius: 41.25, disk_percent: 50 }],
    legacy_history:
      "Earlier history remains in the current build’s System history view.",
  };
  const html = renderToStaticMarkup(React.createElement(HistoryTable, { history }));
  assert.match(html, /Earlier history remains/);
  assert.match(html, /0\.0%/);
  assert.match(html, /Unknown/);
  assert.match(html, /41\.3 °C|41\.2 °C/);
  assert.match(renderToStaticMarkup(React.createElement(ResourceTable, { items: [] })), /No resources in this view/);
});
test("inventory requests use bounded pagination and propagate server details", async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async (url) => {
    assert.equal(
      url,
      "/api/v1/resources?limit=20&offset=20&kind=container&revision=7",
    );
    return new Response(
      JSON.stringify({
        code: "conflict",
        message: "Inventory changed. Reload the first page.",
        retry: false,
        audit_id: "read-7",
      }),
      { status: 409 },
    );
  };
  try {
    await assert.rejects(
      api.resources("container", 20, 7),
      (error) =>
        error instanceof ApiError &&
        error.detail.audit_id === "read-7" &&
        error.message === "Inventory changed. Reload the first page.",
    );
  } finally {
    globalThis.fetch = original;
  }
});
