import test from "node:test";
import assert from "node:assert/strict";
import { parseRoute, navRoutes, documentTitle, routes } from "../src/app/routes.ts";
import { liveState, pollNeeded, STREAM_GRACE_SECONDS } from "../src/app/live-status.ts";
import { healthSummary, metricTone, metricBarValue, workloadCounts, thresholds } from "../src/overview/overview-model.ts";
import { describeError } from "../src/ui/error-copy.ts";
import { formatBytes, formatDuration, formatPercent, formatCelsius, formatAge, formatLoad } from "../src/ui/format.ts";
import { seriesPaths, summarize, domainFor, nearestIndex, timeSpan, xFor, yFor, axisTimes } from "../src/overview/chart-model.ts";

const NOW = 1_800_000_000;
const fresh = (source) => ({ source, state: "fresh", sampled_at: NOW - 5, max_age_seconds: 90, warnings: [] });
const resource = (patch) => ({
  id: patch.id ?? patch.name, kind: "container", name: "x", status: "running", source: "docker", parent: null, identity: null,
  filesystem: null, mountpoint: null, size_bytes: null, image: null, health: null, cpu_percent: null, memory_percent: null, ...patch,
});
const overview = (patch = {}) => ({
  revision: 1, generated_at: NOW - 2,
  host: { cpu_percent: 10, memory_percent: 40, temperature_celsius: 50, disk_percent: 30, load_one: 0.5, uptime_seconds: 3600 },
  sources: ["host", "storage", "pools", "docker", "history"].map(fresh),
  resources: [resource({ name: "jellyfin" })],
  ...patch,
});

test("routes: canonical paths, aliases, normalisation and unknown paths", () => {
  assert.deepEqual(parseRoute("/"), { id: "overview", path: "/", redirect: false });
  assert.deepEqual(parseRoute("/containers"), { id: "containers", path: "/containers", redirect: false });
  assert.deepEqual(parseRoute("/alerts/policy"), { id: "alerts-policy", path: "/alerts/policy", redirect: false });
  assert.deepEqual(parseRoute("/stacks/"), { id: "stacks", path: "/stacks", redirect: true });
  assert.deepEqual(parseRoute("//Storage//"), { id: "storage", path: "/storage", redirect: true });
  for (const [old, now] of [["/disks", "/storage"], ["/pools", "/storage"], ["/mounts", "/storage"], ["/system", "/"], ["/index.html", "/"], ["/notifications", "/alerts"]])
    assert.deepEqual(parseRoute(old), { id: parseRoute(now).id, path: now, redirect: true }, old);
  assert.equal(parseRoute("/nope").id, "not-found");
  assert.equal(parseRoute("/alerts/unknown").id, "not-found");
  assert.equal(parseRoute("").id, "overview");
  assert.equal(documentTitle("containers"), "LimeOS · Containers");
  assert.equal(documentTitle("not-found"), "LimeOS · Page not found");
  assert.deepEqual(navRoutes(false).map((r) => r.path), ["/", "/containers", "/stacks", "/storage", "/alerts"]);
  assert.equal(new Set(routes.map((r) => r.path)).size, routes.length);
});

test("live status: the stream's 25 s rotation never reads as reconnecting", () => {
  // Snapshots every 5 s; the stream ends at 25 s and the browser reconnects ~3 s later.
  for (let gap = 0; gap <= STREAM_GRACE_SECONDS; gap++)
    assert.equal(liveState({ snapshotAt: NOW - gap, succeededAt: NOW - gap, failedAt: null, now: NOW }), "live", `gap ${gap}`);
  assert.equal(liveState({ snapshotAt: NOW - 40, succeededAt: NOW - 10, failedAt: null, now: NOW }), "polling");
  assert.equal(liveState({ snapshotAt: null, succeededAt: NOW - 200, failedAt: null, now: NOW }), "reconnecting");
  assert.equal(liveState({ snapshotAt: null, succeededAt: NOW - 20, failedAt: NOW - 5, now: NOW }), "offline");
  assert.equal(liveState({ snapshotAt: null, succeededAt: null, failedAt: null, now: NOW }), "connecting");
  // A current stream outranks an older failed GET.
  assert.equal(liveState({ snapshotAt: NOW - 2, succeededAt: NOW - 2, failedAt: NOW - 30, now: NOW }), "live");
  assert.equal(pollNeeded(NOW - 10, NOW), false);
  assert.equal(pollNeeded(NOW - 31, NOW), true);
  assert.equal(pollNeeded(null, NOW), true);
});

