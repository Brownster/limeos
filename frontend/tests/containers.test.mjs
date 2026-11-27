import test from "node:test";
import assert from "node:assert/strict";
import {
  actionStates, attentionReason, buildRows, counts, DEFAULT_VIEW, dockerSource, groupRows, imageLabel, jobBadge, latestJobs,
  lineCount, logNote, matchesFilter, matchesSearch, parseImage, parseView, primaryAction, serializeView, settledJobs,
  sortRows, stacksWithRunning, visibleRows, MAX_ROWS, SEARCH_MAX,
} from "../src/containers/container-model.ts";
import { decodeContainerStorage, decodeLogs, DecodeError } from "../src/containers/container-client.ts";

const NOW = 1_800_000_000;
const hex = (n) => n.toString(16).padStart(64, "0");
const container = (n, name, status, stack, patch = {}) => ({
  id: `container:${hex(n)}`, kind: "container", name, status, source: "docker", parent: stack ? `stack:${stack}` : null,
  identity: hex(n), filesystem: null, mountpoint: null, size_bytes: null, image: `lscr.io/linuxserver/${name}:latest`,
  health: null, cpu_percent: status === "running" ? n : null, memory_percent: status === "running" ? n * 2 : null, ...patch,
});
const job = (n, resource, state, patch = {}) => ({
  id: hex(1000 + n), state,
  plan: { id: hex(2000 + n), version: 1, principal: "alex", grant_revision: 1, created_at: NOW - 60,
    expires_at: NOW + 240, expected: { resource, image: `sha256:${"d".repeat(64)}`, running: true, started_at: "2026-11-24T07:00:00Z" }, ...patch },
});
const household = () => [
  container(1, "jellyfin", "running", "media", { health: "healthy" }),
  container(2, "sonarr", "running", "media"),
  container(4, "lidarr", "exited", "media"),
  container(5, "transmission", "running", "downloads", { health: "unhealthy" }),
  container(6, "jackett", "running", "downloads"),
  container(7, "audiobookshelf", "running", null),
  container(10, "watchtower", "exited", null),
  container(11, "old", "exited", "archive"),
  container(12, "looper", "restarting", null),
  { ...container(13, "x", "running", null), kind: "stack", id: "stack:x" },
];

test("needs attention: unhealthy, restart loop, dead, missing, a stopped member beside running ones, a stuck job", () => {
  const all = household().filter((r) => r.kind === "container");
  const running = stacksWithRunning(all);
  assert.deepEqual([...running].sort(), ["downloads", "media"]);
  const reason = (name, j) => attentionReason(all.find((r) => r.name === name), running, j);
  assert.equal(reason("transmission"), "Its healthcheck is failing");
  assert.equal(reason("looper"), "Docker keeps restarting it");
  assert.equal(reason("lidarr"), "Stopped while the rest of its stack runs");
  // A standalone exited container and a wholly stopped stack are deliberate stops.
  assert.equal(reason("watchtower"), null);
  assert.equal(reason("old"), null);
  assert.equal(reason("jellyfin"), null);
  assert.equal(attentionReason(container(20, "d", "dead", null), running), "Docker reports it dead");
  assert.equal(attentionReason(container(21, "m", "missing", null), running), "The host no longer reports it");
  assert.equal(attentionReason(container(22, "p", "paused", null), running), null);
  assert.equal(attentionReason(container(23, "c", "created", "media"), running), null);
  const j = all.find((r) => r.name === "jellyfin");
  assert.equal(reason("jellyfin", job(1, j.id, "needs_intervention")), "Its last operation needs attention");
  assert.equal(reason("jellyfin", job(1, j.id, "outcome_unknown")), "Its last operation needs attention");
  assert.equal(reason("jellyfin", job(1, j.id, "failed")), null);
});

