import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, AutomationController, AutomationReadStatus, AutomationView, AutomationPanel, NotificationsView } from "./helpers/automation.mjs";

const page = (items = [], next = null, partial = false) => ({ items, next, partial });
const row = id => ({ schedule: { id, revision: 1, paused: false, settled_through: null,
  definition: { operation: "observe_health", anchor: 1000, interval_seconds: 60, grace_seconds: 30, missed: "coalesce_latest", window: null } }, status: "awaiting_producer", last_occurrence: null });
const failure = code => new ApiError({ code, message: `Read failed: ${code}`, retry: false, audit_id: "case-1" });
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function reader(handler) { return Object.fromEntries(["schedules", "alerts", "notifications"].map(kind => [kind, (after, signal) => handler(kind, after, signal)])); }
const render = (Component, props) => renderToStaticMarkup(React.createElement(Component, props));

test("three independent pages retain partial observations", async () => {
  const calls = [];
  const controller = new AutomationController(() => assert.fail("unexpected expiry"), reader(async (kind, after, signal) => {
    calls.push({ kind, after, signal }); return page(kind === "schedules" ? [row("health")] : [], null, kind === "alerts");
  }));
  await controller.start(); const state = controller.getSnapshot();
  assert.equal(calls.length, 3); assert.equal(state.alerts.page.partial, true);
  assert.equal(state.schedules.page.items[0].schedule.id, "health");
  assert.equal(state.schedules.stale, false); assert.equal(state.notifications.loading, false);
  controller.dispose();
});

test("failed refresh preserves visibly stale data until a committed read succeeds", async () => {
  let broken = false;
  const controller = new AutomationController(() => {}, reader(async kind => {
    if (broken && kind === "schedules") throw failure("unavailable");
    return page(kind === "schedules" ? [row("health")] : []);
  }));
  await controller.start(); broken = true; await controller.refresh();
  let state = controller.getSnapshot();
  assert.equal(state.schedules.page.items[0].schedule.id, "health"); assert.equal(state.schedules.stale, true);
  assert.equal(state.alerts.stale, false);
  assert.match(render(AutomationReadStatus, { state }), /Previously read schedules may be out of date/);
  const html = render(AutomationView, { schedules: state.schedules.page, alerts: state.alerts.page, canManage: true, disabled: true, create() {}, pause() {}, moreSchedules() {}, moreAlerts() {} });
  assert.match(html, /disabled=""[^>]*>Pause health/); assert.match(html, /disabled=""[^>]*>Create observation schedule/);
  broken = false; await controller.refresh(); state = controller.getSnapshot();
  assert.equal(state.schedules.stale, false); assert.equal(state.schedules.error, null); controller.dispose();
});

test("initial unavailable data is distinct from an empty successful page", async () => {
  const controller = new AutomationController(() => {}, reader(async () => { throw failure("unavailable"); }));
  await controller.start(); const state = controller.getSnapshot();
  assert.equal(state.schedules.page, null);
  const html = render(AutomationView, { schedules: null, alerts: null, canManage: false, moreSchedules() {}, moreAlerts() {} });
  assert.match(html, /Schedule data is unavailable/); assert.match(html, /Alert data is unavailable/);
  assert.doesNotMatch(html, /No schedules|No active alerts/);
  const notes = render(NotificationsView, { value: null, canAcknowledge: false, more() {} });
  assert.match(notes, /Notification data is unavailable/); assert.doesNotMatch(notes, /No notifications/);
  controller.dispose();
});

test("expired session clears all private pages, cancels peers and signals once", async () => {
  const pending = []; let expired = 0;
  const controller = new AutomationController(() => expired++, reader((kind, after, signal) => {
    const task = deferred(); pending.push({ ...task, kind, signal }); return task.promise;
  }));
  const loading = controller.start();
  pending[0].reject(failure("unauthenticated")); await Promise.resolve(); await Promise.resolve();
  assert.equal(expired, 1); assert.ok(pending.every(task => task.signal.aborted));
  pending[1].resolve(page([{ key: "private" }])); pending[2].reject(failure("unauthenticated")); await loading;
  assert.equal(controller.getSnapshot().expired, true);
  for (const kind of ["schedules", "alerts", "notifications"]) assert.equal(controller.getSnapshot()[kind].page, null);
  await controller.refresh(); assert.equal(pending.length, 3); assert.equal(expired, 1); controller.dispose();
});

test("revoked page grant clears that page rather than retaining forbidden content", async () => {
  let denied = false;
  const controller = new AutomationController(() => assert.fail("session still valid"), reader(async kind => {
    if (denied && kind === "schedules") throw failure("forbidden");
    return page(kind === "schedules" ? [row("private")] : []);
  }));
  await controller.start(); denied = true; await controller.refresh();
  assert.equal(controller.getSnapshot().schedules.page, null);
  assert.notEqual(controller.getSnapshot().alerts.page, null); controller.dispose();
});

test("abandoned older page cannot overwrite a newer page even if transport ignores abort", async () => {
  let defer = false; const requests = [];
  const controller = new AutomationController(() => {}, reader(async (kind, after, signal) => {
    if (!defer || kind !== "schedules") return page(kind === "schedules" ? [row("original")] : [], kind === "schedules" ? "p2" : null);
    const task = deferred(); requests.push({ ...task, after, signal }); return task.promise;
  }));
  await controller.start(); defer = true;
  const next = controller.next("schedules"); const first = controller.first("schedules");
  assert.equal(requests[0].signal.aborted, true);
  requests[1].resolve(page([row("fresh-first")], "p2")); await first;
  requests[0].resolve(page([row("old-second")], "p3")); await next;
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "fresh-first");
  assert.equal(controller.getSnapshot().schedules.after, undefined); controller.dispose();
});

