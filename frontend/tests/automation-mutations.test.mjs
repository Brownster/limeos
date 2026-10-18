import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, AutomationController, AutomationPanel, AutomationView, NotificationsView, automationClient } from "./helpers/automation.mjs";

const definition = { operation: "observe_health", anchor: 1000, interval_seconds: 60, grace_seconds: 30, missed: "coalesce_latest", window: null };
const row = { schedule: { id: "health", revision: 4, paused: false, settled_through: null, definition }, status: "active", last_occurrence: null };
const note = { id: "note-1", revision: 2, event: { key: "host:system", generation: 1, event: "incident", kind: "host", severity: "warning", summary: "Host alert", at: 1000 }, channel: "in_app", delivery: { state: "delivered", at: 1001 }, attempts: 1, acknowledged_at: null };
const page = (items = []) => ({ items, next: null, partial: false });
const failure = code => new ApiError({ code, message: `Mutation failed: ${code}`, retry: false, audit_id: "mutation-1" });
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function reader(handler = async kind => page(kind === "schedules" ? [row] : kind === "notifications" ? [note] : [])) {
  return Object.fromEntries(["schedules", "alerts", "notifications"].map(kind => [kind, (after, signal) => handler(kind, after, signal)]));
}
const render = (Component, props) => renderToStaticMarkup(React.createElement(Component, props));

test("same-render duplicate mutations synchronously admit only one POST", async () => {
  const controller = new AutomationController(() => {}, reader()); await controller.start();
  const response = deferred(); let sends = 0;
  const action = () => { sends++; return response.promise; };
  const first = controller.mutate("schedules", action);
  const second = controller.mutate("schedules", action);
  assert.equal(sends, 1); assert.equal(controller.getSnapshot().mutations.schedules.pending, true);
  response.resolve(row); await Promise.all([first, second]);
  assert.equal(controller.getSnapshot().mutations.schedules.pending, false); controller.dispose();
});

test("schedule and notification writes have separate bounded admission", async () => {
  const controller = new AutomationController(() => {}, reader()); await controller.start();
  const response = deferred(); let sends = 0;
  const action = () => { sends++; return response.promise; };
  const pending = [controller.mutate("schedules", action), controller.mutate("notifications", action), controller.mutate("notifications", action)];
  assert.equal(sends, 2); response.resolve({}); await Promise.all(pending); controller.dispose();
});

test("pending-state subscribers cannot reenter mutation admission", async () => {
  const controller = new AutomationController(() => {}, reader()); await controller.start();
  const response = deferred(); let sends = 0, reentered = false;
  const unsubscribe = controller.subscribe(() => {
    if (controller.getSnapshot().mutations.schedules.pending && !reentered) {
      reentered = true; void controller.mutate("schedules", async () => { sends++; });
    }
  });
  const pending = controller.mutate("schedules", () => { sends++; return response.promise; });
  assert.equal(reentered, true); assert.equal(sends, 1); response.resolve(row); await pending;
  unsubscribe(); controller.dispose();
});

test("synchronous owner disposal before dispatch sends no POST", async () => {
  const controller = new AutomationController(() => {}, reader()); await controller.start();
  const unsubscribe = controller.subscribe(() => { if (controller.getSnapshot().mutations.schedules.pending) controller.dispose(); });
  let sends = 0; await controller.mutate("schedules", async () => { sends++; });
  assert.equal(sends, 0); assert.equal(controller.getSnapshot().schedules.page, null); unsubscribe();
});

test("abandoned failed mutation cannot expire a restarted owner", async () => {
  let expiries = 0; const controller = new AutomationController(() => expiries++, reader()); await controller.start();
  const response = deferred(); let signal;
  const old = controller.mutate("schedules", supplied => { signal = supplied; return response.promise; });
  controller.dispose(); assert.equal(signal.aborted, true); await controller.start();
  response.reject(failure("unauthenticated")); await old;
  assert.equal(expiries, 0); assert.equal(controller.getSnapshot().expired, false);
  assert.equal(controller.getSnapshot().mutations.schedules.error, null);
  assert.equal(controller.getSnapshot().schedules.page.items[0].schedule.id, "health"); controller.dispose();
});

test("ignored abort and late success cannot refresh or clear a newer mutation", async () => {
  let reads = 0; const controller = new AutomationController(() => {}, reader(async () => { reads++; return page(); })); await controller.start();
  const abandoned = deferred(); const current = deferred();
  const old = controller.mutate("schedules", () => abandoned.promise); controller.dispose(); await controller.start();
  const fresh = controller.mutate("schedules", () => current.promise);
  abandoned.resolve(row); await old;
  assert.equal(reads, 6); assert.equal(controller.getSnapshot().mutations.schedules.pending, true);
  current.resolve(row); await fresh; assert.equal(controller.getSnapshot().mutations.schedules.pending, false); controller.dispose();
});

test("mutation 401 clears private pages, cancels peer writes and signals once", async () => {
  let expiries = 0; const controller = new AutomationController(() => expiries++, reader()); await controller.start();
  const response = deferred(); let peerSignal;
  const peer = controller.mutate("notifications", signal => { peerSignal = signal; return response.promise; });
  await controller.mutate("schedules", async () => { throw failure("unauthenticated"); });
  assert.equal(expiries, 1); assert.equal(peerSignal.aborted, true);
  for (const kind of ["schedules", "alerts", "notifications"]) assert.equal(controller.getSnapshot()[kind].page, null);
  response.reject(failure("unauthenticated")); await peer; assert.equal(expiries, 1); controller.dispose();
});

