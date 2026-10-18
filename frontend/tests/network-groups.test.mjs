import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { decodeNetworkInventory, readNetworkInventory, networkInventoryExpired, NetworkReadError, NetworkInventoryController, NetworkGroups, NetworkGroupsPanel, wire } from "./helpers/network-groups.mjs";

const clone = () => structuredClone(wire);
const render = props => renderToStaticMarkup(React.createElement(NetworkGroups, { now: 105, refresh() {}, ...props }));
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fakeClock() {
  let now = 105, serial = 0; const tasks = new Map();
  return { now: () => now, schedule(fn, ms) { tasks.set(++serial, { fn, at: now + ms / 1000 }); return serial; }, cancel(id) { tasks.delete(id); },
    advance(at) { now = at; for (const [id, task] of [...tasks]) if (task.at <= now) { tasks.delete(id); task.fn(); } },
    early(at) { now = at; const [id, task] = tasks.entries().next().value; tasks.delete(id); task.fn(); }, tasks };
}
function unavailable(source = "unavailable") {
  return { version: 1, generated_at: 105, source, sampled_at: source === "unavailable" ? null : 100, max_age_seconds: 90,
    observed_containers: null, independent_containers: null, unknown_mode_containers: null, groups: [], members: [] };
}
function healthy() {
  const value = clone(); value.members.splice(1, 1); value.groups[0].hinted_orphan_ids = []; value.groups[0].state = "observed_running"; value.observed_containers = 3; return value;
}

