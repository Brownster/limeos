import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  inventoryUnknown, readerDown, stackStatus, countsSentence, runningPercent, containerIndex, serviceRows, unhealthyCount,
  omittedRows, stackSummary, observedStacks, dockerFresh, jobForStack, jobOutcomes, jobHeadline, shortId, RECENT_JOB_SECONDS,
} from "../src/stacks/stack-model.ts";
import { StackManagement, StackJobStrip, wire } from "./helpers/stack-discovery.mjs";

const NOW = 1_800_000_000;
const id = (n) => `container:${n.toString(16).padStart(64, "0")}`;
const resource = (patch) => ({
  id: patch.id ?? `x:${patch.name}`, kind: "container", name: "x", status: "running", source: "docker", parent: null, identity: null,
  filesystem: null, mountpoint: null, size_bytes: null, image: null, health: null, cpu_percent: null, memory_percent: null, ...patch,
});
const clone = () => structuredClone(wire);
const fresh = (v) => { v.catalog.sampled_at = NOW - 5; v.engine.sampled_at = NOW - 5; v.generated_at = NOW - 4; return v; };
const stack = (patch = {}) => ({ ...clone().stacks[0], ...patch });

// ---- Stack status ---------------------------------------------------------

test("stack status: running, partly running, stopped, empty, conflict", () => {
  const counts = (running, ordinary) => ({ ordinary, running, one_offs: 0, one_off_running: 0 });
  assert.deepEqual(
    [counts(2, 2), counts(1, 2), counts(0, 2), counts(0, 0)].map((c) => stackStatus(stack({ counts: c, runtime: "partial" }), null).label),
    ["Running", "Partly running", "Stopped", "No service containers"],
  );
  assert.equal(stackStatus(stack({ counts: counts(2, 2) }), null).tone, "success");
  assert.equal(stackStatus(stack({ counts: counts(1, 2) }), null).tone, "warning");
  assert.equal(stackStatus(stack({ runtime: "conflict", counts: null }), null).label, "Identity conflict");
  assert.equal(stackStatus(stack({ issues: ["overlapping_project"] }), null).key, "conflict");
  assert.equal(stackStatus(stack({ runtime: "unknown", counts: null }), null).label, "Unknown");
});