test("rows: only containers, readings withheld unless current, unknown is never zero, newest job per container", () => {
  const resources = household();
  const id = resources[0].id;
  const jobs = [job(2, id, "running"), job(1, id, "succeeded")];
  const rows = buildRows(resources, jobs, true);
  assert.equal(rows.length, 9);
  const jellyfin = rows.find((r) => r.name === "jellyfin");
  assert.equal(jellyfin.job.state, "running");
  assert.equal(jellyfin.stack, "media");
  assert.equal(jellyfin.cpu, 1);
  assert.equal(rows.find((r) => r.name === "lidarr").cpu, null);
  assert.equal(buildRows([container(3, "z", "running", null, { cpu_percent: 0 })], [], true)[0].cpu, 0);
  assert.equal(buildRows([container(3, "z", "running", null, { cpu_percent: NaN })], [], true)[0].cpu, null);
  for (const row of buildRows(resources, [], false)) assert.equal(row.cpu === null && row.memory === null, true);
  assert.equal(latestJobs(jobs).get(id).state, "running");
});

test("filters and counts: running, stopped (anything not running), needs attention", () => {
  const rows = buildRows(household(), [], true);
  const names = (filter) => rows.filter((r) => matchesFilter(r, filter)).map((r) => r.name).sort();
  assert.deepEqual(names("running"), ["audiobookshelf", "jackett", "jellyfin", "sonarr", "transmission"]);
  assert.deepEqual(names("stopped"), ["lidarr", "looper", "old", "watchtower"]);
  assert.deepEqual(names("attention"), ["lidarr", "looper", "transmission"]);
  assert.equal(names("all").length, 9);
  assert.deepEqual(counts(rows), { all: 9, running: 5, stopped: 4, attention: 3 });
});

test("search matches name, image and stack, case-insensitively and trimmed", () => {
  const rows = buildRows(household(), [], true);
  const hits = (q) => rows.filter((r) => matchesSearch(r, q)).map((r) => r.name).sort();
  assert.deepEqual(hits("SON"), ["sonarr"]);
  assert.deepEqual(hits(" media "), ["jellyfin", "lidarr", "sonarr"]);
  assert.deepEqual(hits("linuxserver/jackett"), ["jackett"]);
  assert.equal(hits("").length, 9);
  assert.deepEqual(hits("nothing-like-this"), []);
});

test("sorting: name A–Z (numeric aware), CPU and memory highest first with unknown last", () => {
  const rows = buildRows([
    container(3, "web10", "running", null, { cpu_percent: 5, memory_percent: 1 }),
    container(4, "web2", "running", null, { cpu_percent: 50, memory_percent: null }),
    container(5, "Alpha", "exited", null),
    container(6, "beta", "running", null, { cpu_percent: 5, memory_percent: 9 }),
  ], [], true);
  assert.deepEqual(sortRows(rows, "name").map((r) => r.name), ["Alpha", "beta", "web2", "web10"]);
  assert.deepEqual(sortRows(rows, "cpu").map((r) => r.name), ["web2", "beta", "web10", "Alpha"]);
  assert.deepEqual(sortRows(rows, "memory").map((r) => r.name), ["beta", "web10", "Alpha", "web2"]);
  assert.deepEqual(visibleRows(rows, { filter: "running", query: "web", sort: "cpu" }).map((r) => r.name), ["web2", "web10"]);
});

test("grouping: stacks A–Z then containers outside any stack; counts cover the whole stack", () => {
  const all = buildRows(household(), [], true);
  const shown = all.filter((r) => matchesFilter(r, "attention"));
  const groups = groupRows(shown, all);
  assert.deepEqual(groups.map((g) => g.label), ["downloads", "media", "Not in a stack"]);
  const media = groups.find((g) => g.stack === "media");
  assert.deepEqual([media.rows.length, media.running, media.total], [1, 2, 3]);
  const standalone = groups.at(-1);
  assert.equal(standalone.stack, null);
  assert.deepEqual([standalone.running, standalone.total], [1, 3]);
  assert.deepEqual(groupRows([], all), []);
});