test("Rust wire preserves full identities, case paths, stopped members and one-offs", () => {
  const value = decodeNetworkInventory(clone()); assert.deepEqual(value, wire);
  const html = render({ value });
  assert.match(html, /4 containers observed/); assert.match(html, /1 configured group/);
  assert.match(html, /\/opt\/stacks\/Media/); assert.match(html, /sonarr-oneoff/); assert.match(html, /One-off/); assert.match(html, /Not running/);
  assert.match(html, new RegExp("a".repeat(64))); assert.match(html, /No configured health check/);
});
test("orphan hints remain separate from actual member associations", () => {
  const html = render({ value: decodeNetworkInventory(clone()) });
  assert.match(html, /Unverified orphan hints/); assert.match(html, /Suggested provider only/); assert.match(html, /Provider missing/);
  assert.match(html, /Unverified hint: vpn/); assert.doesNotMatch(html, /VPN safe\b|VPN verified\b|>Probe<|>Recreate<|>Start<|>Stop</);
});
test("a running provider with no health check is not described as a verified VPN", () => {
  const html = render({ value: decodeNetworkInventory(healthy()) });
  assert.match(html, /Provider running/); assert.match(html, /No configured health check/);
  assert.match(html, /do not confirm that traffic uses the provider or a VPN/);
  assert.doesNotMatch(html, /Healthy group|VPN connected|Leak free/);
});
test("provider health and topology distinctions have explicit labels", () => {
  for (const [state, running, health, label] of [
    ["provider_stopped", false, "not_configured", "Provider stopped"],
    ["provider_unhealthy", true, "unhealthy", "Provider health check failed"],
    ["provider_starting", true, "starting", "Provider starting"],
    ["health_unknown", true, "unknown", "Provider health unknown"],
    ["provider_network_unknown", true, "healthy", "Provider network mode unknown"],
  ]) {
    const value = healthy(); value.groups[0].state = state; value.groups[0].provider.running = running; value.groups[0].provider.health = health;
    assert.match(render({ value: decodeNetworkInventory(value) }), new RegExp(label));
  }
});
test("missing, partial and mismatched observations never display zero groups as health", () => {
  for (const source of ["unavailable", "incomplete", "engine_mismatch"]) {
    const value = decodeNetworkInventory(unavailable(source)); const html = render({ value });
    assert.match(html, /Current network groups and counts are unknown/);
    assert.doesNotMatch(html, /0 configured groups|No containers were present/);
  }
  assert.match(render({ value: null }), /Current network groups and counts are unknown/);
});
test("only a fresh complete empty cohort displays zero", () => {
  const value = { ...unavailable(), source: "fresh", sampled_at: 100, observed_containers: 0, independent_containers: 0, unknown_mode_containers: 0 };
  const html = render({ value: decodeNetworkInventory(value) }); assert.match(html, /0 containers observed/); assert.match(html, /No containers were present/);
});
test("expiry and clock rollback suppress all retained names and health rows", () => {
  for (const props of [{ stale: true }, { now: 191 }, { now: 104 }, { now: NaN }]) {
    const html = render({ value: decodeNetworkInventory(clone()), ...props });
    assert.match(html, /Current network groups and counts are unknown/);
    assert.doesNotMatch(html, /sonarr|transmission|4 containers observed|Unverified orphan hints/);
  }
});
test("principal and grant guards render no unauthenticated panel", () => {
  for (const [principal, grantRevision] of [[null, 1], ["", 1], ["viewer", 0], ["viewer", NaN], ["viewer", 1.5]]) {
    assert.equal(renderToStaticMarkup(React.createElement(NetworkGroupsPanel, { principal, grantRevision, expired() {} })), "");
  }
  assert.match(renderToStaticMarkup(React.createElement(NetworkGroupsPanel, { principal: "viewer", grantRevision: 1, expired() {} })), /Current network groups and counts are unknown/);
});
test("selected-field construction drops unknown private payloads", () => {
  const value = clone(); value.raw = { token: "fixture-do-not-retain" }; value.groups[0].provider.environment = ["fixture-do-not-retain"]; value.members[0].container.compose.project.secret = "fixture-do-not-retain";
  const result = decodeNetworkInventory(value); assert.equal(JSON.stringify(result).includes("fixture-do-not-retain"), false);
  value.groups[0].provider.name = "changed-after-admission"; assert.equal(result.groups[0].provider.name, "vpn");
});
test("wire admission rejects age, source, count and version contradictions", () => {
  for (const change of [v => { v.version = 2; }, v => { v.max_age_seconds = 91; }, v => { v.sampled_at = null; },
    v => { v.generated_at = 99; }, v => { v.generated_at = 191; }, v => { v.observed_containers = 0; },
    v => { v.independent_containers = null; }, v => { v.unknown_mode_containers = 17; }, v => { v.observed_containers = 1.5; },
    v => { v.source = "incomplete"; }, v => { v.source = "stale"; }, v => { v.source = "future"; }, v => { v.source = "unavailable"; }]) {
    const value = clone(); change(value); assert.throws(() => decodeNetworkInventory(value), NetworkReadError);
  }
});
test("wire admission rejects duplicate, conflicting and abbreviated identities", () => {
  for (const change of [v => { v.groups.push(v.groups[0]); }, v => { v.members.push(v.members[0]); },
    v => { v.members[0].container.id = "bbbbbbbbbbbb"; }, v => { v.members[1].container.name = v.members[0].container.name; },
    v => { v.groups[0].provider.id = v.members[0].container.id; }, v => { v.groups = Array(17).fill(v.groups[0]); },
    v => { v.members[0].container.name = "<script>fixture</script>"; }, v => { v.members[0].container.health = "safe"; },
    v => { v.members[0].container.compose.project.config_files = ["relative.yaml"]; }, v => { v.members[0].container.compose.project.config_files.push(v.members[0].container.compose.project.config_files[0]); },
    v => { v.members[0].container.compose.project.working_directory = "/opt/../Media"; }]) {
    const value = clone(); change(value); assert.throws(() => decodeNetworkInventory(value), NetworkReadError);
  }
});
test("wire admission rejects broken provider/member/hint associations", () => {
  for (const change of [v => { v.groups[0].member_ids = []; }, v => { v.groups[0].hinted_orphan_ids = []; },
    v => { v.members[0].provider_id = null; }, v => { v.members[0].provider_id = "f".repeat(64); },
    v => { v.members[1].provider_id = "a".repeat(64); }, v => { v.members[0].hinted_provider_id = "a".repeat(64); },
    v => { v.members[1].dependency_hint = null; }, v => { v.members[1].reference_kind = "service"; },
    v => { v.groups[0].state = "observed_running"; }, v => { v.members.reverse(); }, v => { v.groups[0].member_ids.reverse(); }]) {
    const value = clone(); change(value); assert.throws(() => decodeNetworkInventory(value), NetworkReadError);
  }
});
test("contradictory running and configured-health labels are refused", () => {
  for (const [state, running, health] of [["observed_running", false, "healthy"], ["observed_running", true, "unknown"], ["provider_stopped", true, "healthy"], ["provider_unhealthy", true, "healthy"], ["provider_starting", true, "unknown"], ["health_unknown", true, "healthy"]]) {
    const value = healthy(); Object.assign(value.groups[0], { state }); Object.assign(value.groups[0].provider, { running, health });
    assert.throws(() => decodeNetworkInventory(value), NetworkReadError);
  }
});
test("inclusive original-sample deadline and invalid clocks stay explicit", () => {
  assert.equal(networkInventoryExpired(wire, 190.99), false); assert.equal(networkInventoryExpired(wire, 191), true);
  for (const at of [104, -1, NaN, Infinity, 253402300800]) assert.equal(networkInventoryExpired(wire, at), true);
  assert.equal(networkInventoryExpired(unavailable(), 105), true);
});
test("client performs one fixed same-origin cache-only GET", async () => {
  const calls = []; const result = await readNetworkInventory(new AbortController().signal, async (url, init) => { calls.push({ url, init }); return Response.json(wire); });
  assert.deepEqual(result, wire); assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/v1/deployment/networks"); assert.equal(calls[0].init.method, "GET");
  assert.equal(calls[0].init.credentials, "same-origin"); assert.equal(calls[0].init.cache, "no-store");
  assert.deepEqual(calls[0].init.headers, { Accept: "application/json" });
});
test("HTTP access status wins over malformed, oversized and private error bodies", async () => {
  for (const status of [401, 403]) for (const body of ["not-json", "x".repeat(70_000), JSON.stringify({ code: "unavailable", message: "fixture-private" })]) {
    await assert.rejects(readNetworkInventory(new AbortController().signal, async () => new Response(body, { status })), error => error instanceof NetworkReadError && error.status === status && !error.message.includes("fixture-private"));
  }
  await assert.rejects(readNetworkInventory(new AbortController().signal, async () => new Response("fixture-private", { status: 503 })), error => error.status === 503 && !error.message.includes("fixture-private"));
});
test("streamed admission accepts the byte ceiling and cancels overflow", async () => {
  const encoded = new TextEncoder().encode(JSON.stringify(wire)); const exact = new Uint8Array(65_536).fill(32); exact.set(encoded);
  assert.deepEqual(await readNetworkInventory(new AbortController().signal, async () => new Response(exact)), wire);
  let cancelled = false;
  const stream = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array(65_537)); }, cancel() { cancelled = true; } });
  await assert.rejects(readNetworkInventory(new AbortController().signal, async () => new Response(stream)), NetworkReadError); assert.equal(cancelled, true);
});
test("invalid UTF-8, JSON and missing response bodies fail closed", async () => {
  for (const response of [new Response(new Uint8Array([255])), new Response("{"), new Response(null)]) {
    await assert.rejects(readNetworkInventory(new AbortController().signal, async () => response), NetworkReadError);
  }
});
test("a pre-aborted read performs no fetch", async () => {
  const controller = new AbortController(); controller.abort(); let calls = 0;
  await assert.rejects(readNetworkInventory(controller.signal, async () => { calls++; return Response.json(wire); }), NetworkReadError);
  assert.equal(calls, 0);
});
test("aborting a streaming body cancels it without accepting a valid prefix", async () => {
  const controller = new AbortController(); let cancelled = false;
  const stream = new ReadableStream({ start(reader) { reader.enqueue(new TextEncoder().encode(JSON.stringify(wire))); }, cancel() { cancelled = true; } });
  const promise = readNetworkInventory(controller.signal, async () => new Response(stream));
  await Promise.resolve(); await Promise.resolve(); controller.abort();
  await assert.rejects(promise, NetworkReadError); assert.equal(cancelled, true);
});
test("the read deadline also cancels a stalled response body", async () => {
  const savedSet = globalThis.setTimeout, savedClear = globalThis.clearTimeout; let deadline, cancelled = false, delay;
  globalThis.setTimeout = (fn, ms) => { deadline = fn; delay = ms; return 1; }; globalThis.clearTimeout = () => {};
  try {
    const stream = new ReadableStream({ cancel() { cancelled = true; } });
    const promise = readNetworkInventory(new AbortController().signal, async () => new Response(stream));
    await Promise.resolve(); await Promise.resolve(); assert.equal(delay, 12_000); deadline();
    await assert.rejects(promise, NetworkReadError); assert.equal(cancelled, true);
  } finally { globalThis.setTimeout = savedSet; globalThis.clearTimeout = savedClear; }
});
test("controller coalesces concurrent refreshes without polling", async () => {
  const wait = deferred(); const clock = fakeClock(); let calls = 0;
  const controller = new NetworkInventoryController(() => {}, () => { calls++; return wait.promise; }, clock);
  const first = controller.start(); assert.equal(controller.refresh(), first); await Promise.resolve(); assert.equal(calls, 1);
  wait.resolve(decodeNetworkInventory(clone())); await first; assert.equal(controller.getSnapshot().stale, false); assert.equal(clock.tasks.size, 1); controller.dispose();
});
test("early timers rearm the original deadline and expiry clears private rows", async () => {
  const clock = fakeClock(); const controller = new NetworkInventoryController(() => {}, async () => decodeNetworkInventory(clone()), clock);
  await controller.start(); clock.early(106); assert.equal(clock.tasks.size, 1);
  clock.advance(190.99); assert.notEqual(controller.getSnapshot().value, null);
  clock.advance(191); assert.equal(controller.getSnapshot().value, null); assert.equal(controller.getSnapshot().stale, true); assert.equal(clock.tasks.size, 0); controller.dispose();
});
test("an already expired cache response cannot restore private rows", async () => {
  const clock = fakeClock(); clock.advance(191);
  const controller = new NetworkInventoryController(() => {}, async () => decodeNetworkInventory(clone()), clock);
  await controller.start(); assert.equal(controller.getSnapshot().value, null); assert.match(controller.getSnapshot().error, /expired/); assert.equal(clock.tasks.size, 0); controller.dispose();
});
test("clock rollback clears previous rows before an in-flight refresh finishes", async () => {
  const clock = fakeClock(); const wait = deferred(); let calls = 0;
  const controller = new NetworkInventoryController(() => {}, () => ++calls === 1 ? Promise.resolve(decodeNetworkInventory(clone())) : wait.promise, clock);
  await controller.start(); clock.advance(99); const pending = controller.refresh(); assert.equal(controller.getSnapshot().value, null);
  wait.resolve(decodeNetworkInventory(clone())); await pending; assert.equal(controller.getSnapshot().value, null); controller.dispose();
});
test("read failures clear retained data and never expose raw transport errors", async () => {
  let calls = 0; const controller = new NetworkInventoryController(() => {}, async () => { if (++calls === 1) return decodeNetworkInventory(clone()); throw new Error("fixture-private-token"); }, fakeClock());
  await controller.start(); await controller.refresh(); assert.equal(controller.getSnapshot().value, null); assert.doesNotMatch(controller.getSnapshot().error, /fixture-private-token/); controller.dispose();
});
test("access loss clears data, stops refreshes and expires the shell only for 401", async () => {
  for (const status of [401, 403]) {
    let calls = 0, expired = 0; const clock = fakeClock();
    const controller = new NetworkInventoryController(() => { expired++; }, async () => { if (++calls === 1) return decodeNetworkInventory(clone()); throw new NetworkReadError("fixture-private", status); }, clock);
    await controller.start(); await controller.refresh(); const state = controller.getSnapshot();
    assert.equal(state.value, null); assert.equal(state.expired, status === 401); assert.equal(state.denied, status === 403); assert.equal(expired, status === 401 ? 1 : 0);
    await controller.refresh(); assert.equal(calls, 2); assert.equal(clock.tasks.size, 0); assert.doesNotMatch(state.error, /fixture-private/); controller.dispose();
  }
});
test("disposing before deferred startup launches no read", async () => {
  let calls = 0; const controller = new NetworkInventoryController(() => {}, async () => { calls++; return clone(); }, fakeClock());
  const pending = controller.start(); controller.dispose(); await pending; assert.equal(calls, 0); assert.equal(controller.getSnapshot().value, null);
});
test("disposal aborts old work and ignores late results across a new grant owner", async () => {
  const oldWait = deferred(); let oldSignal; const clock = fakeClock();
  const old = new NetworkInventoryController(() => {}, signal => { oldSignal = signal; return oldWait.promise; }, clock);
  const pending = old.start(); await Promise.resolve(); old.dispose(); assert.equal(oldSignal.aborted, true);
  const next = new NetworkInventoryController(() => {}, async () => unavailable(), fakeClock()); await next.start();
  oldWait.resolve(decodeNetworkInventory(clone())); await pending;
  assert.equal(old.getSnapshot().value, null); assert.equal(next.getSnapshot().value.source, "unavailable"); old.dispose(); next.dispose();
});
test("synchronous failures still coalesce and a stopped controller can restart", async () => {
  let calls = 0; const controller = new NetworkInventoryController(() => {}, () => { calls++; throw new Error("fixture-only"); }, fakeClock());
  const pending = controller.start(); assert.equal(controller.refresh(), pending); await pending; assert.equal(calls, 1);
  controller.dispose(); await controller.start(); assert.equal(calls, 2); controller.dispose();
});
test("self and cyclic references remain visible as unsupported topology", () => {
  for (const cycle of [false, true]) {
    const a = structuredClone(wire.groups[0].provider), b = structuredClone(wire.members[0].container);
    const containers = cycle ? [a, b] : [a];
    const members = containers.map((container, index) => ({ container, reference_kind: "container", target: containers[(index + 1) % containers.length].id,
      state: cycle ? "cyclic_provider" : "self_reference", provider_id: containers[(index + 1) % containers.length].id, hinted_provider_id: null, dependency_hint: null }));
    const value = { ...healthy(), observed_containers: containers.length, independent_containers: 0, members,
      groups: containers.map(provider => ({ provider, state: "unsupported_topology", member_ids: members.filter(m => m.provider_id === provider.id).map(m => m.container.id), hinted_orphan_ids: [] })) };
    const html = render({ value: decodeNetworkInventory(value) });
    assert.match(html, /Unsupported reference topology/); assert.match(html, cycle ? /Reference cycle/ : /Self reference/);
    assert.doesNotMatch(html, /Provider running/);
  }
});
test("unselected dependency hints never substitute for a missing provider", () => {
  const value = clone(); value.members[1].hinted_provider_id = null; value.groups[0].hinted_orphan_ids = []; value.groups[0].state = "observed_running";
  value.observed_containers = 5; value.independent_containers = 2;
  const html = render({ value: decodeNetworkInventory(value) });
  assert.match(html, /Provider missing/); assert.match(html, /Dependency hint \(unverified\): vpn/); assert.doesNotMatch(html, /Suggested provider only/);
});
test("valid stale and future reports keep empty rows and explicit unknown state", () => {
  const stale = unavailable("stale"); stale.sampled_at = 14;
  const future = unavailable("future"); future.sampled_at = 106;
  for (const value of [stale, future]) {
    const html = render({ value: decodeNetworkInventory(value) }); assert.match(html, /Current network groups and counts are unknown/); assert.doesNotMatch(html, /0 configured groups/);
  }
});
test("error text is escaped and access-denied views offer no refresh action", () => {
  const html = render({ value: null, error: "<script>fixture-only</script>", refresh: undefined });
  assert.match(html, /&lt;script&gt;fixture-only/); assert.doesNotMatch(html, /<script>|<button/);
});