test("thresholds match the previous app and unknown is not a colour", () => {
  assert.deepEqual(thresholds, { cpu: { warning: 60, danger: 85 }, memory: { warning: 70, danger: 90 }, disk: { warning: 75, danger: 90 }, temperature: { warning: 65, danger: 80 } });
  assert.equal(metricTone("cpu", 59.9), "success");
  assert.equal(metricTone("cpu", 60), "warning");
  assert.equal(metricTone("cpu", 85), "danger");
  assert.equal(metricTone("temperature", 79.9), "warning");
  assert.equal(metricTone("disk", 0), "success");
  assert.equal(metricTone("memory", null), "neutral");
  assert.equal(metricTone("memory", NaN), "neutral");
  assert.equal(metricBarValue("temperature", 45), 50);
  assert.equal(metricBarValue("temperature", 120), 100);
  assert.equal(metricBarValue("cpu", -3), 0);
  assert.equal(metricBarValue("cpu", null), null);
});

test("health: healthy only when everything is current and fine", () => {
  assert.equal(healthSummary(overview(), NOW).state, "healthy");
  assert.equal(healthSummary(null, NOW).state, "unknown");
  assert.equal(healthSummary(null, NOW, true).title, "Can't reach the host");
  const stale = overview({ sources: [...overview().sources.slice(0, 3), { ...fresh("docker"), sampled_at: NOW - 600 }, fresh("history")] });
  const s = healthSummary(stale, NOW);
  assert.equal(s.state, "unknown");
  assert.ok(s.issues.some((i) => i.label === "docker readings are out of date"));
  assert.equal(healthSummary(overview({ sources: [] }), NOW).state, "unknown");
  assert.equal(healthSummary(overview({ host: null }), NOW).state, "unknown");
  assert.equal(healthSummary(overview({ generated_at: NOW - 600 }), NOW).state, "unknown");
  assert.equal(healthSummary(overview(), NOW, true).state, "unknown");
  const unavailable = overview({ sources: [{ ...fresh("host"), state: "unavailable", sampled_at: null }] });
  assert.equal(healthSummary(unavailable, NOW).issues[0].label, "host readings are unavailable");
});

test("health: attention and critical, with stale data still saying so", () => {
  const hot = healthSummary(overview({ host: { ...overview().host, temperature_celsius: 81 } }), NOW);
  assert.equal(hot.state, "critical");
  assert.equal(hot.issues[0].label, "Temperature is very high: 81.0 °C");
  const unhealthy = healthSummary(overview({ resources: [resource({ name: "transmission", health: "unhealthy" })] }), NOW);
  assert.equal(unhealthy.state, "attention");
  assert.equal(unhealthy.issues[0].path, "/containers");
  const missing = healthSummary(overview({ resources: [resource({ kind: "partition", name: "sda1", status: "missing", source: "storage" })] }), NOW);
  assert.equal(missing.state, "critical");
  assert.equal(missing.issues[0].path, "/storage");
  const degraded = healthSummary(overview({ resources: [resource({ kind: "stack", name: "media", status: "degraded" })] }), NOW);
  assert.equal(degraded.issues[0].label, "media stack is only partly running");
  const both = healthSummary(overview({ generated_at: NOW - 600, resources: [resource({ name: "a", status: "restarting" })] }), NOW);
  assert.equal(both.state, "attention");
  assert.match(both.detail, /may be more/);
  // A stopped container is a count, not an issue: people stop things on purpose.
  assert.equal(healthSummary(overview({ resources: [resource({ name: "lidarr", status: "exited" })] }), NOW).state, "healthy");
});

test("workload counts from resources", () => {
  const counts = workloadCounts([
    resource({ name: "a" }), resource({ name: "b", health: "unhealthy" }), resource({ name: "c", status: "exited" }),
    resource({ name: "d", status: "dead" }), resource({ name: "e", status: "paused" }),
    resource({ kind: "stack", name: "s1", status: "running" }), resource({ kind: "stack", name: "s2", status: "degraded" }),
    resource({ kind: "stack", name: "s3", status: "stopped" }), resource({ kind: "disk", name: "sda", status: "unmounted" }),
  ]);
  assert.deepEqual(counts, { containers: { total: 5, running: 2, unhealthy: 2, stopped: 2 }, stacks: { total: 3, running: 1, partial: 1, stopped: 1 } });
  assert.deepEqual(workloadCounts([]), { containers: { total: 0, running: 0, unhealthy: 0, stopped: 0 }, stacks: { total: 0, running: 0, partial: 0, stopped: 0 } });
});