test("stack status is never a clean Running from an incomplete or stale inventory", () => {
  const all = { ordinary: 2, running: 2, one_offs: 0, one_off_running: 0 };
  const incomplete = stackStatus(stack({ counts: all, issues: ["incomplete_inventory"] }), null);
  assert.equal(incomplete.label, "Running"); assert.equal(incomplete.tone, "warning"); assert.match(incomplete.reason, /couldn't be read/);
  const value = fresh(clone());
  assert.equal(inventoryUnknown(value, false), null);
  assert.match(inventoryUnknown(value, true), /90 seconds/);
  value.engine.state = "stale"; assert.match(inventoryUnknown(value, false), /Docker reading is out of date/);
  value.engine.state = "unavailable"; assert.match(inventoryUnknown(value, false), /Docker can't be read/);
  value.catalog.state = "unavailable"; assert.match(inventoryUnknown(value, false), /stack reader isn't answering/); assert.equal(readerDown(value), true);
  const other = fresh(clone()); other.engine_matches_catalog = false; assert.match(inventoryUnknown(other, false), /different Docker Engine/);
  const status = stackStatus(stack({ counts: all }), "Docker can't be read right now.");
  assert.equal(status.label, "Unknown"); assert.equal(status.tone, "neutral"); assert.notEqual(status.tone, "success");
  assert.equal(countsSentence(all, true), "Current container counts are unknown.");
  assert.equal(countsSentence({ ...all, running: 1 }, false), "1 of 2 service containers running");
  assert.equal(runningPercent({ ...all, running: 1 }, false), 50);
  assert.equal(runningPercent(all, true), null);
  assert.equal(runningPercent({ ...all, ordinary: 0, running: 0 }, false), null);
});

test("stack summary counts states and withholds totals when any stack is unknown", () => {
  const value = fresh(clone());
  const second = structuredClone(value.stacks[0]); second.stack = "music"; second.project = "music";
  second.counts = { ordinary: 3, running: 3, one_offs: 0, one_off_running: 0 }; value.stacks.push(second);
  assert.deepEqual(stackSummary(value, null), { stacks: 2, running: 1, partial: 1, stopped: 0, unknown: 0, services: { running: 4, total: 5 } });
  const unknown = stackSummary(value, "stale");
  assert.equal(unknown.unknown, 2); assert.equal(unknown.services, null);
});

// ---- Member names -----------------------------------------------------------

test("members are named from the overview, fall back to short IDs, and carry current health", () => {
  const index = containerIndex([
    resource({ id: id(0), name: "web-1", status: "running", health: "unhealthy" }),
    resource({ id: id(2), name: "web-run", status: "running" }),
    resource({ id: "stack:media", kind: "stack", name: "media" }),
  ]);
  assert.equal(index.size, 2, "only containers are indexed");
  const [web] = serviceRows(clone().stacks[0].services, index, { live: true, previous: false });
  assert.deepEqual(web.members.map((m) => m.label), ["web-1", "000000000000", "web-run"]);
  assert.equal(web.members[1].name, null);
  assert.equal(web.members[0].tone, "danger"); assert.equal(web.members[0].health, "unhealthy");
  assert.equal(web.members[1].state, "not running");
  assert.equal(unhealthyCount([web]), 1);
  assert.equal(shortId(id(255)), "000000000000");
  // A stale docker source: names stay, status and health are not claimed.
  const [old] = serviceRows(clone().stacks[0].services, index, { live: false, previous: false });
  assert.equal(old.members[0].label, "web-1"); assert.equal(old.members[0].health, null); assert.equal(old.members[0].state, "running");
  // A stale inventory: its own states are past observations.
  const [prev] = serviceRows(clone().stacks[0].services, new Map(), { live: false, previous: true });
  assert.equal(prev.members[0].state, "last seen running"); assert.equal(prev.members[1].state, "last seen stopped");
  assert.equal(prev.running, null); assert.equal(prev.members[0].tone, "neutral");
});

test("omitted containers are listed by name where the overview knows them", () => {
  const value = clone();
  value.omitted_containers = [{ resource: id(9), project: "media", reason: "changing" }, { resource: id(10), project: null, reason: "unsupported" }];
  const rows = omittedRows(value, containerIndex([resource({ id: id(9), name: "lidarr" })]));
  assert.deepEqual(rows.map((r) => [r.label, r.project, r.reason]), [["lidarr", "media", "changing"], [shortId(id(10)), null, "unsupported"]]);
  assert.equal(rows[1].name, null);
});

test("the host observer's stacks: grouping, counts and an out-of-date source", () => {
  const resources = [
    resource({ id: "stack:media", kind: "stack", name: "media" }),
    resource({ id: "c1", name: "sonarr", parent: "stack:media" }),
    resource({ id: "c2", name: "lidarr", parent: "stack:media", status: "exited" }),
    resource({ id: "c3", name: "jellyfin", parent: "stack:media", health: "unhealthy" }),
    resource({ id: "c4", name: "navidrome" }),
  ];
  const [media] = observedStacks(resources, true);
  assert.deepEqual(media.containers.map((c) => c.name), ["jellyfin", "lidarr", "sonarr"]);
  assert.equal(media.running, 2); assert.equal(media.total, 3); assert.equal(media.unhealthy, 1);
  assert.equal(media.status.label, "Partly running");
  assert.equal(observedStacks(resources, false)[0].status.label, "Unknown");
  const sources = [{ source: "docker", state: "fresh", sampled_at: NOW - 10, max_age_seconds: 90, warnings: [] }];
  assert.equal(dockerFresh(sources, NOW), true);
  assert.equal(dockerFresh(sources, NOW + 200), false);
  assert.equal(dockerFresh([], NOW), false);
});

// ---- Jobs on cards ------------------------------------------------------------

const member = (n, running) => ({ resource: id(n), image: `sha256:${"b".repeat(64)}`, started_at: "2026-10-05T07:00:00Z", running });
const job = (state, operation = "restart", created = NOW - 30) => ({
  id: "j".repeat(64), state,
  plan: { id: "p".repeat(64), version: 1, operation, principal: "admin", grant_revision: 1, catalog_stack: "media", created_at: created, expires_at: created + 300,
    expected: { resource: "stack:media", members: [member(1, true), member(2, false), member(3, true)] } },
});

test("a stack's card shows its job in flight, else one finished in the last 15 minutes", () => {
  const media = { stack: "media", project: "media" };
  assert.equal(jobForStack([job("running")], media, NOW).state, "running");
  assert.equal(jobForStack([job("succeeded"), { ...job("needs_intervention"), id: "k".repeat(64) }], media, NOW).state, "needs_intervention");
  assert.equal(jobForStack([job("succeeded", "restart", NOW - RECENT_JOB_SECONDS - 1)], media, NOW), null);
  assert.equal(jobForStack([job("running")], { stack: "music", project: "music" }, NOW), null);
});

test("job outcomes per container: targets by operation, receipts where reported, names where known", () => {
  const index = containerIndex([resource({ id: id(1), name: "sonarr" })]);
  assert.deepEqual(jobOutcomes(job("running"), null, index).map((r) => [r.label, r.outcome]), [["sonarr", "waiting"], ["000000000000", "waiting"]]);
  assert.deepEqual(jobOutcomes(job("queued", "start"), null, index).map((r) => r.resource), [id(2)]);
  const receipt = { action: "j", plan_digest: "d", operation: "restart", before: job("x").plan.expected, state: "partly_done", error: null,
    members: [{ resource: id(1), state: "verified", error: null }, { resource: id(3), state: "precondition_changed", error: "conflict" }] };
  assert.deepEqual(jobOutcomes(job("failed"), receipt, index).map((r) => [r.outcome, r.tone]), [["done", "success"], ["not attempted (conflict)", "neutral"]]);
  assert.equal(jobHeadline(job("failed"), receipt).text, "Restarting stack · partly done, 1 of 2");
  assert.equal(jobHeadline(job("running")).text, "Restarting stack · in progress");
  assert.equal(jobHeadline(job("succeeded", "stop")).text, "Stopped stack");
  const html = renderToStaticMarkup(React.createElement(StackJobStrip, { job: job("running"), receipt: null, names: index }));
  assert.match(html, /Restarting stack · in progress/); assert.match(html, /sonarr/); assert.match(html, /waiting/);
});

// ---- The rendered page ----------------------------------------------------------

test("cards name containers, keep IDs in the disclosure and link to the Containers page", () => {
  const value = fresh(clone());
  const resources = [resource({ id: id(0), name: "media-web-1" }), resource({ id: id(1), name: "media-web-2", status: "exited" })];
  const sources = [{ source: "docker", state: "fresh", sampled_at: NOW - 3, max_age_seconds: 90, warnings: [] }];
  const html = renderToStaticMarkup(React.createElement(StackManagement, { value, refresh() {}, resources, sources, now: NOW }));
  assert.match(html, /href="\/containers\?search=media-web-1"/);
  assert.match(html, /Partly running/);
  const card = html.slice(html.indexOf('aria-label="Stack media"'), html.indexOf("<details"));
  assert.doesNotMatch(card, /0{64}|sha256:/, "IDs and digests stay behind the disclosure");
  assert.match(card, /exited/);
  assert.match(html, /<details[^>]*>.*0{63}1/s);
});

test("the reader down after a reboot is said plainly, with the observer's stacks", () => {
  const value = fresh(clone());
  for (const s of ["catalog", "engine"]) Object.assign(value[s], { state: "unavailable", sampled_at: null, valid_for_millis: 0 });
  value.stacks = []; value.engine_matches_catalog = null; value.standalone_containers = null;
  const resources = [resource({ id: "stack:media", kind: "stack", name: "media" }), resource({ id: id(1), name: "jellyfin", parent: "stack:media" })];
  const sources = [{ source: "docker", state: "fresh", sampled_at: NOW - 3, max_age_seconds: 90, warnings: [] }];
  const html = renderToStaticMarkup(React.createElement(StackManagement, { value, refresh() {}, resources, sources, now: NOW }));
  assert.match(html, /The stack reader isn&#x27;t running/); assert.match(html, /systemctl start limeos-storage-reader/);
  assert.match(html, /Observed stack media/); assert.match(html, /jellyfin/);
  assert.doesNotMatch(html, /No stacks are registered/);
});

test("no registered stacks explains the management catalog; denied clears the stacks", () => {
  const value = fresh(clone()); value.stacks = [];
  const html = renderToStaticMarkup(React.createElement(StackManagement, { value, refresh() {}, now: NOW }));
  assert.match(html, /No stacks are registered\./); assert.match(html, /management-catalog\.json/); assert.match(html, /propose-catalog/);
  const denied = renderToStaticMarkup(React.createElement(StackManagement, { value: null, denied: true, error: "You no longer have access to stack inventory.", refresh() {} }));
  assert.match(denied, /You no longer have access to stack inventory\./); assert.doesNotMatch(denied, /role="alert"/);
});
