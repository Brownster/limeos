import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, AutomationController, HealthHistory, RecordedAlerts, NotificationsView } from "./helpers/automation.mjs";

const page = (items = [], next = null, partial = false) => ({ items, next, partial });
const alert = { key: "mount:/Case/Path", kind: "mount", last_sequence: 2,
  incident: { phase: "open", generation: 2, severity: "warning", summary: "<script>fixture</script>", announced: true } };
const note = { id: "a".repeat(64), revision: 3, event: { key: alert.key, generation: 2, kind: "mount", event: "incident", severity: "warning", summary: "Fixture", at: 100 },
  channel: "in_app", delivery: { state: "delivered", at: 101 }, attempts: 1, acknowledged_at: null };
const render = (component, props) => renderToStaticMarkup(React.createElement(component, props));

test("history mode reads only alert/notification pages and cannot change schedules", async () => {
  const calls = []; let writes = 0;
  const reader = Object.fromEntries(["schedules", "alerts", "notifications"].map(kind => [kind, async () => { calls.push(kind); return page(kind === "notifications" ? [note] : [alert]); }]));
  const controller = new AutomationController(() => assert.fail("live session"), reader, "history");
  await controller.start(); await controller.first("schedules");
  await controller.mutate("schedules", async () => { writes++; });
  assert.deepEqual(calls.sort(), ["alerts", "notifications"]); assert.equal(writes, 0);
  assert.equal(controller.getSnapshot().schedules.page, null); controller.dispose();
});

test("history owner identity includes principal, grant revision and CSRF", () => {
  const props = { principal: "owner", grantRevision: 1, csrf: "csrf", expired() {} };
  const original = HealthHistory(props);
  for (const change of [{ principal: "other" }, { grantRevision: 2 }, { csrf: "replacement" }]) assert.notEqual(HealthHistory({ ...props, ...change }).key, original.key);
  assert.equal(HealthHistory({ ...props, expired() {} }).key, original.key);
});

test("stored alert display preserves incomplete, clear-streak and unavailable distinctions", () => {
  const html = render(RecordedAlerts, { value: page([alert], null, true), more() {} });
  assert.match(html, /Existing incidents stay open until a verified recovery/); assert.match(html, /mount:\/Case\/Path/);
  assert.match(html, /&lt;script&gt;/); assert.doesNotMatch(html, /<script>/);
  const clear = render(RecordedAlerts, { value: page([{ ...alert, incident: { phase: "clear", failures: 1 } }]), more() {} });
  assert.match(clear, /No open incident/); assert.match(clear, /1 unsuccessful observation/); assert.doesNotMatch(clear, /Healthy/);
  const missing = render(RecordedAlerts, { value: null, more() {} });
  assert.match(missing, /Alert data is unavailable/); assert.doesNotMatch(missing, /No alert history/);
  assert.match(render(HealthHistory, { principal: "owner", grantRevision: 1, csrf: "csrf", expired() {} }), /New automatic health observations are not available yet/);
});

test("acknowledgment requires delivered state and stays one POST after an ambiguous reply", async () => {
  for (const delivery of [{ state: "pending" }, { state: "failed", retry_at: null }, { state: "ambiguous", at: 101 }]) {
    const html = render(NotificationsView, { value: page([{ ...note, delivery }]), canAcknowledge: true, acknowledge() {}, more() {} });
    assert.doesNotMatch(html, /<button[^>]*>Acknowledge/); assert.match(html, /Awaiting verified delivery/);
  }
  let sends = 0;
  const reader = Object.fromEntries(["schedules", "alerts", "notifications"].map(kind => [kind, async () => page(kind === "notifications" ? [note] : [])]));
  const controller = new AutomationController(() => {}, reader, "history"); await controller.start();
  await controller.mutate("notifications", async () => { sends++; throw new ApiError({ code: "unavailable", message: "Lost acknowledgment reply", retry: true, audit_id: "fixture" }); });
  await controller.refresh(); assert.equal(sends, 1); controller.dispose();
});
