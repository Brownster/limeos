import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApiError, healthPolicyClient, validHealthPolicy, HealthPolicyController, HealthPolicy, HealthPolicyForm } from "./helpers/automation.mjs";

const definition = () => ({ fail_threshold: 2, disabled: [], required_mounts: [], silences: [] });
const view = (revision = 0, policy = definition()) => ({ revision, definition: policy, updated: revision === 0 ? null : 200 });
const error = code => new ApiError({ code, message: `Fixture ${code}`, retry: false, audit_id: "fixture" });
const pending = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const silence = () => ({ kind: "mount", key: "mount:/mnt/Media", created_at: "2026-10-20T12:00:00.123456Z", expires_at: "2026-10-20T13:00:00+00:00", reason: "disk work" });

test("policy client preserves literal paths, metadata, explicit revisions and GET-only reads", async () => {
  const previous = globalThis.fetch; const calls = []; const policy = { ...definition(), required_mounts: ["/mnt/Media", "/mnt/media"], silences: [silence()] };
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return new Response(JSON.stringify(init.method === "POST" ? view(1, policy) : view()), { status: 200 }); };
  try {
    assert.deepEqual(await healthPolicyClient.read(), view());
    assert.deepEqual(await healthPolicyClient.replace({ revision: 0, definition: policy }, "csrf"), view(1, policy));
    assert.equal(calls[0].url, "/api/v1/automation/health-policy"); assert.equal(calls[0].init.method, undefined);
    assert.equal(calls[0].init.cache, "no-store"); assert.equal(calls[0].init.credentials, "same-origin"); assert.ok(calls[0].init.signal instanceof AbortSignal);
    assert.deepEqual(JSON.parse(calls[1].init.body), { revision: 0, definition: policy }); assert.equal(calls[1].init.headers["X-CSRF-Token"], "csrf");
  } finally { globalThis.fetch = previous; }
});

test("policy client refuses invalid input, malformed output, excessive bytes and unrelated success", async () => {
  for (const policy of [{ ...definition(), fail_threshold: 100 }, { ...definition(), required_mounts: ["/x", "/x"] }, { ...definition(), silences: [{ ...silence(), expires_at: "2026-10-20T13:00:00.1Z" }] }, { ...definition(), silences: [{ ...silence(), created_at: "2100-02-29T00:00:00Z" }] }, { ...definition(), owner: "injected" }]) assert.equal(validHealthPolicy(policy), false);
  assert.equal(validHealthPolicy({ ...definition(), silences: [{ ...silence(), expires_at: "2026-10-20T13:00:00.000000Z" }] }), true);
  const previous = globalThis.fetch; let calls = 0;
  globalThis.fetch = async () => { calls++; return new Response(JSON.stringify(view())); };
  try {
    await assert.rejects(healthPolicyClient.replace({ revision: 0, definition: { ...definition(), fail_threshold: 100 } }, "csrf")); assert.equal(calls, 0);
    for (const result of [{ ...view(), owner: "injected" }, view(0, { ...definition(), fail_threshold: 3 }), view(1, { ...definition(), required_mounts: ["relative"] }), "x".repeat(70_000)]) {
      globalThis.fetch = async () => new Response(typeof result === "string" ? result : JSON.stringify(result));
      await assert.rejects(healthPolicyClient.read());
    }
    globalThis.fetch = async () => new Response(JSON.stringify(view(5)));
    await assert.rejects(healthPolicyClient.replace({ revision: 0, definition: definition() }, "csrf"));
    const malformed = new TextEncoder().encode(JSON.stringify(view(1, { ...definition(), required_mounts: ["/fixture"] })));
    const position = new TextDecoder().decode(malformed).indexOf("fixture"); malformed[position] = 255;
    const replaced = JSON.parse(new TextDecoder().decode(malformed));
    assert.notEqual(replaced.definition.required_mounts[0], "/fixture"); assert.equal(validHealthPolicy(replaced.definition), true, "replacement decoding would admit a different literal path");
    globalThis.fetch = async () => new Response(malformed);
    await assert.rejects(healthPolicyClient.read(), "invalid UTF-8 cannot change a literal path");
    // JSON property order carries no authority or policy meaning.
    globalThis.fetch = async () => new Response(JSON.stringify({ updated: null, definition: { silences: [], required_mounts: [], disabled: [], fail_threshold: 2 }, revision: 0 }));
    assert.equal((await healthPolicyClient.read()).revision, 0);
  } finally { globalThis.fetch = previous; }
});

test("lost save response is reconciled by GET with one mutation and current revision", async () => {
  let saved = view(), reads = 0, writes = 0; const policy = { ...definition(), fail_threshold: 3 };
  const controller = new HealthPolicyController(() => assert.fail("active session"), {
    read: async () => { reads++; return saved; },
    replace: async (input, csrf) => { writes++; assert.equal(input.revision, 0); assert.equal(csrf, "csrf"); saved = view(1, input.definition); throw error("unavailable"); },
  });
  await controller.refresh(); await controller.save(policy, "csrf");
  assert.equal(writes, 1); assert.equal(reads, 2); assert.equal(controller.getSnapshot().value.revision, 1); assert.equal(controller.getSnapshot().stale, false);
  assert.match(controller.getSnapshot().notice, /save was not confirmed/); await controller.refresh(); assert.equal(writes, 1); controller.dispose();
});

