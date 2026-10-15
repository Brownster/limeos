import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { activityExpired, MediaActivityView, wire } from "./helpers/media-activity.mjs";

const clone = () => structuredClone(wire);
const render = props => renderToStaticMarkup(React.createElement(MediaActivityView, { now: 101, refresh() {}, ...props }));
test("Rust-bound activity wire shows stream progress and numeric totals", () => {
  const html = render({ value: clone() });
  assert.match(html, /Example title/); assert.match(html, /25.0%/); assert.match(html, /Transcoding/); assert.match(html, /viewer/);
  assert.match(html, /aria-label="Example title progress"/); assert.match(html, /1 \/ 1/);
  assert.doesNotMatch(html, />Start<|>Stop<|>Delete<|>Apply</);
});
test("unavailable activity never appears as an idle zero", () => {
  const html = render({ value: null }); assert.match(html, /Media activity is unavailable/); assert.doesNotMatch(html, /No streams observed|0 \/ 0/);
});
test("expired retained observations hide current metrics and mark old rows", () => {
  const html = render({ value: clone(), now: 109 }); assert.match(html, /Previous observation/); assert.match(html, /Current activity and totals are unknown/); assert.match(html, /Progress unknown/); assert.doesNotMatch(html, /25.0%|1 \/ 1/);
});
test("expiry includes original age bound and clock rollback", () => {
  assert.equal(activityExpired(wire, 108), false); assert.equal(activityExpired(wire, 109), true); assert.equal(activityExpired(wire, 99), true);
  assert.equal(activityExpired(wire, Number.NaN), true);
});
test("partial failure keeps good peer visible while totals stay unknown", () => {
  const value = clone(); value.complete = false; value.totals = Object.fromEntries(Object.keys(value.totals).map(key => [key, null]));
  const failed = structuredClone(value.services[0]); failed.target.container.resource = `container:${"f".repeat(64)}`; failed.target.name = "Sonarr"; failed.target.provider = "sonarr"; failed.state = "credential_unavailable"; value.services.push(failed);
  const html = render({ value }); assert.match(html, /Example title/); assert.match(html, /aggregate totals are unknown/); assert.match(html, /Service credential unavailable/); assert.doesNotMatch(html, /No supported media services/);
});
test("partial queues show sampled problems and inspected counts", () => {
  const value = clone(); value.complete = false; value.queues = [{ source: value.services[0].target.container.resource, data: { total: 50, observed: 12, problems: 2, complete: false, items: [{ title: "Queue item", state: "downloading", status: "warning" }] } }];
  const html = render({ value }); assert.match(html, /50 queued · 2 problems in 12 observed items · partial page/); assert.match(html, /Showing 1 of 12 inspected items/); assert.match(html, /Queue item/); assert.doesNotMatch(html, /1 \/ 1/);
});
test("download metrics preserve unknown rates and show seeding state", () => {
  const value = clone(); value.downloads = [{ source: value.services[0].target.container.resource, data: { name: "Transfer", state: "seeding", progress_percent: 50, size_bytes: 1000, remaining_bytes: null, speed_bytes: null, eta: null } }];
  const html = render({ value }); assert.match(html, /seeding/); assert.match(html, /Unknown rate/); assert.match(html, /Unknown size/); assert.match(html, /Unknown ETA/); assert.match(html, /50.0%/);
});
test("payload strings render as data without executable markup or links", () => {
  const value = clone(); value.streams[0].data.title = '<script>alert("x")</script>'; value.streams[0].data.user = '<img src=x onerror="alert(1)">';
  const html = render({ value }); assert.match(html, /&lt;script&gt;/); assert.match(html, /&lt;img/); assert.doesNotMatch(html, /<script>|<img|href="javascript:/);
});
test("display omissions and busy/error states are explicit", () => {
  const value = clone(); value.omitted_streams = 3; value.omitted_downloads = 2;
  const html = render({ value, loading: true, error: "Activity unavailable" }); assert.match(html, /3 additional observed streams/); assert.match(html, /2 additional observed transfers/); assert.match(html, /aria-busy="true"/); assert.match(html, /role="alert"/); assert.match(html, /disabled=""/);
});
test("only fresh complete empty discovery reports no supported services", () => {
  const value = clone(); value.streams = []; value.services = []; value.totals.streams = 0; value.totals.transcodes = 0;
  assert.match(render({ value }), /No supported media services were discovered/);
  value.complete = false; value.source = "incomplete";
  assert.match(render({ value }), /Media service discovery could not be confirmed/); assert.doesNotMatch(render({ value }), /No supported media services were discovered/);
});
