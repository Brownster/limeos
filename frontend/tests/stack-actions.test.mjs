import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { StackActionController, stackTargets, stackAvailability, StackJobRow, StackManagement, wire } from "./helpers/stack-discovery.mjs";

const clone = () => structuredClone(wire);
const operations = { csrf: "c".repeat(64), revision: 0, changed() {}, expired() {} };
const render = props => renderToStaticMarkup(React.createElement(StackManagement, { refresh() {}, ...props }));
const member = (c, running) => ({ resource: `container:${c.repeat(64)}`, image: `sha256:${"b".repeat(64)}`, started_at: "2026-10-05T07:00:00Z", running });
const proposal = (operation = "restart", project = "media") => ({
  plan: { id: "p".repeat(64), version: 1, operation, principal: "admin", grant_revision: 1, created_at: 100, expires_at: 400,
    expected: { resource: `stack:${project}`, members: [member("a", true), member("b", false), member("c", true)] } },
  digest: "d".repeat(64),
});
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function harness(overrides = {}) {
  const calls = [];
  const client = {
    plan: async (...args) => { calls.push(["plan", ...args]); return proposal(args[1]); },
    approve: async (...args) => { calls.push(["approve", ...args]); return { token: "t".repeat(64), expires_at: 400 }; },
    queue: async (...args) => { calls.push(["queue", ...args]); return { id: "j".repeat(64) }; },
    ...overrides,
  };
  let changes = 0, serial = 0;
  const controller = new StackActionController("media", "csrf", client, () => { changes += 1; }, e => e.message, () => `key-${++serial}`);
  return { controller, calls, changes: () => changes };
}

test("stack controls exist only for an authenticated operations session and name no file or container", () => {
  assert.doesNotMatch(render({ value: clone() }), /stack<\/button>/);
  const html = render({ value: clone(), operations });
  for (const label of ["Start stack", "Stop stack", "Restart stack"]) assert.match(html, new RegExp(`>${label}<`));
  assert.match(html, /never create, remove or edit/);
  assert.doesNotMatch(html, />(Apply|Delete|Up|Down|Recreate|Remove)[ <]/);
  assert.doesNotMatch(html, /disabled=""[^>]*>(Start|Stop|Restart) stack/);
});

test("availability follows observed state and never offers an unknown stack", () => {
  const stack = clone().stacks[0];
  for (const action of ["start", "stop", "restart"]) assert.equal(stackAvailability(stack, false, action), null);
  for (const action of ["start", "stop", "restart"]) assert.match(stackAvailability(stack, true, action), /unknown/);
  const running = structuredClone(stack); running.counts.running = running.counts.ordinary;
  assert.match(stackAvailability(running, false, "start"), /already running/);
  assert.equal(stackAvailability(running, false, "stop"), null);
  const stopped = structuredClone(stack); stopped.counts.running = 0;
  assert.equal(stackAvailability(stopped, false, "start"), null);
  assert.match(stackAvailability(stopped, false, "stop"), /No service container is running/);
  assert.match(stackAvailability(stopped, false, "restart"), /No service container is running/);
  for (const change of [s => { s.issues = ["undeclared_services"]; }, s => { s.runtime = "conflict"; }, s => { s.runtime = "unknown"; },
    s => { s.counts = null; }, s => { s.counts.ordinary = 0; s.counts.running = 0; }]) {
    const broken = structuredClone(stack); change(broken);
    assert.notEqual(stackAvailability(broken, false, "restart"), null);
  }
  const html = render({ value: clone(), stale: true, operations });
  assert.equal((html.match(/disabled=""/g) ?? []).length >= 3, true);
  assert.match(html, /title="Current stack state is unknown\."/);
});

test("targets are the stopped members for start and the running members otherwise", () => {
  const p = proposal();
  assert.deepEqual(stackTargets(p, "start").map(m => m.resource[10]), ["b"]);
  assert.deepEqual(stackTargets(p, "stop").map(m => m.resource[10]), ["a", "c"]);
  assert.deepEqual(stackTargets(p, "restart").map(m => m.resource[10]), ["a", "c"]);
});

test("preview plans the exact project and action; approval is a separate confirmed step", async () => {
  const { controller, calls, changes } = harness();
  await controller.preview("stop");
  assert.deepEqual(calls, [["plan", "media", "stop", "csrf"]]);
  assert.equal(controller.getSnapshot().proposal.plan.operation, "stop");
  assert.equal(controller.getSnapshot().queued, false);
  assert.equal(changes(), 0);
  await controller.confirm();
  assert.deepEqual(calls.map(c => c[0]), ["plan", "approve", "queue"]);
  assert.equal(calls[1][2], "csrf"); assert.equal(calls[2][2], "t".repeat(64));
  assert.equal(calls[2][3], "key-1"); assert.equal(calls[2][4], "csrf");
  assert.equal(controller.getSnapshot().queued, true);
  assert.equal(changes(), 1);
});

