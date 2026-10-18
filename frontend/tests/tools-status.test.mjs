import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { decodeToolsStatus, readToolsStatus, ageToolsStatus, toolsExpiryDelay, ToolsReadError, ToolsStatusController, ToolsStatus, ToolsStatusPanel, wire } from "./helpers/tools-status.mjs";

const clone = () => structuredClone(wire);
const decoded = () => decodeToolsStatus(clone());
const markup = (value, props = {}) => renderToStaticMarkup(React.createElement(ToolsStatus, { value, now: 105, ...props }));
const unknown = () => { const value = clone(); for (const key of ["config", "payload", "service"]) value[key] = { sampled_at: null, freshness: "unavailable", value: null }; return decodeToolsStatus(value); };
function staggered() { const value = clone(); value.payload.sampled_at = 103; value.service.sampled_at = 105; return decodeToolsStatus(value); }
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fakeClock() {
  let now = 105, next = 0; const tasks = new Map();
  return { now: () => now, schedule: (callback, ms) => { const id = ++next; tasks.set(id, { callback, ms }); return id; }, cancel: id => tasks.delete(id), advance: value => { now = value; },
    fire: () => { const [id, task] = tasks.entries().next().value; tasks.delete(id); task.callback(); }, tasks };
}
const capture = value => new Response(JSON.stringify(value));