test("forbidden and expired writes suspend controls across successful reads", async () => {
  for (const code of ["forbidden", "expired"]) {
    const controller = new AutomationController(() => assert.fail("session still valid"), reader()); await controller.start();
    let cleared = false; const unsubscribe = controller.subscribe(() => {
      if (controller.getSnapshot().mutations.notifications.denied && controller.getSnapshot().notifications.page === null) cleared = true;
    });
    let sends = 0; await controller.mutate("notifications", async () => { sends++; throw failure(code); });
    await controller.refresh(); await controller.mutate("notifications", async () => { sends++; });
    assert.equal(cleared, true); assert.equal(sends, 1); assert.equal(controller.getSnapshot().mutations.notifications.denied, true);
    assert.match(controller.getSnapshot().mutations.notifications.error, new RegExp(code)); unsubscribe(); controller.dispose();
  }
});

test("revision conflict feedback survives a successful committed read", async () => {
  const controller = new AutomationController(() => {}, reader()); await controller.start();
  await controller.mutate("schedules", async () => { throw failure("conflict"); }); await controller.refresh();
  assert.equal(controller.getSnapshot().schedules.error, null);
  assert.match(controller.getSnapshot().mutations.schedules.error, /conflict.*reference mutation-1/); controller.dispose();
});

test("lost replies remain unconfirmed through refresh without automatic POST retry", async () => {
  for (const error of [new TypeError("network lost"), failure("unavailable"), failure("state_not_durable")]) {
    const controller = new AutomationController(() => {}, reader()); await controller.start();
    let sends = 0; await controller.mutate("schedules", async () => { sends++; throw error; });
    await controller.refresh(); await controller.refresh();
    assert.equal(sends, 1); assert.match(controller.getSnapshot().mutations.schedules.error, /outcome is unconfirmed/);
    assert.equal(controller.getSnapshot().schedules.stale, false); controller.dispose();
  }
});

test("initial, loading, failed and disposed pages cannot dispatch stale callbacks", async () => {
  const response = deferred(); const controller = new AutomationController(() => {}, reader(() => response.promise)); let sends = 0;
  const action = async () => { sends++; };
  await controller.mutate("schedules", action); const loading = controller.start(); await controller.mutate("schedules", action);
  response.reject(failure("unavailable")); await loading; await controller.mutate("schedules", action);
  controller.dispose(); await controller.mutate("schedules", action); assert.equal(sends, 0);
});

test("refresh cannot mark a pending mutation page fresh before its result", async () => {
  let reads = 0;
  const controller = new AutomationController(() => {}, reader(async kind => { if (kind === "schedules") reads++; return page(); })); await controller.start();
  const response = deferred(); const pending = controller.mutate("schedules", () => response.promise);
  await controller.refresh(); await controller.first("schedules");
  assert.equal(reads, 1); assert.equal(controller.getSnapshot().schedules.stale, true);
  response.resolve(row); await pending;
  assert.equal(reads, 2); assert.equal(controller.getSnapshot().schedules.stale, false); controller.dispose();
});

test("all three actual mutation clients combine owner cancellation with the request deadline", async () => {
  const saved = globalThis.fetch;
  try {
    for (const kind of ["create", "pause", "acknowledge"]) {
      const abort = new AbortController(); let calls = 0, options;
      globalThis.fetch = async (_url, init) => {
        calls++; options = init;
        return new Promise((_resolve, reject) => init.signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true }));
      };
      const pending = automationClient[kind](kind === "create" ? definition : kind === "pause" ? row : note, "current-csrf", abort.signal);
      assert.equal(options.method, "POST"); assert.equal(options.headers["X-CSRF-Token"], "current-csrf");
      assert.notEqual(options.signal, abort.signal); assert.equal(options.signal.aborted, false);
      abort.abort(); await assert.rejects(pending, error => error.detail.code === "unavailable");
      assert.equal(options.signal.aborted, true); assert.equal(calls, 1);
    }
  } finally { globalThis.fetch = saved; }
});

test("owner key changes with identity, grant revision, CSRF and displayed capabilities", () => {
  const props = { principal: "operator", grantRevision: 1, csrf: "csrf-a", canManage: true, canAcknowledge: true, expired() {} };
  const original = AutomationPanel(props);
  for (const change of [{ principal: "another" }, { grantRevision: 2 }, { csrf: "csrf-b" }, { canManage: false }, { canAcknowledge: false }]) assert.notEqual(AutomationPanel({ ...props, ...change }).key, original.key);
  assert.equal(AutomationPanel({ ...props, expired() {} }).key, original.key);
});

test("pending and uncertain feedback is visible without optimistic acknowledgment", () => {
  const schedules = render(AutomationView, { schedules: page([row]), alerts: page(), canManage: true, create() {}, pause() {}, moreSchedules() {}, moreAlerts() {}, busy: true, error: "Request outcome is unconfirmed." });
  assert.match(schedules, /Saving schedule change/); assert.match(schedules, /outcome is unconfirmed/);
  assert.match(schedules, /disabled=""[^>]*>Pause health/);
  const notifications = render(NotificationsView, { value: page([note]), canAcknowledge: true, acknowledge() {}, more() {}, busy: true });
  assert.match(notifications, /Saving acknowledgment/); assert.match(notifications, /disabled=""[^>]*>Acknowledge note-1/);
  assert.doesNotMatch(notifications, />Acknowledged</);
});
