import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { decodeMediaActivity, readMediaActivity, ActivityReadError, MediaActivityController, MediaActivityPanel, MediaActivityView, wire } from "./helpers/media-activity.mjs";

const clone = () => structuredClone(wire);
const nullTotals = () => Object.fromEntries(Object.keys(wire.totals).map(key => [key, null]));
const id = n => `container:${n.toString(16).padStart(64, "0")}`;
function service(provider, n = 2) {
  const s = structuredClone(wire.services[0]); s.target.provider = provider; s.target.name = provider; s.target.container.resource = id(n); return s;
}
function empty() { const v = clone(); v.services = []; v.streams = []; v.totals.streams = 0; v.totals.transcodes = 0; return v; }
function mixed() {
  const v = clone(); v.services.push(service("transmission"), service("sonarr", 3));
  v.downloads.push({ source: id(2), data: { name: "Example transfer", state: "seeding", progress_percent: 100, size_bytes: 1234, remaining_bytes: 0, speed_bytes: null, eta: null } });
  v.queues.push({ source: id(3), data: { total: 1, observed: 1, problems: 1, complete: true, items: [{ title: "Queue example", state: "warning", status: "downloading" }] } });
  Object.assign(v.totals, { downloads: 1, download_speed: null, queued: 1, queue_problems: 1 }); return v;
}
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fakeClock() {
  let now = 101.25; let serial = 0; const timers = new Map();
  return { now: () => now, schedule(fn, ms) { timers.set(++serial, { fn, at: now + ms / 1000 }); return serial; }, cancel(n) { timers.delete(n); },
    set(at) { now = at; }, advance(at) { now = at; for (const [n, timer] of [...timers]) if (timer.at <= now) { timers.delete(n); timer.fn(); } }, timers };
}
const html = controller => renderToStaticMarkup(React.createElement(MediaActivityView, { ...controller.getSnapshot(), now: 109, refresh() {} }));