test("image references: registry, repository, tag, digest, Docker Hub library and bare image IDs", () => {
  assert.deepEqual(parseImage("lscr.io/linuxserver/sonarr:latest"),
    { raw: "lscr.io/linuxserver/sonarr:latest", kind: "reference", registry: "lscr.io", repository: "linuxserver/sonarr", tag: "latest", digest: null });
  const pinned = parseImage(`ghcr.io/home-assistant/home-assistant:2024.7@sha256:${"a".repeat(64)}`);
  assert.deepEqual([pinned.registry, pinned.repository, pinned.tag, pinned.digest], ["ghcr.io", "home-assistant/home-assistant", "2024.7", `sha256:${"a".repeat(64)}`]);
  assert.deepEqual([parseImage("localhost:5000/app:dev").registry, parseImage("localhost:5000/app:dev").tag], ["localhost:5000", "dev"]);
  assert.deepEqual([parseImage("localhost/app").registry, parseImage("localhost/app").tag], ["localhost", null]);
  assert.deepEqual([parseImage("jellyfin/jellyfin").registry, parseImage("jellyfin/jellyfin").tag], [null, null]);
  assert.equal(parseImage("docker.io/library/nginx:1.27").repository, "nginx");
  assert.equal(parseImage("library/redis").repository, "redis");
  assert.equal(imageLabel(parseImage("nginx")), "nginx");
  assert.equal(imageLabel(parseImage("lscr.io/linuxserver/sonarr:latest")), "linuxserver/sonarr:latest");
  const id = parseImage(`sha256:${"9".repeat(64)}`);
  assert.equal(id.kind, "id");
  assert.equal(imageLabel(id), "image 999999999999");
  assert.equal(parseImage("f".repeat(64)).kind, "id");
  assert.equal(parseImage(null).kind, "unknown");
  assert.equal(imageLabel(parseImage("")), "Unknown image");
});

test("URL state: round trip, defaults omitted, unknown values ignored, search bounded", () => {
  assert.deepEqual(parseView(""), DEFAULT_VIEW);
  assert.equal(serializeView(DEFAULT_VIEW), "");
  const view = { filter: "attention", query: "son arr", group: true, sort: "memory", detail: "jellyfin" };
  const search = serializeView(view);
  assert.equal(search, "?filter=attention&q=son+arr&group=stack&sort=memory&container=jellyfin");
  assert.deepEqual(parseView(search), view);
  assert.deepEqual(parseView("?filter=bogus&sort=evil&group=yes&container="), DEFAULT_VIEW);
  assert.equal(parseView(`?q=${"x".repeat(500)}`).query.length, SEARCH_MAX);
  assert.equal(parseView(`?container=${"y".repeat(200)}`).detail, null);
  assert.equal(serializeView({ ...DEFAULT_VIEW, query: "   " }), "");
  assert.equal(MAX_ROWS >= 64, true);
});

test("docker source: fresh, stale by age, unavailable, missing", () => {
  const source = (patch) => [{ source: "docker", state: "fresh", sampled_at: NOW - 5, max_age_seconds: 90, warnings: [], ...patch }];
  assert.equal(dockerSource(source({}), NOW).state, "fresh");
  assert.equal(dockerSource(source({ sampled_at: NOW - 600 }), NOW).state, "stale");
  assert.equal(dockerSource(source({ sampled_at: null }), NOW).state, "stale");
  assert.equal(dockerSource(source({ state: "unavailable" }), NOW).state, "unavailable");
  assert.equal(dockerSource([], NOW).state, "missing");
  assert.deepEqual(dockerSource(source({ warnings: ["The Engine holds 70 containers"] }), NOW).warnings, ["The Engine holds 70 containers"]);
});

test("actions: one contextual action, every action says why it is unavailable", () => {
  const [running] = buildRows([container(1, "a", "running", null)], [], true);
  const [stopped] = buildRows([container(2, "b", "exited", null)], [], true);
  const [looping] = buildRows([container(3, "c", "restarting", null)], [], true);
  assert.equal(primaryAction(running), "restart");
  assert.equal(primaryAction(stopped), "start");
  assert.equal(primaryAction(looping), null);
  const why = (row) => Object.fromEntries(actionStates(row).map((s) => [s.action, s.disabled]));
  assert.deepEqual(why(running), { start: "It is already running", stop: null, restart: null });
  assert.deepEqual(why(stopped), { start: null, stop: "It is exited, not running", restart: "It is exited, not running" });
  assert.equal(why(looping).start, "It can't be started while restarting");
  const busy = { ...running, job: job(1, running.resource.id, "queued") };
  assert.deepEqual(Object.values(why(busy)), Array(3).fill("An operation on it is in progress"));
  const stuck = { ...running, job: job(1, running.resource.id, "needs_intervention") };
  assert.equal(why(stuck).restart, "Its last operation must be resolved first");
});

