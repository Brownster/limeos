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
const source = await readFile(
  new URL("../src/ObservationView.tsx", import.meta.url),
  "utf8",
);
let code = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, jsx: ts.JsxEmit.ReactJSX },
}).outputText;
code = code
  .replace(
    '"react/jsx-runtime"',
    JSON.stringify(pathToFileURL(require.resolve("react/jsx-runtime")).href),
  )
  .replace(
    '"./model"',
    JSON.stringify(new URL("../src/model.ts", import.meta.url).href),
  );
const { ObservationView } = await import(
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
  const html = renderToStaticMarkup(
    React.createElement(ObservationView, {
      value: overview,
      items: [resource],
      history: null,
    }),
  );
  assert.match(html, /Unknown/);
  assert.match(html, /incomplete or out of date/);
  assert.match(html, /unavailable/);
  assert.match(html, /missing/);
  assert.match(html, /Read failed/);
  assert.match(html, /&lt;script&gt;/);
  assert.doesNotMatch(html, /<script>/);
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
  const html = renderToStaticMarkup(
    React.createElement(ObservationView, {
      value: overview,
      items: [],
      history: {
        range: "24h",
        from: 1,
        to: 2,
        bucket_seconds: 300,
        points: [],
        legacy_history:
          "Earlier history remains in the current build’s System history view.",
      },
    }),
  );
  assert.match(html, /Earlier history remains/);
  assert.match(html, /No resources in this view/);
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
