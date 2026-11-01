import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, AutomationController, AutomationView, HealthEnablementController, HealthSchedules, automationClient } from "./helpers/automation.mjs";

const page = { items: [], next: null, partial: false };
const failure = code => new ApiError({ code, message: "Permission refused", retry: false, audit_id: "permission-test" });
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }

test("schedule mode never reads or mutates unregistered alert/notification routes", async () => {
  const reads = []; let sends = 0;
  const reader = Object.fromEntries(["schedules", "alerts", "notifications"].map(kind => [kind, async () => { reads.push(kind); return page; }]));
  const controller = new AutomationController(() => assert.fail("valid owner"), reader, "schedules");
  await controller.start(); await controller.first("alerts"); await controller.first("notifications");
  await controller.mutate("notifications", async () => { sends++; }); await controller.refresh();
  assert.deepEqual(reads, ["schedules", "schedules"]); assert.equal(sends, 0); controller.dispose();
});

test("fixed enablement sends only password to its registered POST with CSRF", async () => {
  const original = globalThis.fetch; const calls = [];
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response(JSON.stringify({ enabled: true, reauthenticate: true })); };
  try {
    assert.deepEqual(await automationClient.enableHealth("synthetic-password", "csrf"), { enabled: true, reauthenticate: true });
    assert.equal(calls.length, 1); assert.equal(calls[0].url, "/api/v1/auth/capabilities/observe-health/enable");
    assert.equal(calls[0].init.method, "POST"); assert.equal(calls[0].init.headers["X-CSRF-Token"], "csrf");
    assert.deepEqual(JSON.parse(calls[0].init.body), { password: "synthetic-password" });
    assert.equal(calls[0].init.cache, "no-store"); assert.equal(calls[0].init.credentials, "same-origin");
  } finally { globalThis.fetch = original; }
});

test("malformed enablement, overlong credentials and status errors cannot confirm permission", async () => {
  const original = globalThis.fetch; let requests = 0;
  try {
    globalThis.fetch = async () => { requests++; return new Response("{}"); };
    for (const password of ["", "🦉".repeat(257)]) await assert.rejects(automationClient.enableHealth(password, "csrf"));
    assert.equal(requests, 0);
    for (const response of [
      new Response("<html>bad</html>", { status: 401 }), new Response("<html>bad</html>", { status: 403 }),
      new Response(JSON.stringify({ enabled: false, reauthenticate: false })),
      new Response(JSON.stringify({ enabled: true, reauthenticate: "yes" })),
      new Response(JSON.stringify({ enabled: true, reauthenticate: false, subject: "someone-else" })),
      new Response("x".repeat(65537)),
    ]) {
      globalThis.fetch = async () => response;
      await assert.rejects(automationClient.enableHealth("synthetic-password", "csrf"), error => error instanceof ApiError && (response.status === 401 ? error.detail.code === "unauthenticated" : response.status === 403 ? error.detail.code === "forbidden" : error.detail.code === "unavailable"));
    }
  } finally { globalThis.fetch = original; }
});

test("same-frame confirmation and confirmed-state reentry send one POST", async () => {
  const response = deferred(); let sends = 0; const settled = [];
  const controller = new HealthEnablementController(value => settled.push(value), () => {}, async () => { sends++; return response.promise; });
  const first = controller.confirm("synthetic", "csrf"); const duplicate = controller.confirm("synthetic", "csrf");
  assert.equal(sends, 1); assert.equal(controller.getSnapshot().pending, true);
  response.resolve({ enabled: true, reauthenticate: false }); await Promise.all([first, duplicate]);
  await controller.confirm("synthetic", "csrf"); assert.equal(sends, 1); assert.equal(settled.length, 1);
  assert.deepEqual(controller.getSnapshot(), { pending: false, confirmed: true, error: null }); controller.dispose();
});

test("synchronous owner disposal before confirmation dispatch sends no POST", async () => {
  let sends = 0; const controller = new HealthEnablementController(() => assert.fail("abandoned"), () => assert.fail("abandoned"), async () => { sends++; });
  controller.subscribe(() => { if (controller.getSnapshot().pending) controller.dispose(); });
  await controller.confirm("synthetic", "csrf"); assert.equal(sends, 0);
});

test("abandoned replies cannot settle or expire a restarted owner", async () => {
  for (const rejected of [false, true]) {
    const response = deferred(); let expiry = 0, settled = 0, signal;
    const controller = new HealthEnablementController(() => settled++, () => expiry++, async (_password, _csrf, supplied) => { signal = supplied; return response.promise; });
    const old = controller.confirm("synthetic", "old"); controller.dispose(); assert.equal(signal.aborted, true); controller.start();
    if (rejected) response.reject(failure("unauthenticated")); else response.resolve({ enabled: true, reauthenticate: true });
    await old; assert.equal(expiry, 0); assert.equal(settled, 0); assert.equal(controller.getSnapshot().confirmed, false); controller.dispose();
  }
});

test("unconfirmed reply is visible without automatic retry or false confirmation", async () => {
  let sends = 0; const results = [];
  const controller = new HealthEnablementController(value => results.push(value), () => {}, async () => { sends++; throw new TypeError("lost reply"); });
  await controller.confirm("synthetic", "csrf");
  assert.equal(sends, 1); assert.equal(controller.getSnapshot().confirmed, false);
  assert.match(controller.getSnapshot().error, /unconfirmed/); assert.deepEqual(results, [undefined]);
  assert.equal(JSON.stringify(controller.getSnapshot()).includes("synthetic"), false); controller.dispose();
});

test("current unauthorized confirmation expires once and retains no credential state", async () => {
  let expiry = 0; const controller = new HealthEnablementController(() => assert.fail("not confirmed"), () => expiry++, async () => { throw failure("unauthenticated"); });
  await controller.confirm("synthetic", "csrf"); await controller.confirm("synthetic", "csrf");
  assert.equal(expiry, 1); assert.deepEqual(controller.getSnapshot(), { pending: false, confirmed: false, error: null });
});

test("schedule-only rendering retains honest producer status without unsupported panels", () => {
  const html = renderToStaticMarkup(React.createElement(AutomationView, { schedules: page, alerts: null, canManage: true, showAlerts: false, moreSchedules() {}, moreAlerts() {} }));
  assert.match(html, /Health schedules/); assert.doesNotMatch(html, /Active alerts|Alert data is unavailable/);
  const panel = renderToStaticMarkup(React.createElement(HealthSchedules, { principal: "owner", grantRevision: 1, csrf: "csrf", expired() {}, reauthenticate() {} }));
  assert.match(panel, /Each schedule shows whether automatic container checks are enabled/); assert.match(panel, /Health coverage remains partial/); assert.match(panel, /Current password/);
  assert.doesNotMatch(panel, /Loading alerts|Loading notifications|No notifications/);
});