test("a row's job: in flight, stuck, recently failed; settled jobs are noticed once", () => {
  const id = `container:${hex(1)}`;
  assert.deepEqual(jobBadge(job(1, id, "queued"), NOW), { label: "Restart queued", tone: "info" });
  assert.deepEqual(jobBadge(job(1, id, "running", { operation: "stop" }), NOW), { label: "Stopping", tone: "info" });
  assert.deepEqual(jobBadge(job(1, id, "verifying", { operation: "start" }), NOW), { label: "Checking the start", tone: "info" });
  assert.equal(jobBadge(job(1, id, "needs_intervention"), NOW).tone, "danger");
  assert.deepEqual(jobBadge(job(1, id, "failed"), NOW), { label: "Restart failed", tone: "warning" });
  assert.equal(jobBadge(job(1, id, "failed"), NOW + 3600), null);
  assert.equal(jobBadge(job(1, id, "succeeded"), NOW), null);
  assert.equal(jobBadge(null, NOW), null);
  const before = [job(1, id, "running"), job(2, id, "queued"), job(3, id, "succeeded")];
  const after = [job(1, id, "succeeded"), job(2, id, "running"), job(3, id, "succeeded")];
  assert.deepEqual(settledJobs(before, after), [hex(1001)]);
  assert.deepEqual(settledJobs(after, after), []);
});

test("logs: line counts and the 'showing the last N lines' note", () => {
  assert.equal(lineCount(""), 0);
  assert.equal(lineCount("a\nb\n"), 2);
  assert.equal(lineCount("a\nb"), 2);
  const text = (n) => Array.from({ length: n }, (_, i) => `line ${i}`).join("\n") + "\n";
  assert.equal(logNote(100, text(100), false), "Showing the last 100 lines.");
  assert.equal(logNote(200, text(3), false), "Showing all 3 lines; the container has written fewer than 200.");
  assert.equal(logNote(200, text(120), true), "Showing the newest 120 of the last 200 lines.");
  assert.match(logNote(50, "", false), /written nothing/);
});

test("decoders refuse what the host would never send", () => {
  const resource = `container:${hex(1)}`;
  assert.deepEqual(decodeLogs({ resource, text: "ok\n", truncated: false }, resource), { resource, text: "ok\n", truncated: false });
  for (const bad of [
    { resource: `container:${hex(2)}`, text: "", truncated: false },
    { resource, text: 3, truncated: false },
    { resource, text: "", truncated: "no" },
    { resource, text: "", truncated: false, extra: 1 },
    { resource, text: "x".repeat(24 * 1024 + 1), truncated: true },
    null,
  ]) assert.throws(() => decodeLogs(bad, resource), DecodeError);

  const view = {
    inventory: { version: 1, engine_id: "engine", observed_at: NOW, containers: [{
      container: { resource, image: `sha256:${"d".repeat(64)}`, started_at: "2026-11-24T07:00:00Z", running: true },
      mounts: [
        { source: { kind: "bind", path: "/srv/config" }, destination: "/config", writable: true },
        { source: { kind: "volume", path: "/var/lib/docker/volumes/v/_data", name: "v", driver: "local" }, destination: "/data", writable: false },
      ],
    }] },
    digest: "f".repeat(64),
  };
  assert.deepEqual(decodeContainerStorage(view), view);
  const variant = (change) => { const copy = structuredClone(view); change(copy); return copy; };
  for (const bad of [
    variant((v) => { v.inventory.version = 2; }),
    variant((v) => { v.extra = true; }),
    variant((v) => { v.inventory.containers[0].container.resource = "container:short"; }),
    variant((v) => { v.inventory.containers[0].mounts[0].source.kind = "tmpfs"; }),
    variant((v) => { v.inventory.containers[0].mounts[0].source.path = "relative"; }),
    variant((v) => { v.inventory.containers[0].mounts[1].source.name = "bad name"; }),
    variant((v) => { v.inventory.containers[0].mounts[0].writable = "yes"; }),
    variant((v) => { v.inventory.containers.push(structuredClone(v.inventory.containers[0])); }),
    variant((v) => { v.inventory.containers = Array(65).fill(v.inventory.containers[0]); }),
  ]) assert.throws(() => decodeContainerStorage(bad), DecodeError);
});
