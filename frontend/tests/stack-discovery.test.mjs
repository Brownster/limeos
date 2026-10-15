import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { decodeStackInventory, readStackInventory, inventoryStale, StackReadError, StackInventoryController, StackManagement, wire } from "./helpers/stack-discovery.mjs";

const clone = () => structuredClone(wire);
const render = props => renderToStaticMarkup(React.createElement(StackManagement, { refresh() {}, ...props }));
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fakeClock() { let now = 101; let serial = 0; const timers = new Map(); return { now: () => now, schedule(fn, ms) { timers.set(++serial, { fn, ms }); return serial; }, cancel(id) { timers.delete(id); }, advance(at) { now = at; for (const [id, { fn }] of [...timers]) { timers.delete(id); fn(); } }, timers }; }

test("Rust-bound wire preserves replicas, one-offs and exact case paths", () => {
  const value = decodeStackInventory(clone());
  assert.equal(value.stacks[0].working_directory, "/opt/stacks/Media");
  assert.equal(value.stacks[0].services[0].members.length, 3);
  assert.equal(value.stacks[0].counts.one_offs, 1);
  const html = render({ value });
  assert.match(html, /Media/); assert.match(html, /1 of 2 service containers running/);
  assert.match(html, /One-off/); assert.match(html, /Service \/ replica/);
  assert.doesNotMatch(html, />Start<|>Stop<|>Apply<|>Delete</);
});

test("unknown initial state never becomes an empty inventory", () => {
  const html = render({ value: null });
  assert.match(html, /Stack inventory is unavailable/);
  assert.doesNotMatch(html, /No stacks are registered|0 service containers/);
});

test("file conflicts and unsupported source diagnostics reach the UI", () => {
  const value = clone(); value.stacks[0].files = { state: "conflict", candidates: ["compose.yaml", "docker-compose.yml"] };
  assert.match(render({ value }), /Multiple Compose files: compose.yaml, docker-compose.yml/);
  value.stacks[0].files = { state: "unsupported", reason: "environment" };
  assert.match(render({ value }), /unsupported \(environment\)/);
  value.stacks[0].files = { state: "unavailable" };
  assert.match(render({ value }), /source inspection is unavailable/);
});

test("stale UI withholds current counts and labels retained members as previous", () => {
  const html = render({ value: clone(), stale: true });
  assert.match(html, /Current container counts are unknown/);
  assert.match(html, /previously observed members/);
  assert.doesNotMatch(html, /1 of 2 service containers running/);
});

test("decoder refuses malformed, duplicate and oversized member reports", () => {
  for (const change of [v => { v.version = 2; }, v => { v.catalog.sampled_at = null; },
    v => { v.stacks[0].counts.running = 99; }, v => { v.stacks[0].services[0].members.push(v.stacks[0].services[0].members[0]); },
    v => { v.stacks = Array(9).fill(v.stacks[0]); }, v => { v.stacks[0].files = { state: "unsupported", reason: "shell" }; },
    v => { v.stacks[0].services[0].members[0].container.resource = "container:short"; }]) {
    const value = clone(); change(value); assert.throws(() => decodeStackInventory(value), StackReadError);
  }
});

test("age boundaries and clock rollback remain explicit", () => {
  assert.equal(inventoryStale(wire, 190), false); assert.equal(inventoryStale(wire, 191), true);
  assert.equal(inventoryStale(wire, 99), true);
});

test("client uses one fixed GET and preserves bounded server error details", async () => {
  const calls = []; const value = await readStackInventory(new AbortController().signal, async (url, init) => {
    calls.push({ url, init }); return new Response(JSON.stringify(wire));
  });
  assert.equal(value.version, 1); assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/v1/deployment/stacks"); assert.equal(calls[0].init.method, "GET");
  assert.equal(calls[0].init.credentials, "same-origin"); assert.equal(calls[0].init.cache, "no-store");
  await assert.rejects(readStackInventory(new AbortController().signal, async () => new Response(JSON.stringify({ message: "Collection budget exceeded", code: "unavailable", audit_id: "read-1" }), { status: 503 })), error => error.message === "Collection budget exceeded" && error.auditId === "read-1");
});

test("client caps streamed bodies and handles access loss before reading payloads", async () => {
  let cancelled = false;
  const body = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array(65_537)); }, cancel() { cancelled = true; } });
  await assert.rejects(readStackInventory(new AbortController().signal, async () => new Response(body)), StackReadError);
  assert.equal(cancelled, true);
  for (const status of [401, 403]) {
    await assert.rejects(readStackInventory(new AbortController().signal, async () => new Response("unreadable-private-error", { status })), error => error.status === status);
  }
});