test("Rust wire fixture supplies the three independent components", () => {
  assert.deepEqual(decoded(), wire); assert.equal(decoded().applied_configuration, "not_established");
});
test("selected fields discard private errors, arguments and arbitrary links", () => {
  const value = clone(); value.raw = "fixture-secret"; value.url = "https://attacker.invalid";
  value.config.value.config.extra_args = "--password fixture-secret"; value.service.value.environment = "fixture-secret";
  const admitted = decodeToolsStatus(value); assert.doesNotMatch(JSON.stringify(admitted), /fixture-secret|attacker|extra_args|environment/);
});
test("version, timestamps, loaded-result and component shape must match", () => {
  for (const change of [v => v.version = 2, v => v.max_age_seconds = 91, v => v.generated_at = -1, v => v.generated_at = 1.5, v => v.generated_at = 253402300800, v => v.applied_configuration = "verified", v => v.config = [], v => delete v.service, v => v.payload.value = true]) {
    const value = clone(); change(value); assert.throws(() => decodeToolsStatus(value), ToolsReadError);
  }
});
test("freshness enforces original sample and null-value relationships", () => {
  for (const part of [
    { sampled_at: null, freshness: "fresh", value: "present" }, { sampled_at: 100, freshness: "unavailable", value: null },
    { sampled_at: 100, freshness: "fresh", value: null }, { sampled_at: 106, freshness: "fresh", value: "present" },
    { sampled_at: 14, freshness: "fresh", value: "present" }, { sampled_at: 15, freshness: "stale", value: null },
    { sampled_at: 100, freshness: "future", value: null }, { sampled_at: 1, freshness: "stale", value: "present" },
  ]) { const value = clone(); value.payload = part; assert.throws(() => decodeToolsStatus(value)); }
  const value = clone(); value.payload = { sampled_at: 14, freshness: "stale", value: null }; assert.equal(decodeToolsStatus(value).payload.value, null);
});
test("nondefault values cannot masquerade as missing-file defaults", () => {
  for (const config of [{ share_path: "/data", port: 3923 }, { share_path: "/srv/copyparty", port: 8080 }]) {
    const value = clone(); value.config.value = { state: "defaults", config }; assert.throws(() => decodeToolsStatus(value));
  }
  const value = clone(); value.config.value = { state: "defaults", config: { share_path: "/srv/copyparty", port: 3923 } };
  assert.match(markup(decodeToolsStatus(value)), /Configuration file absent; defaults shown/);
});
test("path and port admission preserves Rust's literal rules", () => {
  const value = clone(); value.config.value.config = { share_path: "/srv/Media Library/🎵", port: 65535 };
  assert.deepEqual(decodeToolsStatus(value).config.value.config, value.config.value.config);
  for (const path of ["/", "relative", " /data", "/data/", "/data//tv", "/data/../tv", "/data/./tv", "/data/ tv", "/data\n", "/data\u0085", "/data\ud800", `/${"a".repeat(256)}`]) {
    const bad = clone(); bad.config.value.config.share_path = path; assert.throws(() => decodeToolsStatus(bad), undefined, path);
  }
  for (const port of [0, -1, 65536, 1.5, "8080", true, null]) { const bad = clone(); bad.config.value.config.port = port; assert.throws(() => decodeToolsStatus(bad)); }
});
test("UTF-8 path bounds and exact maximum components are admitted", () => {
  const value = clone(); value.config.value.config.share_path = `/${"é".repeat(127)}/${"a".repeat(255)}`;
  assert.equal(new TextEncoder().encode(value.config.value.config.share_path).length, 511); assert.doesNotThrow(() => decodeToolsStatus(value));
  value.config.value.config.share_path += "b"; assert.throws(() => decodeToolsStatus(value));
  value.config.value.config.share_path = `/${"a".repeat(255)}/${"b".repeat(255)}`; assert.doesNotThrow(() => decodeToolsStatus(value));
  value.config.value.config.share_path += "/x"; assert.throws(() => decodeToolsStatus(value));
});
test("typed unknown config and service causes never supply defaults or inactive", () => {
  const value = clone(); value.config.value = { state: "unknown", reason: "unavailable", error: "fixture-secret" };
  value.service.value = { state: "unknown", reason: "incomplete" };
  const html = markup(decodeToolsStatus(value)); assert.match(html, /Configuration could not be read/); assert.match(html, /Service observation incomplete/);
  assert.doesNotMatch(html, /defaults shown|Service inactive|fixture-secret|Media Library/);
});
test("unsupported enums and unknown reasons refuse the whole read", () => {
  for (const change of [v => v.config.value = { state: "unknown", reason: "new_reason" }, v => v.service.value = { state: "stopped" }, v => v.payload.value = "installed", v => v.config.freshness = "partial", v => v.config.value.state = "applied"]) {
    const value = clone(); change(value); assert.throws(() => decodeToolsStatus(value));
  }
});
test("independent age 90 survives and age 91 removes only expired fields", () => {
  const value = staggered(); assert.equal(ageToolsStatus(value, 190.999), value);
  const aged = ageToolsStatus(value, 191); assert.equal(aged.config.value, null); assert.equal(aged.config.sampled_at, 100);
  assert.equal(aged.config.freshness, "stale"); assert.equal(aged.payload.value, "present"); assert.equal(aged.service.value.state, "active");
  assert.equal(ageToolsStatus(aged, 194).payload.value, null); assert.equal(ageToolsStatus(aged, 194).service.value.state, "active");
  assert.equal(ageToolsStatus(aged, 196).service.value, null);
});
test("invalid clocks and rollback before report generation withhold everything", () => {
  for (const now of [NaN, Infinity, -1, 253402300800, 104.99]) assert.equal(ageToolsStatus(decoded(), now), null);
  assert.doesNotMatch(markup(decoded(), { now: 104 }), /Media Library|Service active|payload present/);
});
test("stale or future supplied components never become fresh with passing time", () => {
  const value = clone(); value.config = { sampled_at: 1, freshness: "stale", value: null }; value.service = { sampled_at: 106, freshness: "future", value: null };
  const admitted = decodeToolsStatus(value); assert.equal(ageToolsStatus(admitted, 190).config.value, null); assert.equal(ageToolsStatus(admitted, 190).service.value, null);
  assert.equal(toolsExpiryDelay(unknown(), 105), null);
});
test("view separates saved settings, payload presence and unverified loaded state", () => {
  const html = markup(decoded()); assert.match(html, /Saved configuration/); assert.match(html, /Media Library/); assert.match(html, /authenticity unverified/); assert.match(html, /Service active/);
  assert.match(html, /Loaded configuration is unverified/); assert.doesNotMatch(html, /href=|<form|Install|Configure|Restart|verified installation/);
});
test("expired component values never render even before an expiry callback", () => {
  const html = markup(staggered(), { now: 191 }); assert.doesNotMatch(html, /Media Library|8080/); assert.match(html, /Observation expired/);
  assert.match(html, /Service active/); assert.match(html, /payload present/); assert.match(html, /1970-01-01T00:01:40.000Z/);
});
test("fresh absent, failed and transition states stay distinguishable", () => {
  const value = clone(); value.payload.value = "absent";
  for (const [state, label] of [["inactive", "Service inactive"], ["failed", "Service failed"], ["activating", "Service starting"], ["deactivating", "Service stopping"], ["reloading", "Service reloading"], ["not_found", "Service unit missing"], ["masked", "Service unit masked"], ["load_error", "Service unit could not be loaded"]]) {
    value.service.value = { state }; const html = markup(decodeToolsStatus(value)); assert.match(html, new RegExp(label)); assert.match(html, /Legacy payload absent/);
  }
});
test("unavailable components render unknown without absent or inactive claims", () => {
  const html = markup(unknown()); assert.match(html, /Current configuration unknown/); assert.match(html, /Current payload presence unknown/); assert.match(html, /Current service state unknown/);
  assert.doesNotMatch(html, /payload absent|Service inactive|defaults shown/);
});
test("refresh has accessible loading and failure states", () => {
  const html = markup(decoded(), { loading: true, error: "Tools status is unavailable.", refresh: () => {} });
  assert.match(html, /aria-busy="true"/); assert.match(html, /role="status"/); assert.match(html, /role="alert"/); assert.match(html, /disabled=""/); assert.match(html, /Refresh tools status/);
});
test("multiple status views retain unique accessible heading relationships", () => {
  const html = renderToStaticMarkup(React.createElement(React.Fragment, {}, React.createElement(ToolsStatus, { value: null }), React.createElement(ToolsStatus, { value: null })));
  const labels = [...html.matchAll(/aria-labelledby="([^"]+)"/g)].map(match => match[1]);
  const ids = [...html.matchAll(/<h2 id="([^"]+)"/g)].map(match => match[1]);
  assert.equal(labels.length, 2); assert.equal(new Set(labels).size, 2); assert.deepEqual(labels, ids);
});
test("principal, grant and host lifetime key a fresh panel owner", () => {
  const props = { principal: "viewer", grantRevision: 1, hostLifetime: "host-a:boot-a:cache-1", expired: () => {} };
  const first = ToolsStatusPanel(props); assert.ok(first.key);
  for (const patch of [{ principal: "other" }, { grantRevision: 2 }, { hostLifetime: "host-a:boot-b:cache-2" }]) assert.notEqual(ToolsStatusPanel({ ...props, ...patch }).key, first.key);
  for (const patch of [{ principal: null }, { hostLifetime: null }, { hostLifetime: "" }, { grantRevision: null }, { grantRevision: 0 }, { grantRevision: 1.5 }, { grantRevision: Number.MAX_SAFE_INTEGER + 1 }]) {
    const html = renderToStaticMarkup(React.createElement(ToolsStatusPanel, { ...props, ...patch })); assert.match(html, /status is unknown/); assert.doesNotMatch(html, /Refresh tools status/);
  }
});
test("client requests only the fixed cached same-origin GET", async () => {
  const result = await readToolsStatus(new AbortController().signal, async (url, options) => {
    assert.equal(url, "/api/v1/deployment/tools/copyparty"); assert.equal(options.method, "GET"); assert.equal(options.credentials, "same-origin"); assert.equal(options.cache, "no-store"); assert.equal(options.redirect, "error"); assert.equal(options.body, undefined); return capture(wire);
  }); assert.deepEqual(result, wire);
});
test("HTTP access loss wins over malformed or private error bodies", async () => {
  for (const status of [401, 403]) {
    await assert.rejects(readToolsStatus(new AbortController().signal, async () => new Response("fixture-secret {{{", { status })), error => error instanceof ToolsReadError && error.status === status && !error.message.includes("fixture-secret"));
  }
});
test("other HTTP failures do not decode or display bodies", async () => {
  await assert.rejects(readToolsStatus(new AbortController().signal, async () => new Response("fixture-secret", { status: 503 })), error => error instanceof ToolsReadError && error.status === 503 && !error.message.includes("fixture-secret"));
});
test("invalid UTF-8, malformed JSON and missing bodies refuse status", async () => {
  for (const response of [new Response(Uint8Array.of(255)), new Response("{"), new Response(null)]) await assert.rejects(readToolsStatus(new AbortController().signal, async () => response), ToolsReadError);
});
test("exact streamed byte budget is inclusive and overflow cancels", async () => {
  const text = JSON.stringify(wire).padEnd(8192); assert.deepEqual(await readToolsStatus(new AbortController().signal, async () => new Response(text)), wire);
  let cancelled = false; const stream = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode(text + " ")); }, cancel() { cancelled = true; } });
  await assert.rejects(readToolsStatus(new AbortController().signal, async () => new Response(stream)), ToolsReadError); assert.equal(cancelled, true);
});
test("stream chunks preserve multibyte UTF-8 before decoding", async () => {
  const value = clone(); value.config.value.config.share_path = "/data/音楽"; const bytes = new TextEncoder().encode(JSON.stringify(value));
  const stream = new ReadableStream({ start(c) { for (const byte of bytes) c.enqueue(Uint8Array.of(byte)); c.close(); } });
  assert.equal((await readToolsStatus(new AbortController().signal, async () => new Response(stream))).config.value.config.share_path, "/data/音楽");
});
test("pre-aborted reads never fetch", async () => {
  const abort = new AbortController(); abort.abort(); let calls = 0;
  await assert.rejects(readToolsStatus(abort.signal, async () => { calls++; return capture(wire); })); assert.equal(calls, 0);
});
test("abort cancels a stalled body and withholds incomplete data", async () => {
  const abort = new AbortController(); const wait = deferred(); let cancelled = false;
  const stream = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("{")); wait.resolve(); }, cancel() { cancelled = true; } });
  const reading = readToolsStatus(abort.signal, async () => new Response(stream)); await wait.promise; await Promise.resolve(); abort.abort(); await assert.rejects(reading, ToolsReadError); assert.equal(cancelled, true);
});
test("the twelve-second deadline cancels a stalled read without polling", async () => {
  const original = globalThis.setTimeout, clear = globalThis.clearTimeout; let deadline; let cleared = false, cancelled = false;
  globalThis.setTimeout = (callback, ms) => { assert.equal(ms, 12000); deadline = callback; return 1; }; globalThis.clearTimeout = () => { cleared = true; };
  try {
    const started = deferred(); const stream = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("{")); started.resolve(); }, cancel() { cancelled = true; } });
    const reading = readToolsStatus(new AbortController().signal, async () => new Response(stream)); await started.promise; await Promise.resolve(); deadline(); await assert.rejects(reading); assert.equal(cancelled, true); assert.equal(cleared, true);
  } finally { globalThis.setTimeout = original; globalThis.clearTimeout = clear; }
});
test("controller coalesces reads and arms the earliest original deadline", async () => {
  const wait = deferred(), clock = fakeClock(); let calls = 0;
  const controller = new ToolsStatusController(() => {}, () => { calls++; return wait.promise; }, clock);
  const pending = controller.start(); assert.equal(controller.refresh(), pending); await Promise.resolve(); assert.equal(calls, 1);
  wait.resolve(staggered()); await pending; assert.equal(clock.tasks.size, 1); assert.equal([...clock.tasks.values()][0].ms, 86000); controller.dispose();
});
test("controller expiry removes each retained component without making requests", async () => {
  const clock = fakeClock(); let calls = 0;
  const controller = new ToolsStatusController(() => {}, async () => { calls++; return staggered(); }, clock); await controller.start();
  clock.advance(191); clock.fire(); let value = controller.getSnapshot().value; assert.equal(value.config.value, null); assert.equal(value.payload.value, "present"); assert.equal(value.service.value.state, "active");
  assert.equal([...clock.tasks.values()][0].ms, 3000); clock.advance(194); clock.fire(); assert.equal(controller.getSnapshot().value.payload.value, null);
  clock.advance(196); clock.fire(); assert.equal(controller.getSnapshot().value.service.value, null); assert.equal(clock.tasks.size, 0); assert.equal(calls, 1); controller.dispose();
});
test("early timers rearm the original deadline", async () => {
  const clock = fakeClock(); const controller = new ToolsStatusController(() => {}, async () => decoded(), clock); await controller.start();
  clock.advance(190.999); clock.fire(); assert.ok(controller.getSnapshot().value.config.value); assert.ok([...clock.tasks.values()][0].ms < 2);
  clock.advance(191); clock.fire(); assert.equal(controller.getSnapshot().value.config.value, null); controller.dispose();
});
test("a late cache reply cannot restore expired component values", async () => {
  const clock = fakeClock(); clock.advance(191); const controller = new ToolsStatusController(() => {}, async () => staggered(), clock); await controller.start();
  assert.equal(controller.getSnapshot().value.config.value, null); assert.equal(controller.getSnapshot().value.service.value.state, "active"); controller.dispose();
});
test("rollback clears private state while a refresh is pending", async () => {
  const clock = fakeClock(), wait = deferred(); let calls = 0; const controller = new ToolsStatusController(() => {}, () => ++calls === 1 ? Promise.resolve(decoded()) : wait.promise, clock);
  await controller.start(); clock.advance(104); const pending = controller.refresh(); assert.equal(controller.getSnapshot().value, null); wait.resolve(decoded()); await pending; assert.equal(controller.getSnapshot().value, null); assert.equal(clock.tasks.size, 0); controller.dispose();
});
test("read failures clear retained values and hide raw diagnostics", async () => {
  let calls = 0; const controller = new ToolsStatusController(() => {}, async () => { if (++calls === 1) return decoded(); throw new Error("fixture-secret"); }, fakeClock());
  await controller.start(); await controller.refresh(); assert.equal(controller.getSnapshot().value, null); assert.doesNotMatch(controller.getSnapshot().error, /fixture-secret/); controller.dispose();
});
test("401 and 403 clear status and stop further reads", async () => {
  for (const status of [401, 403]) {
    let calls = 0, expired = 0; const clock = fakeClock(); const controller = new ToolsStatusController(() => { expired++; }, async () => { if (++calls === 1) return decoded(); throw new ToolsReadError("fixture-secret", status); }, clock);
    await controller.start(); await controller.refresh(); const state = controller.getSnapshot(); assert.equal(state.value, null); assert.equal(state.expired, status === 401); assert.equal(state.denied, status === 403); assert.equal(expired, status === 401 ? 1 : 0);
    await controller.refresh(); assert.equal(calls, 2); assert.equal(clock.tasks.size, 0); assert.doesNotMatch(state.error, /fixture-secret/); controller.dispose();
  }
});
test("disposing before deferred startup launches no read", async () => {
  let calls = 0; const controller = new ToolsStatusController(() => {}, async () => { calls++; return decoded(); }, fakeClock()); const pending = controller.start(); controller.dispose(); await pending; assert.equal(calls, 0); assert.equal(controller.getSnapshot().value, null);
});
test("old lifetime cancellation ignores late data and errors", async () => {
  for (const outcome of ["resolve", "reject"]) {
    const wait = deferred(); let signal; const old = new ToolsStatusController(() => {}, s => { signal = s; return wait.promise; }, fakeClock()); const pending = old.start(); await Promise.resolve(); old.dispose(); assert.equal(signal.aborted, true);
    const next = new ToolsStatusController(() => {}, async () => unknown(), fakeClock()); await next.start(); wait[outcome](outcome === "resolve" ? decoded() : new Error("fixture-secret")); await pending;
    assert.equal(old.getSnapshot().value, null); assert.equal(old.getSnapshot().error, ""); assert.equal(next.getSnapshot().value.config.freshness, "unavailable"); next.dispose();
  }
});
test("synchronous failure coalesces and disposed controller can restart safely", async () => {
  let calls = 0; const controller = new ToolsStatusController(() => {}, () => { calls++; throw new Error("fixture-secret"); }, fakeClock()); const pending = controller.start(); assert.equal(controller.refresh(), pending); await pending; assert.equal(calls, 1); controller.dispose(); await controller.start(); assert.equal(calls, 2); controller.dispose();
});
