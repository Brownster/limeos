import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, createMountApi, StorageMountController, StorageMounts, disk, proposal, approval, job, fakeApi, apiError } from "./helpers/storage-mounts.mjs";

function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
const create = (api = fakeApi(), now = () => 105, expired = () => {}, key = () => "fixture-key") => new StorageMountController("fixture-csrf", expired, api, now, key);
async function ready(api = fakeApi(), now, expired, key) { const controller = create(api, now, expired, key); await controller.start(); await controller.preview(); return controller; }

test("initial catalog and history reads never create plans or approvals", async () => {
  let posts = 0; const controller = create(fakeApi({ plan: async () => { posts++; return proposal(); }, approve: async () => { posts++; return approval(); } }));
  await controller.start(); assert.equal(controller.getSnapshot().target, "data"); assert.deepEqual(controller.getSnapshot().jobs, []); assert.equal(posts, 0); controller.dispose();
});
test("catalog and history each coalesce overlapping reads", async () => {
  const catalog = deferred(), history = deferred(); let catalogs = 0, histories = 0;
  const controller = create(fakeApi({ targets: () => { catalogs++; return catalog.promise; }, jobs: () => { histories++; return history.promise; } }));
  const a = controller.refreshTargets(), b = controller.refreshJobs(); assert.equal(controller.refreshTargets(), a); assert.equal(controller.refreshJobs(), b);
  await Promise.resolve(); assert.equal(catalogs, 1); assert.equal(histories, 1); catalog.resolve({ targets: [disk] }); history.resolve([]); await Promise.all([a, b]); controller.dispose();
});
test("failed reads remove previous rows and never report an empty successful catalog", async () => {
  const api = fakeApi({ jobs: async () => [job()] }); const controller = create(api); await controller.start();
  api.targets = async () => { throw new Error("fixture-secret"); }; api.jobs = api.targets; await controller.refreshTargets(); await controller.refreshJobs();
  const state = controller.getSnapshot(); assert.deepEqual(state.targets, []); assert.deepEqual(state.jobs, []); assert.ok(state.catalogError); assert.ok(state.historyError); assert.doesNotMatch(JSON.stringify(state), /fixture-secret/); controller.dispose();
});
test("catalog count, duplicate identities and unsupported disks are refused", async () => {
  const many = Array.from({ length: 33 }, (_, i) => ({ ...disk, id: `data-${i}`, filesystem_uuid: `uuid-${i}`, mountpoint: `/mnt/data-${i}` }));
  for (const targets of [null, many, [disk, disk], [{ ...disk, filesystem: "xfs" }], [{ ...disk, role: "parity" }]]) {
    const controller = create(fakeApi({ targets: async () => ({ targets }) })); await controller.start(); assert.equal(controller.getSnapshot().target, ""); assert.ok(controller.getSnapshot().catalogError); controller.dispose();
  }
});
test("malformed or duplicate history rows cannot render successful empty history", async () => {
  const many = Array.from({ length: 33 }, (_, i) => { const p = proposal(); p.plan.id = (i + 100).toString(16).padStart(64, "0"); p.plan.effect.action = (i + 10).toString(16).padStart(64, "0"); return job("queued", p); });
  for (const jobs of [{}, [job(), job()], many, [{ ...job(), state: "not-a-job-state" }], [{ ...job(), id: "4".repeat(64) }]]) {
    const controller = create(fakeApi({ jobs: async () => jobs })); await controller.start(); assert.ok(controller.getSnapshot().historyError); assert.deepEqual(controller.getSnapshot().jobs, []); controller.dispose();
  }
});
test("preview requires an admitted selected target and coalesces double clicks", async () => {
  let calls = 0; const wait = deferred(); const controller = create(fakeApi({ plan: () => { calls++; return wait.promise; } }));
  await controller.preview(); assert.equal(calls, 0); await controller.start(); controller.setTarget("unknown"); assert.equal(controller.getSnapshot().target, "data");
  const pending = controller.preview(); await controller.preview(); assert.equal(calls, 1); wait.resolve(proposal()); await pending; assert.ok(controller.getSnapshot().proposal); controller.dispose();
});
test("wrong target, operation and plan identity refuse approval readiness", async () => {
  for (const change of [p => p.plan.effect.device.id = "other", p => p.plan.effect.operation = "storage.unmount", p => p.digest = "bad", p => p.plan.effect.action = "bad"]) {
    const value = proposal(); change(value); const controller = await ready(fakeApi({ plan: async () => value })); assert.equal(controller.getSnapshot().proposal, null); assert.ok(controller.getSnapshot().error); controller.dispose();
  }
});
test("disposed preview cannot populate a restarted controller", async () => {
  const wait = deferred(); let calls = 0; const controller = create(fakeApi({ plan: () => ++calls === 1 ? wait.promise : Promise.resolve(proposal()) }));
  await controller.start(); const old = controller.preview(); controller.dispose(); await controller.start(); wait.resolve(proposal()); await old; assert.equal(controller.getSnapshot().proposal, null); await controller.preview(); assert.ok(controller.getSnapshot().proposal); controller.dispose();
});
test("abandoned approval cannot queue after it completes", async () => {
  const wait = deferred(); let queues = 0, signal; const controller = await ready(fakeApi({ approve: (_, __, value) => { signal = value; return wait.promise; }, queue: async () => { queues++; return job(); } }));
  const pending = controller.confirm(); controller.dispose(); assert.equal(signal.aborted, true); wait.resolve(approval()); await pending; assert.equal(queues, 0); assert.equal(controller.getSnapshot().proposal, null);
});
test("disposal during approval-state publication also prevents queue", async () => {
  let queues = 0; const controller = await ready(fakeApi({ queue: async () => { queues++; return job(); } }));
  const stop = controller.subscribe(() => { if (controller.getSnapshot().retryAvailable) controller.dispose(); }); await controller.confirm(); assert.equal(queues, 0); stop();
});
test("confirm guards duplicate clicks before React can rerender", async () => {
  const wait = deferred(); let approvals = 0, queues = 0; const controller = await ready(fakeApi({ approve: () => { approvals++; return wait.promise; }, queue: async () => { queues++; return job(); } }));
  const pending = controller.confirm(); await controller.confirm(); wait.resolve(approval()); await pending; assert.equal(approvals, 1); assert.equal(queues, 1); assert.equal(controller.getSnapshot().reviewOpen, false); controller.dispose();
});
test("lost queue response retries the exact proposal, approval and key", async () => {
  let approvals = 0, keys = 0; const calls = []; const api = fakeApi({ approve: async () => { approvals++; return approval(); }, queue: async (...args) => { calls.push(args.slice(0, 4)); if (calls.length === 1) throw new Error("fixture-secret"); return job(); } });
  const controller = await ready(api, () => 105, undefined, () => `fixture-key-${++keys}`); await controller.confirm(); assert.equal(controller.getSnapshot().retryAvailable, true); assert.equal(controller.getSnapshot().reviewOpen, true);
  assert.doesNotMatch(controller.getSnapshot().error, /fixture-secret/); await controller.confirm(); assert.deepEqual(calls[0], calls[1]); assert.equal(approvals, 1); assert.equal(keys, 1); assert.equal(controller.getSnapshot().retryAvailable, false); controller.dispose();
});
test("already consumed retry remains exact after approval expiry", async () => {
  let now = 105, approvals = 0, queues = 0; const controller = await ready(fakeApi({ approve: async () => { approvals++; return approval(); }, queue: async () => { if (++queues === 1) throw new Error("lost reply"); return job(); } }), () => now);
  await controller.confirm(); now = 500; await controller.confirm(); assert.equal(approvals, 1); assert.equal(queues, 2); assert.equal(controller.getSnapshot().retryAvailable, false); controller.dispose();
});
test("first confirmation refuses expired, future or invalid browser time", async () => {
  for (const now of [400, 99, NaN, Infinity]) {
    let approvals = 0; const controller = await ready(fakeApi({ approve: async () => { approvals++; return approval(); } }), () => now); await controller.confirm(); assert.equal(approvals, 0); assert.match(controller.getSnapshot().error, /expired/); controller.dispose();
  }
});
test("invalid or expired approval never generates a queue tuple", async () => {
  for (const value of [{ ...approval(), token: "bad" }, { ...approval(), expires_at: 105 }]) {
    let queues = 0; const controller = await ready(fakeApi({ approve: async () => value, queue: async () => { queues++; return job(); } })); await controller.confirm(); assert.equal(queues, 0); assert.equal(controller.getSnapshot().retryAvailable, false); controller.dispose();
  }
});
test("mismatched queue result retains uncertainty and exact retry", async () => {
  const returned = job(); returned.plan.id = "4".repeat(64); const controller = await ready(fakeApi({ queue: async () => returned })); await controller.confirm(); assert.equal(controller.getSnapshot().retryAvailable, true); assert.equal(controller.getSnapshot().reviewOpen, true); assert.ok(controller.getSnapshot().error); controller.dispose();
});
test("an old history read cannot erase post-queue history", async () => {
  const wait = deferred(); let reads = 0; const controller = create(fakeApi({ jobs: () => ++reads === 1 ? wait.promise : Promise.resolve([job()]) }));
  await controller.refreshTargets(); const old = controller.refreshJobs(); await Promise.resolve(); await controller.preview(); await controller.confirm(); assert.deepEqual(controller.getSnapshot().jobs, [job()]); wait.resolve([]); await old; assert.deepEqual(controller.getSnapshot().jobs, [job()]); assert.equal(reads, 2); controller.dispose();
});
test("cancel invalidates old reads and waits for committed job state", async () => {
  const wait = deferred(); let reads = 0, cancels = 0; const api = fakeApi({ jobs: () => { reads++; return reads === 1 ? Promise.resolve([job()]) : reads === 2 ? wait.promise : Promise.resolve([job("canceled")]); }, cancel: async () => { cancels++; } });
  const controller = create(api); await controller.start(); const old = controller.refreshJobs(); await Promise.resolve(); await controller.cancel(job().id); assert.equal(controller.getSnapshot().jobs[0].state, "canceled"); wait.resolve([job()]); await old; assert.equal(controller.getSnapshot().jobs[0].state, "canceled"); assert.equal(cancels, 1); controller.dispose();
});
test("cancel failure stays visible after a successful history refresh", async () => {
  const controller = create(fakeApi({ jobs: async () => [job()], cancel: async () => { throw apiError("conflict"); } })); await controller.start(); await controller.cancel(job().id); assert.match(controller.getSnapshot().historyError, /state changed/); assert.doesNotMatch(controller.getSnapshot().historyError, /fixture-private/); assert.equal(controller.getSnapshot().jobs[0].state, "queued"); controller.dispose();
});
test("only an explicitly selected queued job can request cancellation", async () => {
  let cancels = 0; const controller = create(fakeApi({ jobs: async () => [job("running")], cancel: async () => { cancels++; } })); await controller.start(); await controller.cancel(job().id); await controller.cancel("unknown"); assert.equal(cancels, 0); controller.dispose();
});
test("aborting queue never fabricates job cancellation or a success", async () => {
  const wait = deferred(); let signal, cancels = 0; const controller = await ready(fakeApi({ queue: (_, __, ___, ____, value) => { signal = value; return wait.promise; }, cancel: async () => { cancels++; } }));
  const pending = controller.confirm(); await Promise.resolve(); controller.dispose(); assert.equal(signal.aborted, true); wait.resolve(job()); await pending; assert.equal(cancels, 0); assert.deepEqual(controller.getSnapshot().jobs, []); assert.equal(controller.getSnapshot().retryAvailable, false);
});
test("closing a pending submission cannot imply cancellation", async () => {
  const wait = deferred(); const controller = await ready(fakeApi({ approve: () => wait.promise })); const pending = controller.confirm(); controller.closeReview(); assert.equal(controller.getSnapshot().reviewOpen, true); wait.resolve(approval()); await pending; assert.equal(controller.getSnapshot().reviewOpen, false); controller.dispose();
});
test("a closed uncertain submission can reopen without changing its tuple", async () => {
  let approvals = 0; const controller = await ready(fakeApi({ approve: async () => { approvals++; return approval(); }, queue: async () => { throw new Error("lost reply"); } }));
  await controller.confirm(); controller.closeReview(); assert.equal(controller.getSnapshot().reviewOpen, false); controller.reopenReview(); assert.equal(controller.getSnapshot().reviewOpen, true); await controller.confirm(); assert.equal(approvals, 1); controller.dispose();
});
test("401 and 403 from every operation clear authority-bound UI state", async () => {
  for (const code of ["unauthenticated", "forbidden"]) for (const phase of ["targets", "jobs", "plan", "approve", "queue", "cancel"]) {
    let expired = 0; const api = fakeApi({ jobs: async () => [job()] }); const controller = await ready(api, () => 105, () => { expired++; }); api[phase] = async () => { throw apiError(code); };
    if (phase === "targets") await controller.refreshTargets(); else if (phase === "jobs") await controller.refreshJobs(); else if (phase === "plan") await controller.preview(); else if (phase === "cancel") await controller.cancel(job().id); else await controller.confirm();
    const state = controller.getSnapshot(); assert.equal(state.proposal, null, phase); assert.deepEqual(state.jobs, [], phase); assert.deepEqual(state.targets, [], phase); assert.equal(state.retryAvailable, false); assert.equal(state.expired, code === "unauthenticated"); assert.equal(state.denied, code === "forbidden"); assert.equal(expired, code === "unauthenticated" ? 1 : 0); assert.doesNotMatch(JSON.stringify(state), /fixture-private/);
    await controller.refreshJobs(); await controller.preview(); assert.equal(expired, code === "unauthenticated" ? 1 : 0); controller.dispose();
  }
});
test("disposal before deferred startup launches no request", async () => {
  let reads = 0; const api = fakeApi({ targets: async () => { reads++; return { targets: [disk] }; }, jobs: async () => { reads++; return []; } }); const controller = create(api); const pending = controller.start(); controller.dispose(); await pending; assert.equal(reads, 0);
});
test("old catalog and auth failures cannot overwrite a restarted owner", async () => {
  const catalog = deferred(), history = deferred(); let catalogs = 0, histories = 0, expired = 0;
  const controller = create(fakeApi({ targets: () => ++catalogs === 1 ? catalog.promise : Promise.resolve({ targets: [disk] }), jobs: () => ++histories === 1 ? history.promise : Promise.resolve([job()]) }), () => 105, () => { expired++; });
  const old = controller.start(); await Promise.resolve(); controller.dispose(); await controller.start();
  catalog.resolve({ targets: [{ ...disk, id: "old-data" }] }); history.reject(apiError("unauthenticated")); await old;
  assert.equal(controller.getSnapshot().target, "data"); assert.deepEqual(controller.getSnapshot().jobs, [job()]); assert.equal(expired, 0); assert.equal(controller.getSnapshot().expired, false); controller.dispose();
});
test("unknown structured codes cannot become inherited message properties", async () => {
  for (const code of ["__proto__", "constructor"]) {
    const controller = await ready(fakeApi({ approve: async () => { throw apiError(code); } })); await controller.confirm(); assert.match(controller.getSnapshot().error, /Mount operations are unavailable/); assert.doesNotMatch(controller.getSnapshot().error, /native code|fixture-private|object Object/); controller.dispose();
  }
});
test("loading-state disposal returns the original read promise without a request", async () => {
  for (const method of ["refreshTargets", "refreshJobs"]) {
    let reads = 0; const controller = create(fakeApi({ targets: async () => { reads++; return { targets: [] }; }, jobs: async () => { reads++; return []; } }));
    let fired = false; const unsubscribe = controller.subscribe(() => { if (!fired) { fired = true; controller.dispose(); } });
    const pending = controller[method](); assert.ok(pending instanceof Promise); await pending; assert.equal(reads, 0); unsubscribe();
  }
});
test("disposal during pending-state publication prevents the first POST", async () => {
  for (const phase of ["preview", "confirm", "cancel"]) {
    let posts = 0; const api = fakeApi({ jobs: async () => [job()] }); const controller = await ready(api);
    api.plan = async () => { posts++; return proposal(); }; api.approve = async () => { posts++; return approval(); }; api.cancel = async () => { posts++; };
    let fired = false; const stop = controller.subscribe(() => { const state = controller.getSnapshot(); if (!fired && (state.pending || state.canceling)) { fired = true; controller.dispose(); } });
    await controller[phase](...(phase === "cancel" ? [job().id] : [])); assert.equal(posts, 0, phase); stop();
  }
});
test("the outer component keys a new controller by CSRF session", () => {
  assert.notEqual(StorageMounts({ csrf: "first", expired() {} }).key, StorageMounts({ csrf: "second", expired() {} }).key);
  assert.equal(StorageMounts({ csrf: "first", expired() {} }).key, StorageMounts({ csrf: "first", expired() {} }).key);
});
test("initial markup exposes loading rather than a false empty history", () => {
  const html = renderToStaticMarkup(React.createElement(StorageMounts, { csrf: "fixture-private-csrf", expired() {} })); assert.match(html, /Reading recent mount jobs/); assert.doesNotMatch(html, /No recent mount jobs|fixture-private-csrf/); assert.match(html, /Review nonpersistent mount/);
});
test("storage requests preserve fixed routes, CSRF, exact replay and cancellation", async () => {
  const calls = [], api = createMountApi(async (url, init) => { calls.push([url, init]); return url.endsWith("/cancel") ? new Response(null, { status: 204 }) : Response.json(url.endsWith("/targets") ? { targets: [disk] } : url.endsWith("/approval") ? approval() : url.endsWith("/plans") ? proposal() : url.endsWith("/jobs") && init.method === "POST" ? job() : []); });
  const signal = new AbortController().signal; await api.targets(signal); await api.plan("data", "csrf", signal); await api.approve(proposal(), "csrf", signal); await api.queue(proposal(), approval().token, "same-key", "csrf", signal); await api.queue(proposal(), approval().token, "same-key", "csrf", signal); await api.jobs(signal); await api.cancel(job().id, "csrf", signal);
  for (const [url, options] of calls) { assert.match(url, /^\/api\/v1\/storage\/mounts\//); assert.equal(options.credentials, "same-origin"); assert.equal(options.cache, "no-store"); assert.equal(options.redirect, "error"); assert.ok(options.signal instanceof AbortSignal); if (options.method === "POST") assert.equal(options.headers["X-CSRF-Token"], "csrf"); assert.doesNotMatch(url, /same-key/); }
  assert.equal(calls[0][1].body, undefined); assert.equal(calls[5][1].body, undefined); assert.equal(calls[3][1].body, calls[4][1].body);
});
test("transport 401 and 403 take precedence over malformed bodies", async () => {
  for (const status of [401, 403]) {
    const api = createMountApi(async () => new Response("fixture-private {{{", { status })); await assert.rejects(api.targets(), error => error instanceof ApiError && error.detail.code === (status === 401 ? "unauthenticated" : "forbidden") && !error.message.includes("fixture-private"));
  }
});
test("bounded structured error codes survive without raw proxy diagnostics", async () => {
  const api = createMountApi(async () => Response.json({ code: "storage_mount_limit", message: "collection limit", retry: false, audit_id: "fixture-audit" }, { status: 503 })); await assert.rejects(api.targets(), error => error instanceof ApiError && error.detail.code === "storage_mount_limit");
  const proxy = createMountApi(async () => new Response("fixture-private", { status: 503 })); await assert.rejects(proxy.targets(), error => error instanceof ApiError && !error.message.includes("fixture-private"));
});
test("pre-aborted storage transport sends no request", async () => {
  const signal = new AbortController(); signal.abort(); let calls = 0; const api = createMountApi(async () => { calls++; return Response.json({ targets: [] }); }); await assert.rejects(api.targets(signal.signal)); assert.equal(calls, 0);
});
test("caller abort cancels an already stalled storage body", async () => {
  let cancelled = false; const signal = new AbortController(); const api = createMountApi(async () => new Response(new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("{")); }, cancel() { cancelled = true; } })));
  const pending = api.targets(signal.signal); await Promise.resolve(); signal.abort(); await assert.rejects(pending); assert.equal(cancelled, true);
});
test("storage deadline cancels stalled body without accepting partial JSON", async () => {
  const timeout = globalThis.setTimeout, clear = globalThis.clearTimeout; let deadline, cancelled = false, cleared = false;
  globalThis.setTimeout = (fn, ms) => { assert.equal(ms, 12000); deadline = fn; return 1; }; globalThis.clearTimeout = () => { cleared = true; };
  try { const api = createMountApi(async () => new Response(new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("{")); }, cancel() { cancelled = true; } }))); const pending = api.targets(); await Promise.resolve(); deadline(); await assert.rejects(pending); assert.equal(cancelled, true); assert.equal(cleared, true); } finally { globalThis.setTimeout = timeout; globalThis.clearTimeout = clear; }
});
test("storage catalog and history byte limits are inclusive", async () => {
  const exact = createMountApi(async () => new Response(JSON.stringify({ targets: [] }).padEnd(32768))); assert.deepEqual(await exact.targets(), { targets: [] });
  const big = createMountApi(async () => new Response(JSON.stringify({ targets: [] }).padEnd(32769))); await assert.rejects(big.targets());
  const history = createMountApi(async () => new Response("[]".padEnd(1024 * 1024))); assert.deepEqual(await history.jobs(), []);
  const tooBig = createMountApi(async () => new Response("[]".padEnd(1024 * 1024 + 1))); await assert.rejects(tooBig.jobs());
});
test("malformed UTF-8, JSON and empty bodies yield no typed response", async () => {
  for (const response of [new Response(Uint8Array.of(255)), new Response("{"), new Response(null)]) { const api = createMountApi(async () => response); await assert.rejects(api.targets()); }
});
test("oversized queue request refuses before fetch and does not expose approval", async () => {
  let calls = 0; const value = proposal(); value.padding = "x".repeat(32768); const api = createMountApi(async () => { calls++; return Response.json(job()); }); await assert.rejects(api.queue(value, approval().token, "key", "csrf")); assert.equal(calls, 0);
});
