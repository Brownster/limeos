import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { automationClient, deliveryLabel, AutomationView, NotificationsView } from "./helpers/automation.mjs";
const definition = { operation: "observe_health", anchor: 1000, interval_seconds: 60, grace_seconds: 30, missed: "coalesce_latest", window: null };
const row = { schedule: { id: "health", revision: 4, paused: false, settled_through: null, definition }, status: "active", last_occurrence: null };
const note = { id: "note-1", revision: 2, event: { key: "mount:/mnt/Media", generation: 1, event: "incident", kind: "mount", severity: "critical", summary: "<script>alert(1)</script>", at: 1000 }, channel: "in_app", delivery: { state: "failed", retry_at: null }, attempts: 4, acknowledged_at: null };
const page = items => ({ items, next: null, partial: false });
const render = (Component, props) => renderToStaticMarkup(React.createElement(Component, props));

test("pause and acknowledgment send POST with current revision and CSRF", async () => {
  const saved = globalThis.fetch; const calls = [];
  globalThis.fetch = async (url, options) => { calls.push({ url, options }); return Response.json(url.includes("acknowledgment") ? note : row); };
  try {
    await automationClient.pause(row, "csrf"); await automationClient.acknowledge(note, "csrf"); await automationClient.create(definition, "csrf");
    assert.deepEqual(JSON.parse(calls[0].options.body), { revision: 4, paused: true });
    assert.deepEqual(JSON.parse(calls[1].options.body), { revision: 2 });
    assert.deepEqual(JSON.parse(calls[2].options.body), { definition });
    for (const call of calls) { assert.equal(call.options.method, "POST"); assert.equal(call.options.headers["X-CSRF-Token"], "csrf"); assert.equal(call.options.credentials, "same-origin"); }
  } finally { globalThis.fetch = saved; }
});
test("failed mutation remains a failure and is not retried", async () => {
  const saved = globalThis.fetch; let calls = 0;
  globalThis.fetch = async () => { calls++; return Response.json({ code: "conflict", message: "Revision changed. Refresh.", retry: false, audit_id: "case-1" }, { status: 409 }); };
  try { await assert.rejects(automationClient.pause(row, "csrf"), error => error.detail.code === "conflict"); assert.equal(calls, 1); }
  finally { globalThis.fetch = saved; }
});
test("reads encode cursor and retain partial state", async () => {
  const saved = globalThis.fetch; let call;
  globalThis.fetch = async (url, options) => { call = { url, options }; return Response.json({ ...page([row]), partial: true }); };
  try { assert.equal((await automationClient.schedules("case:one/two")).partial, true); assert.match(call.url, /after=case%3Aone%2Ftwo/); assert.equal(call.options.method, undefined); }
  finally { globalThis.fetch = saved; }
});
test("oversized and malformed response bodies are refused", async () => {
  const saved = globalThis.fetch;
  try {
    for (const response of [Response.json(page(Array(33).fill(row))), Response.json(page([{ schedule: {} }])), new Response("x".repeat(65537))]) {
      globalThis.fetch = async () => response;
      await assert.rejects(automationClient.schedules());
    }
  } finally { globalThis.fetch = saved; }
});
test("blocked schedules and partial observations have visible explanations", () => {
  const blocked = { ...row, schedule: { ...row.schedule, definition: { ...definition, operation: "backup" } }, status: "blocked" };
  const html = render(AutomationView, { schedules: { ...page([blocked]), partial: true }, alerts: page([]), canManage: true, create() {}, pause() {}, moreSchedules() {}, moreAlerts() {} });
  assert.match(html, /Operation adapter is unavailable/); assert.match(html, /Missing observations keep active alerts open/);
  assert.match(html, /Pause health/); assert.match(html, /Create observation schedule/);
});
test("read-only UI offers no schedule or acknowledgment mutations", () => {
  const html = render(AutomationView, { schedules: page([row]), alerts: page([]), canManage: false, moreSchedules() {}, moreAlerts() {} });
  assert.doesNotMatch(html, /Create observation schedule|Pause health|Resume health/);
  const notes = render(NotificationsView, { value: page([note]), canAcknowledge: false, more() {} });
  assert.match(notes, /Unacknowledged/); assert.doesNotMatch(notes, /Acknowledge note-1/);
});
test("notification UI preserves failed, ambiguous and acknowledgment states", () => {
  assert.match(deliveryLabel(note), /retries exhausted/);
  assert.match(deliveryLabel({ ...note, delivery: { state: "ambiguous", at: 1001 } }), /check receiver/);
  const html = render(NotificationsView, { value: { ...page([note]), partial: true }, canAcknowledge: true, acknowledge() {}, more() {} });
  assert.match(html, /Delivery failed/); assert.match(html, /Acknowledge note-1/); assert.match(html, /Some notification state is unavailable/);
  assert.match(html, /&lt;script&gt;/); assert.doesNotMatch(html, /<script>/);
});
test("latest durable occurrence and job state are visible after refresh", () => {
  const current = { ...row, last_occurrence: { key: { schedule: "health", revision: 4, due: 1000 }, outcome: { outcome: "job", id: "job-1", state: "succeeded" } } };
  const html = render(AutomationView, { schedules: page([current]), alerts: page([]), canManage: false, moreSchedules() {}, moreAlerts() {} });
  assert.match(html, /Latest occurrence/); assert.match(html, /Job job-1/); assert.match(html, /succeeded/);
});
test("read abort signal is forwarded and invalid-body 401 still expires the session", async () => {
  const saved = globalThis.fetch; const abort = new AbortController(); let signal;
  globalThis.fetch = async (_url, options) => { signal = options.signal; return new Response("not JSON", { status: 401 }); };
  try {
    await assert.rejects(automationClient.schedules(undefined, abort.signal), error => error.detail.code === "unauthenticated");
    abort.abort(); assert.equal(signal.aborted, true);
  } finally { globalThis.fetch = saved; }
});
test("invalid or contradictory error bodies cannot mask a forbidden read", async () => {
  const saved = globalThis.fetch;
  try {
    for (const response of [new Response("not JSON", { status: 403 }), Response.json({ code: "unavailable", message: "Proxy failure", retry: true, audit_id: "case-2" }, { status: 403 })]) {
      globalThis.fetch = async () => response;
      await assert.rejects(automationClient.schedules(), error => error.detail.code === "forbidden");
    }
  } finally { globalThis.fetch = saved; }
});