test("HTTP 403 expiry after a committed save reconciles; malformed forbidden bodies still clear access", async () => {
  const previous = globalThis.fetch; let saved = view(), reads = 0, writes = 0;
  globalThis.fetch = async (_url, init) => {
    if (init.method === "POST") {
      writes++; saved = view(1, JSON.parse(init.body).definition);
      return new Response(JSON.stringify({ code: "expired", message: "Fixture late committed reply", retry: false, audit_id: "fixture" }), { status: 403 });
    }
    reads++; return new Response(JSON.stringify(saved));
  };
  const controller = new HealthPolicyController(() => assert.fail("deadline is not session expiry"));
  try {
    await controller.refresh(); await controller.save({ ...definition(), fail_threshold: 3 }, "csrf");
    assert.equal(writes, 1); assert.equal(reads, 2); assert.equal(controller.getSnapshot().value.revision, 1); assert.equal(controller.getSnapshot().denied, false);
    globalThis.fetch = async () => new Response(JSON.stringify({ code: "expired", private: "must not render" }), { status: 403 });
    await controller.refresh(); assert.equal(controller.getSnapshot().denied, true); assert.equal(controller.getSnapshot().value, null); assert.doesNotMatch(controller.getSnapshot().error, /must not render/);
  } finally { controller.dispose(); globalThis.fetch = previous; }
});

test("one outstanding save excludes refresh and duplicate writes; stale and forbidden reads disable saving", async () => {
  let reads = 0, writes = 0; const held = pending();
  const controller = new HealthPolicyController(() => {}, { read: async () => { reads++; return view(); }, replace: async () => { writes++; return held.promise; } });
  await controller.refresh(); const save = controller.save(definition(), "csrf");
  await controller.refresh(); await controller.save(definition(), "csrf"); assert.equal(reads, 1); assert.equal(writes, 1);
  held.resolve(view(1)); await save; assert.equal(controller.getSnapshot().pending, false); controller.dispose();
  let failure = false;
  const denied = new HealthPolicyController(() => {}, { read: async () => { if (failure) throw error("forbidden"); return view(1); }, replace: async () => { assert.fail("denied mutation"); } });
  await denied.refresh(); failure = true; await denied.refresh();
  assert.equal(denied.getSnapshot().value, null); assert.equal(denied.getSnapshot().denied, true); await denied.save(definition(), "csrf"); denied.dispose();
});

test("disposed owner ignores late reads, saves and unauthenticated responses", async () => {
  let expired = 0; const held = pending();
  const controller = new HealthPolicyController(() => { expired++; }, { read: () => held.promise, replace: async () => view(1) });
  const read = controller.refresh(); controller.dispose(); held.reject(error("unauthenticated")); await read; assert.equal(expired, 0);
  const write = pending(); const owner = new HealthPolicyController(() => { expired++; }, { read: async () => view(), replace: () => write.promise });
  await owner.refresh(); const save = owner.save(definition(), "csrf"); owner.dispose(); write.resolve(view(1)); await save; assert.equal(owner.getSnapshot().value.revision, 0); assert.equal(expired, 0);
  const current = new HealthPolicyController(() => { expired++; }, { read: async () => { throw error("unauthenticated"); }, replace: async () => view(1) });
  await current.refresh(); await current.refresh(); assert.equal(expired, 1); assert.equal(current.getSnapshot().value, null); current.dispose();
});

test("policy form preserves metadata as escaped text and owner identity fences mounted state", () => {
  const props = { principal: "owner", grantRevision: 1, csrf: "csrf", expired() {} };
  const original = HealthPolicy(props);
  for (const change of [{ principal: "other" }, { grantRevision: 2 }, { csrf: "replacement" }]) assert.notEqual(HealthPolicy({ ...props, ...change }).key, original.key);
  const markup = renderToStaticMarkup(React.createElement(HealthPolicyForm, { value: view(1, { ...definition(), required_mounts: ["/mnt/Media", "/mnt/media"], silences: [{ ...silence(), reason: "<script>fixture</script>" }] }), disabled: true, pending: false, save() { assert.fail("render mutation"); } }));
  assert.match(markup, /2026-10-20T12:00:00.123456Z/); assert.match(markup, /mount:\/mnt\/Media/); assert.match(markup, /&lt;script&gt;/); assert.doesNotMatch(markup, /<script>/);
  assert.match(markup, /<fieldset disabled/); assert.match(markup, /Case and spelling are preserved/); assert.match(markup, /Save health settings/);
  assert.match(renderToStaticMarkup(React.createElement(HealthPolicy, props)), /New automatic observations are not available yet/);
});