test("admission accepts the Rust wire and all three provider families", () => {
  assert.deepEqual(decodeMediaActivity(clone()), wire);
  const v = decodeMediaActivity(mixed()); assert.equal(v.queues[0].data.problems, 1); assert.equal(v.downloads[0].data.speed_bytes, null);
  assert.equal(decodeMediaActivity(empty()).complete, true);
});
test("failed peer and incomplete discovery preserve good rows with null totals", () => {
  const v = clone(); const s = service("radarr"); s.state = "credential_unavailable";
  v.services.push(s); v.complete = false; v.totals = nullTotals(); assert.equal(decodeMediaActivity(v).streams.length, 1);
  v.source = "incomplete"; v.services.pop(); assert.equal(decodeMediaActivity(v).complete, false);
});
test("partial queue admits observed-page problems without aggregate claims", () => {
  const v = mixed(); v.complete = false; v.totals = nullTotals(); v.services[2].state = "partial";
  Object.assign(v.queues[0].data, { total: 30, observed: 2, complete: false });
  assert.equal(decodeMediaActivity(v).queues[0].data.total, 30);
});
test("all unavailable source variants have empty output and explicit time relationships", () => {
  for (const source of ["unavailable", "identity_mismatch", "stale", "future"]) {
    const v = empty(); v.source = source; v.complete = false; v.totals = nullTotals();
    v.sampled_at = source === "unavailable" ? null : source === "future" ? 102 : source === "stale" ? 92 : 100;
    assert.equal(decodeMediaActivity(v).source, source);
  }
});
test("version, structure, states and private extra fields are refused", () => {
  const changes = [v => { v.version = 2; }, v => { v.max_age_seconds = 90; }, v => { v.source = "ok"; },
    v => { v.services[0].state = "healthy"; }, v => { v.services[0].target.provider = "plex"; },
    v => { v.api_key = "fixture-private"; }, v => { v.services[0].target.container.environment = "fixture-private"; },
    v => { v.streams[0].data.credential = "fixture-private"; }, v => { delete v.totals.download_speed; },
    v => { v.streams[0].data = []; }, v => { v.services = Array(9).fill(v.services[0]); }];
  for (const change of changes) { const v = clone(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError); }
});
test("full identities and actual Engine calendar timestamps are required", () => {
  const changes = [v => { v.services[0].target.container.resource = "container:short"; },
    v => { v.services[0].target.container.image = `sha256:${"A".repeat(64)}`; }, v => { v.services.push(v.services[0]); },
    v => { v.services[0].target.container.started_at = "2025-02-29T00:00:00Z"; }, v => { v.services[0].target.container.started_at = "2026-10-15T24:00:00Z"; },
    v => { v.services[0].target.container.started_at = "0000-01-01T00:00:00Z"; }, v => { v.services[0].target.container.started_at = "2026-01-01T00:00:00+00:00"; }];
  for (const change of changes) { const v = clone(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError); }
  for (const time of ["2024-02-29T23:59:59Z", "0001-01-01T00:00:00.1Z", "9999-12-31T23:59:59.999999999Z"]) {
    const v = clone(); v.services[0].target.container.started_at = time; decodeMediaActivity(v);
  }
});
test("rows must reference an admitted active service of the matching provider family", () => {
  for (const change of [v => { v.streams[0].source = id(7); }, v => { v.services[0].target.provider = "transmission"; },
    v => { v.services[0].state = "timeout"; v.complete = false; v.totals = nullTotals(); },
    v => { v.services[0].state = "stopped"; v.services[0].target.container.running = false; v.complete = false; v.totals = nullTotals(); }]) {
    const v = clone(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError);
  }
});
test("times, freshness and completeness cannot contradict displayed observations", () => {
  const changes = [v => { v.generated_at = 99; }, v => { v.generated_at = 109; }, v => { v.sampled_at = null; },
    v => { v.services[0].sampled_at = 99; }, v => { v.services[0].sampled_at = 102; }, v => { v.services[0].sampled_at = null; },
    v => { v.services[0].target.container.running = false; }, v => { v.complete = false; }, v => { v.source = "incomplete"; },
    v => { v.services[0].state = "partial"; v.complete = false; v.totals = nullTotals(); }, v => { v.source = "stale"; }];
  for (const change of changes) { const v = clone(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError); }
});
test("UTF-8 byte bounds, controls, lone surrogates and finite metric bounds are enforced", () => {
  for (const title of ["é".repeat(101), "a\u0085b", "a\u0000b", "\ud800"]) {
    const v = clone(); v.streams[0].data.title = title; assert.throws(() => decodeMediaActivity(v), ActivityReadError);
  }
  const v = clone(); v.streams[0].data.title = "é".repeat(100); decodeMediaActivity(v);
  for (const number of [-1, 101, Number.NaN, Number.POSITIVE_INFINITY, "1"]) {
    const v = clone(); v.streams[0].data.progress_percent = number; assert.throws(() => decodeMediaActivity(v), ActivityReadError);
  }
  v.streams[0].data.bitrate = Number.MAX_SAFE_INTEGER + 1; assert.throws(() => decodeMediaActivity(v), ActivityReadError);
});
test("totals, omissions and per-family capacities agree with visible rows", () => {
  for (const change of [v => { v.totals.streams = 0; }, v => { v.totals.transcodes = 0; }, v => { v.totals.downloads = 1; },
    v => { v.totals.download_speed = null; }, v => { v.omitted_streams = 1; }, v => { v.streams = Array(13).fill(v.streams[0]); },
    v => { v.totals.queued = 1; }, v => { v.totals.streams = null; }]) {
    const v = clone(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError);
  }
  const v = clone(); v.streams = Array(12).fill(v.streams[0]); v.omitted_streams = 12; v.totals.streams = 24; v.totals.transcodes = 24;
  decodeMediaActivity(v); v.omitted_streams++; v.totals.streams++; assert.throws(() => decodeMediaActivity(v), ActivityReadError);
});
test("queues require unique service coverage, bounded details and consistent sampled counts", () => {
  for (const change of [v => { v.queues = []; }, v => { v.queues.push(v.queues[0]); }, v => { v.queues[0].data.problems = 2; },
    v => { v.queues[0].data.total = 2; }, v => { v.queues[0].data.items = Array(4).fill(v.queues[0].data.items[0]); },
    v => { v.queues[0].data.complete = false; }, v => { v.totals.queue_problems = 0; }]) {
    const v = mixed(); change(v); assert.throws(() => decodeMediaActivity(v), ActivityReadError);
  }
});
test("client performs exactly one fixed authenticated GET without caller parameters", async () => {
  const calls = []; const signal = new AbortController().signal;
  await readMediaActivity(signal, async (url, init) => { calls.push({ url, init }); return new Response(JSON.stringify(wire)); });
  assert.equal(calls.length, 1); const { url, init } = calls[0]; assert.equal(url, "/api/v1/deployment/activity");
  assert.equal(init.method, "GET"); assert.equal(init.credentials, "same-origin"); assert.equal(init.cache, "no-store"); assert.equal(init.redirect, "error");
  assert.equal(init.headers.Accept, "application/json"); assert.equal(init.body, undefined); assert.equal(signal.aborted, false);
});
test("streamed body limit includes every chunk and cancels overflow before parsing", async () => {
  let cancelled = 0; const bytes = new TextEncoder().encode(JSON.stringify(wire));
  const good = new ReadableStream({ start(c) { c.enqueue(bytes.subarray(0, 11)); c.enqueue(bytes.subarray(11)); c.close(); } });
  assert.equal((await readMediaActivity(new AbortController().signal, async () => new Response(good))).version, 1);
  const large = new ReadableStream({ start(c) { c.enqueue(new Uint8Array(65536)); c.enqueue(new Uint8Array(1)); }, cancel() { cancelled++; } });
  await assert.rejects(readMediaActivity(new AbortController().signal, async () => new Response(large)), ActivityReadError); assert.equal(cancelled, 1);
  const json = JSON.stringify(wire); const padded = json + " ".repeat(65536 - new TextEncoder().encode(json).length);
  assert.equal(new TextEncoder().encode(padded).length, 65536);
  await readMediaActivity(new AbortController().signal, async () => new Response(padded));
});
test("malformed JSON, UTF-8 and network failures expose fixed safe errors", async () => {
  for (const response of [new Response("PRIVATE_RESPONSE"), new Response(new Uint8Array([0xff])), new Response("null"), new Response(null)]) {
    await assert.rejects(readMediaActivity(new AbortController().signal, async () => response), error => error instanceof ActivityReadError && !error.message.includes("PRIVATE"));
  }
  await assert.rejects(readMediaActivity(new AbortController().signal, async () => { throw new Error("PRIVATE_URL_TOKEN"); }), error => !error.message.includes("PRIVATE"));
});
test("access loss is recognized before any error body is read", async () => {
  for (const status of [401, 403, 503]) {
    let read = 0; let cancelled = 0;
    const response = { ok: false, status, body: { getReader() { read++; throw new Error("PRIVATE"); }, cancel() { cancelled++; return Promise.resolve(); } } };
    await assert.rejects(readMediaActivity(new AbortController().signal, async () => response), error => error.status === status && !error.message.includes("PRIVATE"));
    assert.equal(read, 0); assert.equal(cancelled, 1);
  }
});
test("an already-aborted read never calls fetch", async () => {
  const abort = new AbortController(); abort.abort(); let calls = 0;
  await assert.rejects(readMediaActivity(abort.signal, async () => { calls++; return new Response("{}"); }), ActivityReadError); assert.equal(calls, 0);
});
test("abort terminates a stalled body and releases its reader", async () => {
  const abort = new AbortController(); let cancelled = 0;
  const stream = new ReadableStream({ cancel() { cancelled++; } }); const response = new Response(stream);
  const work = readMediaActivity(abort.signal, async () => response); await Promise.resolve(); abort.abort();
  await assert.rejects(work, ActivityReadError); await Promise.resolve(); assert.equal(cancelled, 1); assert.equal(stream.locked, false);
});
test("12-second deadline terminates fetch even when the adapter ignores abort", async context => {
  context.mock.timers.enable({ apis: ["setTimeout"] }); const task = deferred(); let signal;
  const work = readMediaActivity(new AbortController().signal, async (_url, init) => { signal = init.signal; return task.promise; });
  context.mock.timers.tick(12000); await assert.rejects(work, /timed out/); assert.equal(signal.aborted, true);
  let cancelled = false; task.resolve(new Response(new ReadableStream({ cancel() { cancelled = true; } })));
  await Promise.resolve(); await Promise.resolve(); assert.equal(cancelled, true);
});
test("deadline also cancels a stalled response body; successful reads clear their timeout", async context => {
  context.mock.timers.enable({ apis: ["setTimeout"] }); let cancelled = 0;
  const stream = new ReadableStream({ cancel() { cancelled++; } });
  const work = readMediaActivity(new AbortController().signal, async () => new Response(stream)); await Promise.resolve();
  context.mock.timers.tick(12000); await assert.rejects(work, /timed out/); await Promise.resolve();
  assert.equal(cancelled, 1); assert.equal(stream.locked, false);
  let signal; await readMediaActivity(new AbortController().signal, async (_url, init) => { signal = init.signal; return new Response(JSON.stringify(wire)); });
  context.mock.timers.tick(12000); assert.equal(signal.aborted, false);
});
test("service failures preserve their explicit diagnostic state without current rows", () => {
  for (const state of ["stopped", "stale", "future", "identity_mismatch", "unavailable", "credential_unavailable", "timeout", "invalid_response", "unsupported"]) {
    const v = clone(); v.complete = false; v.totals = nullTotals(); v.streams = []; const s = v.services[0]; s.state = state;
    if (state === "stopped") { s.target.container.running = false; s.sampled_at = null; }
    if (state === "stale") s.sampled_at = 99;
    if (state === "future") s.sampled_at = 102;
    if (state === "unavailable") s.sampled_at = null;
    assert.equal(decodeMediaActivity(v).services[0].state, state);
  }
});
test("mount/focus/reconnect/manual refreshes coalesce and subscriptions update", async () => {
  const task = deferred(); let reads = 0; let updates = 0; const clock = fakeClock();
  const controller = new MediaActivityController(() => {}, () => { reads++; return task.promise; }, clock);
  const unsubscribe = controller.subscribe(() => updates++); const first = controller.start();
  assert.equal(controller.refresh(), first); assert.equal(controller.refresh(), first); await Promise.resolve(); assert.equal(reads, 1);
  task.resolve(clone()); await first; assert.equal(controller.getSnapshot().stale, false); assert.equal(controller.getSnapshot().loading, false); assert.ok(updates >= 2);
  unsubscribe(); const count = updates; controller.dispose(); assert.equal(updates, count);
});
test("original eight-second cohort expires at second nine without polling", async () => {
  const clock = fakeClock(); let reads = 0; const controller = new MediaActivityController(() => {}, async () => { reads++; return clone(); }, clock);
  await controller.start(); assert.equal(clock.timers.size, 1); clock.advance(108.999); assert.equal(controller.getSnapshot().stale, false);
  clock.advance(109); assert.equal(controller.getSnapshot().stale, true); assert.equal(clock.timers.size, 0); assert.equal(reads, 1);
  assert.match(html(controller), /Previous observation/); assert.doesNotMatch(html(controller), /25.0%|1 \/ 1/); controller.dispose();
});
test("early timer callbacks preserve the deadline and clock rollback invalidates current data", async () => {
  const clock = fakeClock(); const controller = new MediaActivityController(() => {}, async () => clone(), clock); await controller.start();
  const [n, timer] = [...clock.timers][0]; clock.timers.delete(n); timer.fn(); assert.equal(controller.getSnapshot().stale, false); assert.equal(clock.timers.size, 1);
  clock.set(99); await controller.refresh(); assert.equal(controller.getSnapshot().stale, true); assert.equal(clock.timers.size, 0); controller.dispose();
});
test("pending refresh cannot extend old data and a coalesced focus detects suspended timers", async () => {
  const task = deferred(); const clock = fakeClock(); let reads = 0;
  const controller = new MediaActivityController(() => {}, () => ++reads === 1 ? Promise.resolve(clone()) : task.promise, clock);
  await controller.start(); const work = controller.refresh(); await Promise.resolve(); clock.set(109);
  assert.equal(controller.refresh(), work); assert.equal(controller.getSnapshot().stale, true); assert.equal(reads, 2);
  task.resolve(clone()); await work; assert.equal(controller.getSnapshot().stale, true); assert.equal(clock.timers.size, 0); controller.dispose();
});
test("a genuinely newer sample replaces old expiry while failed refresh is visibly previous", async () => {
  const clock = fakeClock(); let broken = false; const fresh = clone(); fresh.sampled_at = 102; fresh.generated_at = 102; fresh.services[0].sampled_at = 102;
  const controller = new MediaActivityController(() => {}, async () => { if (broken) throw new Error("PRIVATE_TOKEN"); return clock.now() >= 102 ? structuredClone(fresh) : clone(); }, clock);
  await controller.start(); clock.advance(102); await controller.refresh(); assert.equal(clock.timers.size, 1);
  clock.advance(109); assert.equal(controller.getSnapshot().stale, false); broken = true; await controller.refresh();
  assert.equal(controller.getSnapshot().value.sampled_at, 102); assert.equal(controller.getSnapshot().stale, true); assert.equal(clock.timers.size, 0); assert.doesNotMatch(controller.getSnapshot().error, /PRIVATE/);
  broken = false; await controller.refresh(); assert.equal(controller.getSnapshot().stale, false); clock.advance(111); assert.equal(controller.getSnapshot().stale, true); controller.dispose();
});
test("401 clears private data and expires once; 403 clears without expiring the login", async () => {
  for (const status of [401, 403]) {
    const clock = fakeClock(); let expired = 0; let fail = false; let reads = 0;
    const controller = new MediaActivityController(() => expired++, async () => { reads++; if (fail) throw new ActivityReadError("PRIVATE", status); return clone(); }, clock);
    await controller.start(); fail = true; await controller.refresh(); await controller.refresh();
    assert.equal(controller.getSnapshot().value, null); assert.equal(controller.getSnapshot().denied, status === 403); assert.equal(controller.getSnapshot().expired, status === 401);
    assert.equal(expired, status === 401 ? 1 : 0); assert.equal(reads, 2); assert.equal(clock.timers.size, 0); controller.dispose();
  }
});
test("logout/disposal aborts and ignores obsolete success and failure after remount", async () => {
  for (const reject of [false, true]) {
    const task = deferred(); let signal; let reads = 0; let expired = 0; const clock = fakeClock();
    const controller = new MediaActivityController(() => expired++, s => { signal = s; return ++reads === 1 ? task.promise : Promise.resolve(empty()); }, clock);
    const work = controller.start(); await Promise.resolve(); controller.dispose(); assert.equal(signal.aborted, true); assert.equal(controller.getSnapshot().value, null);
    await controller.start(); const before = controller.getSnapshot(); if (reject) task.reject(new ActivityReadError("Old login", 401)); else task.resolve(clone());
    await work; assert.equal(controller.getSnapshot(), before); assert.equal(expired, 0); assert.equal(reads, 2); controller.dispose(); assert.equal(clock.timers.size, 0);
  }
});
test("disposal before deferred launch suppresses the request; synchronous failure settles", async () => {
  let reads = 0; const controller = new MediaActivityController(() => {}, () => { reads++; throw new Error("PRIVATE"); }, fakeClock());
  const work = controller.start(); controller.dispose(); await work; assert.equal(reads, 0);
  await controller.start(); assert.equal(reads, 1); assert.equal(controller.getSnapshot().loading, false); assert.doesNotMatch(controller.getSnapshot().error, /PRIVATE/); controller.dispose();
});
test("default clock and unauthenticated panel have safe initial states", async () => {
  const controller = new MediaActivityController(() => {}, async () => clone()); await controller.start(); assert.equal(controller.getSnapshot().stale, true); controller.dispose();
  const panel = props => renderToStaticMarkup(React.createElement(MediaActivityPanel, { principal: "test", grantRevision: 1, expired() {}, ...props }));
  assert.equal(panel({ principal: null }), ""); assert.equal(panel({ grantRevision: Number.NaN }), "");
  assert.match(panel({}), /Media activity is unavailable/); assert.doesNotMatch(panel({}), /Example title/);
});