test("a plan for another stack or action is refused and cannot be approved", async () => {
  for (const wrong of [proposal("stop", "other"), proposal("start")]) {
    const { controller, calls } = harness({ plan: async () => wrong });
    await controller.preview("stop");
    assert.equal(controller.getSnapshot().proposal, null);
    assert.match(controller.getSnapshot().error, /identity mismatch/);
    await controller.confirm();
    assert.deepEqual(calls, []);
  }
});

test("an interrupted queue retries the same approval and request key without a second approval", async () => {
  let fail = true;
  const { controller, calls, changes } = harness({ queue: async (...args) => {
    calls.push(["queue", ...args]);
    if (fail) { fail = false; throw new Error("The host is unavailable."); }
    return { id: "j".repeat(64) };
  } });
  await controller.preview("restart");
  await controller.confirm();
  assert.equal(controller.getSnapshot().retained, true); assert.equal(controller.getSnapshot().queued, false);
  assert.match(controller.getSnapshot().error, /unavailable/);
  assert.equal(changes(), 1);
  await controller.confirm();
  assert.deepEqual(calls.map(c => c[0]), ["plan", "approve", "queue", "queue"]);
  assert.equal(calls[2][3], calls[3][3]); assert.equal(calls[2][2], calls[3][2]);
  assert.equal(controller.getSnapshot().queued, true); assert.equal(controller.getSnapshot().retained, false);
});

test("a failed approval keeps no request key and a new preview discards a retained one", async () => {
  let approvals = 0;
  const { controller, calls } = harness({ approve: async (...args) => { calls.push(["approve", ...args]); if (++approvals === 1) throw new Error("denied"); return { token: "t".repeat(64), expires_at: 400 }; } });
  await controller.preview("restart");
  await controller.confirm();
  assert.equal(controller.getSnapshot().retained, false);
  assert.equal(calls.some(c => c[0] === "queue"), false);
  await controller.confirm();
  assert.equal(calls.filter(c => c[0] === "queue").length, 1);
  await controller.preview("stop");
  assert.equal(controller.getSnapshot().retained, false);
  assert.equal(controller.getSnapshot().queued, false);
});

test("repeated confirmation while pending and late previews cannot submit twice or overwrite a newer one", async () => {
  const gate = deferred();
  const { controller, calls } = harness({ approve: async (...args) => { calls.push(["approve", ...args]); await gate.promise; return { token: "t".repeat(64), expires_at: 400 }; } });
  await controller.preview("restart");
  const first = controller.confirm();
  await controller.confirm();
  gate.resolve(); await first;
  assert.equal(calls.filter(c => c[0] === "approve").length, 1);
  assert.equal(calls.filter(c => c[0] === "queue").length, 1);

  const slow = deferred(); let n = 0;
  const raced = harness({ plan: async (project, action) => { if (++n === 1) { await slow.promise; return proposal("start"); } return proposal(action); } });
  const older = raced.controller.preview("start");
  await raced.controller.preview("stop");
  slow.resolve(); await older;
  assert.equal(raced.controller.getSnapshot().action, "stop");
  assert.equal(raced.controller.getSnapshot().proposal.plan.operation, "stop");
});

test("closing or disposing the dialog discards the proposal and ignores late results", async () => {
  const gate = deferred();
  const { controller } = harness({ plan: async () => { await gate.promise; return proposal("stop"); } });
  const pending = controller.preview("stop");
  controller.close();
  gate.resolve(); await pending;
  assert.equal(controller.getSnapshot().proposal, null); assert.equal(controller.getSnapshot().action, null);
  const seen = []; controller.subscribe(() => seen.push(1));
  controller.dispose(); await controller.preview("stop");
  assert.deepEqual(seen, []);
});

test("job rows show per-container outcomes, intervention warnings and cancel only while queued", () => {
  const job = state => ({ id: "j".repeat(64), state, plan: proposal("restart").plan });
  const receipt = { action: "j".repeat(64), plan_digest: "d".repeat(64), operation: "restart", before: proposal().plan.expected, state: "outcome_unknown", error: "unavailable",
    members: [{ resource: member("a", true).resource, state: "verified", error: null }, { resource: member("c", true).resource, state: "outcome_unknown", error: "unavailable" },
      { resource: member("d", true).resource, state: "precondition_changed", error: "conflict" }] };
  const html = renderToStaticMarkup(React.createElement(StackJobRow, { job: job("needs_intervention"), receipt, cancel() {}, details() {} }));
  assert.match(html, /Restart stack · media/); assert.match(html, /needs intervention/); assert.match(html, /remain\s+locked/);
  assert.match(html, /aaaaaaaaaaaa<\/code> · verified/); assert.match(html, /outcome unknown \(unavailable\)/);
  assert.match(html, /not attempted \(conflict\)/);
  assert.doesNotMatch(html, /Cancel queued/);
  const queued = renderToStaticMarkup(React.createElement(StackJobRow, { job: job("queued"), cancel() {}, details() {} }));
  assert.match(queued, /Cancel queued restart stack/);
});
