import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";

const source = await readFile(new URL("../src/focus-ring.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const { wrapTarget } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

// Nodes in document order; compareDocumentPosition(other) reports where `other` is relative to this node.
const nodes = ["close", "heading", "refresh"].map((name, order) => ({
  name, order, compareDocumentPosition(other) { return other.order > order ? 4 : other.order < order ? 2 : 0; },
}));
const [close, heading, refresh] = nodes;

test("Tab and Shift+Tab wrap at the first and last control", () => {
  const controls = [close, refresh];
  assert.equal(wrapTarget(controls, refresh, false), close);
  assert.equal(wrapTarget(controls, close, true), refresh);
  assert.equal(wrapTarget(controls, close, false), null);
  assert.equal(wrapTarget(controls, refresh, true), null);
});

test("focus resting on a heading moves to the neighbouring control or wraps, never out of the dialog", () => {
  // The Resolved heading comes after the only control (Close): Tab used to leave the dialog.
  assert.equal(wrapTarget([close], heading, false), close);
  assert.equal(wrapTarget([close], heading, true), close);
  const both = [close, refresh];
  assert.equal(wrapTarget(both, heading, false), refresh);
  assert.equal(wrapTarget(both, heading, true), close);
});

test("nothing to wrap without controls or a current element", () => {
  assert.equal(wrapTarget([], heading, false), null);
  assert.equal(wrapTarget([close], null, false), null);
});