test("concurrent refresh requests coalesce", async () => {
  const task = deferred(); let calls = 0; const clock = fakeClock();
  const controller = new StackInventoryController(() => {}, () => { calls++; return task.promise; }, clock);
  const first = controller.start(); const second = controller.refresh(); assert.equal(first, second);
  await Promise.resolve(); assert.equal(calls, 1); task.resolve(clone()); await first;
  assert.equal(controller.getSnapshot().loading, false); assert.equal(controller.getSnapshot().stale, false); controller.dispose();
});

test("default browser clock can construct and read without injection", async () => {
  const controller = new StackInventoryController(() => {}, async () => clone());
  await controller.start(); assert.equal(controller.getSnapshot().value.version, 1); controller.dispose();
});

test("failed refresh retains visibly stale data and the next read recovers", async () => {
  let broken = false; const controller = new StackInventoryController(() => {}, async () => { if (broken) throw new Error("Offline"); return clone(); }, fakeClock());
  await controller.start(); broken = true; await controller.refresh();
  assert.equal(controller.getSnapshot().value.stacks.length, 1); assert.equal(controller.getSnapshot().stale, true);
  assert.equal(controller.getSnapshot().error, "Offline"); broken = false; await controller.refresh();
  assert.equal(controller.getSnapshot().stale, false); controller.dispose();
});

test("expiry clears data and signals once; forbidden clears without login expiry", async () => {
  for (const status of [401, 403]) {
    let expired = 0; let denied = false;
    const controller = new StackInventoryController(() => expired++, async () => { if (denied) throw new StackReadError("Access changed", status); return clone(); }, fakeClock());
    await controller.start(); denied = true; await controller.refresh(); await controller.refresh();
    assert.equal(controller.getSnapshot().value, null); assert.equal(expired, status === 401 ? 1 : 0);
    assert.equal(controller.getSnapshot().denied, status === 403); controller.dispose();
  }
});

test("dispose aborts and ignores a late response; strict-effect restart works", async () => {
  const task = deferred(); let signal; let count = 0;
  const controller = new StackInventoryController(() => {}, s => { signal = s; return ++count === 1 ? task.promise : Promise.resolve(clone()); }, fakeClock());
  const old = controller.start(); await Promise.resolve(); controller.dispose(); assert.equal(signal.aborted, true);
  await controller.start(); const before = controller.getSnapshot(); task.resolve(clone()); await old;
  assert.equal(controller.getSnapshot(), before); assert.equal(count, 2); controller.dispose();
});

test("freshness expires without polling, including during a pending refresh", async () => {
  const clock = fakeClock(); const task = deferred(); let count = 0;
  const controller = new StackInventoryController(() => {}, () => ++count === 1 ? Promise.resolve(clone()) : task.promise, clock);
  await controller.start(); const refresh = controller.refresh(); await Promise.resolve();
  clock.advance(191); assert.equal(controller.getSnapshot().stale, true);
  assert.equal(count, 2); task.reject(new Error("Offline")); await refresh; controller.dispose();
});

test("focus refresh marks aged observations stale before a response arrives", async () => {
  const clock = fakeClock(); const task = deferred(); let count = 0; let now = 101;
  clock.now = () => now;
  const controller = new StackInventoryController(() => {}, () => ++count === 1 ? Promise.resolve(clone()) : task.promise, clock);
  await controller.start(); now = 200; const refresh = controller.refresh();
  assert.equal(controller.getSnapshot().stale, true); task.resolve(clone()); await refresh; controller.dispose();
});

test("refresh triggered by a subscriber still coalesces into one request", async () => {
  const task = deferred(); let count = 0; let triggered = false;
  const controller = new StackInventoryController(() => {}, () => { count++; return task.promise; }, fakeClock());
  const unsubscribe = controller.subscribe(() => { if (!triggered) { triggered = true; void controller.refresh(); } });
  const loading = controller.start(); await Promise.resolve();
  const observed = count; task.resolve(clone()); await loading; unsubscribe(); controller.dispose(); assert.equal(observed, 1);
});

test("dispose before dispatch prevents a late request from being started", async () => {
  let count = 0; const controller = new StackInventoryController(() => {}, async () => { count++; return clone(); }, fakeClock());
  const loading = controller.start(); controller.dispose(); await loading; assert.equal(count, 0);
});

test("malformed JSON yields a safe inventory error", async () => {
  await assert.rejects(readStackInventory(new AbortController().signal, async () => new Response("PRIVATE_RAW_INVALID_BODY")), error => error instanceof StackReadError && !error.message.includes("PRIVATE_RAW_INVALID_BODY"));
});