test("pagination replaces a bounded page instead of accumulating history", async () => {
  const controller = new AutomationController(() => {}, reader(async (kind, after) => page(
    kind === "schedules" ? Array.from({ length: 32 }, (_, i) => row(`${after ?? "first"}-${i}`)) : [], kind === "schedules" ? "next" : null)));
  await controller.start(); await controller.next("schedules");
  assert.equal(controller.getSnapshot().schedules.page.items.length, 32);
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "next-0"); controller.dispose();
});

test("repeated reconnect triggers coalesce into one pending refresh batch", async () => {
  const pending = []; let calls = 0;
  const controller = new AutomationController(() => {}, reader(() => {
    calls++;
    if (calls > 3) return Promise.resolve(page());
    const task = deferred(); pending.push(task); return task.promise;
  }));
  const starting = controller.start();
  for (let i = 0; i < 20; i++) void controller.refresh();
  assert.equal(calls, 3); for (const task of pending) task.resolve(page());
  await starting; assert.equal(calls, 6); controller.dispose();
});

test("cleanup and React effect restart isolate old request generations", async () => {
  const pending = []; let expired = 0;
  const controller = new AutomationController(() => expired++, reader((kind, after, signal) => {
    const task = deferred(); pending.push({ ...task, kind, signal }); return task.promise;
  }));
  const old = controller.start(); controller.dispose();
  assert.ok(pending.every(task => task.signal.aborted));
  const fresh = controller.start();
  pending[0].reject(failure("unauthenticated")); pending[1].resolve(page()); pending[2].resolve(page()); await old;
  for (const task of pending.slice(3)) task.resolve(page(task.kind === "schedules" ? [row("current")] : []));
  await fresh; assert.equal(expired, 0); assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "current"); controller.dispose();
});

test("failed mutation triggers committed reads without retrying the POST", async () => {
  let calls = 0;
  const controller = new AutomationController(() => {}, reader(async () => { calls++; return page(); }));
  await controller.start(); await controller.mutationFailed("schedules", failure("conflict"));
  assert.equal(calls, 6); assert.equal(controller.getSnapshot().schedules.error, null); controller.dispose();
});

test("mount-ready panel exposes loading and disabled creation before initial reads", () => {
  const html = render(AutomationPanel, { principal: "operator", grantRevision: 1, csrf: "csrf", canManage: true, canAcknowledge: true, expired() {} });
  assert.match(html, /Loading schedules/); assert.match(html, /Loading alerts/); assert.match(html, /Loading notifications/);
  assert.match(html, /disabled=""[^>]*>Create observation schedule/);
});

test("mutation completion invalidates earlier in-flight reads before controls can resume", async () => {
  const pending = []; let calls = 0;
  const controller = new AutomationController(() => {}, reader((kind, after, signal) => {
    calls++;
    if (calls <= 3 || calls > 6) return Promise.resolve(page(kind === "schedules" ? [row(calls <= 3 ? "before" : "after")] : []));
    const task = deferred(); pending.push({ ...task, kind, signal }); return task.promise;
  }));
  await controller.start(); const oldRead = controller.refresh();
  const afterCommit = controller.committed("schedules");
  pending[0].resolve(page([row("outdated")])); await Promise.resolve(); await Promise.resolve();
  assert.equal(pending[0].signal.aborted, true);
  assert.equal(controller.getSnapshot().schedules.stale, true);
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "before");
  pending[1].resolve(page()); pending[2].resolve(page()); await oldRead; await afterCommit;
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "after");
  assert.equal(controller.getSnapshot().schedules.stale, false); controller.dispose();
});

test("queued refresh follows the latest navigation target rather than an older cursor", async () => {
  let phase = "initial"; const pending = []; const followups = [];
  const controller = new AutomationController(() => {}, reader((kind, after, signal) => {
    if (phase === "initial") return Promise.resolve(page(kind === "schedules" ? [row("first")] : [], kind === "schedules" ? "p2" : null));
    if (phase === "followup") { followups.push({ kind, after }); return Promise.resolve(page(kind === "schedules" ? [row("second")] : [])); }
    const task = deferred(); pending.push({ ...task, kind, after, signal }); return task.promise;
  }));
  await controller.start(); phase = "pending";
  const refresh = controller.refresh(); void controller.refresh(); const next = controller.next("schedules");
  phase = "followup";
  for (const task of pending.slice(0, 3)) task.resolve(page());
  await refresh;
  pending[3].resolve(page([row("late-second")])); await next;
  assert.equal(followups.find(call => call.kind === "schedules").after, "p2");
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "second"); controller.dispose();
});

test("disposal publishes cleared private state and remains idempotent", async () => {
  const seen = [];
  const controller = new AutomationController(() => {}, reader(async kind => page(kind === "schedules" ? [row("private")] : [])));
  const unsubscribe = controller.subscribe(() => seen.push(controller.getSnapshot()));
  await controller.start(); assert.notEqual(seen.at(-1).schedules.page, null);
  controller.dispose(); assert.equal(seen.at(-1).schedules.page, null);
  const count = seen.length; controller.dispose(); assert.equal(seen.length, count); unsubscribe();
});