test("errors: friendly copy first, reference kept for support", () => {
  const e = describeError({ detail: { code: "unavailable", message: "Observer down", retry: true, audit_id: "abc-123" } }, "Host observations");
  assert.equal(e.title, "Host observations can't be read right now");
  assert.equal(e.reference, "abc-123");
  assert.equal(e.serverMessage, "Observer down");
  assert.equal(e.retry, true);
  assert.equal(describeError({ detail: { code: "forbidden", message: "x", retry: false, audit_id: "r" } }, "Stacks").retry, false);
  assert.equal(describeError({ detail: { code: "unavailable", message: "x", retry: true, audit_id: "<bad id>" } }, "X").reference, null);
  const offline = describeError(new TypeError("Failed to fetch"), "History");
  assert.equal(offline.reference, null);
  assert.match(offline.detail, /couldn't reach the host/);
  assert.match(describeError({ detail: { code: "brand_new", message: "x", retry: true, audit_id: "r" } }, "X").detail, /quote the reference/);
});

test("formatting says unknown instead of zero", () => {
  assert.equal(formatPercent(null), "Unknown");
  assert.equal(formatPercent(0), "0.0%");
  assert.equal(formatCelsius(undefined), "Unknown");
  assert.equal(formatCelsius(58.26), "58.3 °C");
  assert.equal(formatBytes(4e12), "4.0 TB");
  assert.equal(formatBytes(999), "999 B");
  assert.equal(formatBytes(123_400_000), "123 MB");
  assert.equal(formatBytes(null), "Unknown");
  assert.equal(formatDuration(3 * 86400 + 4 * 3600 + 17 * 60), "3d 4h");
  assert.equal(formatDuration(5 * 3600 + 12 * 60), "5h 12m");
  assert.equal(formatDuration(30), "under a minute");
  assert.equal(formatDuration(-1), "Unknown");
  assert.equal(formatLoad(1.4249), "1.42");
  assert.equal(formatAge(NOW - 2, NOW), "just now");
  assert.equal(formatAge(NOW - 300, NOW), "5 min ago");
  assert.equal(formatAge(null, NOW), "never");
});

const pt = (at, cpu, temp = 50) => ({ at, cpu_percent: cpu, memory_percent: null, temperature_celsius: temp, disk_percent: 40 });

test("chart scaling: time axis, clamped values and broken lines over unknown time", () => {
  const span = timeSpan([pt(100, 1), pt(200, 2)], 0, 400);
  assert.deepEqual(span, { from: 0, to: 400 });
  assert.deepEqual(timeSpan([pt(100, 1), pt(200, 2)]), { from: 100, to: 200 });
  assert.deepEqual(timeSpan([pt(100, 1)]), { from: 99, to: 101 });
  assert.deepEqual(timeSpan([]), { from: -1, to: 1 });
  assert.equal(xFor(200, span), 500);
  assert.equal(yFor(0, [0, 100]), 200);
  assert.equal(yFor(100, [0, 100]), 0);
  assert.equal(yFor(150, [0, 100]), 0);
  assert.equal(yFor(-5, [0, 100]), 200);
  const points = [pt(0, 10), pt(100, 20), pt(200, null), pt(300, 30), pt(400, 40), pt(1000, 50)];
  const paths = seriesPaths(points, "cpu_percent", [0, 100], { from: 0, to: 1000 }, 100);
  assert.deepEqual(paths, ["M 0.0 180.0 L 100.0 160.0", "M 300.0 140.0 L 400.0 120.0", "M 1000.0 100.0 L 1000.0 100.0"]);
  assert.deepEqual(seriesPaths(points, "memory_percent", [0, 100], { from: 0, to: 1000 }, 100), []);
});

test("chart summaries, domains and the nearest sample", () => {
  assert.deepEqual(summarize([pt(0, 10), pt(1, null), pt(2, 30)], "cpu_percent"), { current: 30, min: 10, max: 30, average: 20 });
  assert.equal(summarize([pt(0, 10), pt(1, null)], "cpu_percent").current, 10);
  assert.deepEqual(summarize([pt(0, null)], "cpu_percent"), { current: null, min: null, average: null, max: null });
  assert.deepEqual(domainFor("cpu_percent", []), [0, 100]);
  assert.deepEqual(domainFor("temperature_celsius", [pt(0, 1, 46), pt(1, 1, 59)]), [40, 70]);
  assert.deepEqual(domainFor("temperature_celsius", [pt(0, 1, 52), pt(1, 1, 54)]), [40, 70]);
  assert.deepEqual(domainFor("temperature_celsius", [pt(0, 1, 2)]), [0, 30]);
  assert.deepEqual(domainFor("temperature_celsius", [pt(0, 1, null)]), [20, 80]);
  const points = [pt(0, 1), pt(100, 1), pt(400, 1)];
  assert.equal(nearestIndex(points, 0.3, { from: 0, to: 400 }), 1);
  assert.equal(nearestIndex(points, 0.9, { from: 0, to: 400 }), 2);
  assert.equal(nearestIndex(points, -2, { from: 0, to: 400 }), 0);
  assert.equal(nearestIndex([], 0.5, { from: 0, to: 1 }), null);
  assert.deepEqual(axisTimes({ from: 0, to: 100 }), [0, 50, 100]);
});
