import assert from "node:assert/strict";
import { test } from "node:test";
import { api, ApiError } from "../src/api.ts";

test("login sends credentials in the body with same-origin cookies and a deadline", async (t) => {
  const fixture = {
    username: "fixture-user",
    password: "synthetic-test-password",
  };
  t.mock.method(globalThis, "fetch", async (url, options) => {
    assert.equal(url, "/api/v1/auth/login");
    assert.equal(options.method, "POST");
    assert.equal(options.credentials, "same-origin");
    assert.equal(options.headers["Content-Type"], "application/json");
    assert.ok(options.signal instanceof AbortSignal);
    assert.deepEqual(JSON.parse(options.body), fixture);
    assert.ok(!url.includes(fixture.password));
    return Response.json({ csrf_token: "fixture-csrf" });
  });
  assert.equal((await api.login(fixture)).csrf_token, "fixture-csrf");
});

test("logout supplies CSRF over POST, and session reads have no mutation body", async (t) => {
  const calls = [];
  t.mock.method(globalThis, "fetch", async (url, options) => {
    calls.push([url, options]);
    return url.endsWith("logout")
      ? new Response(null, { status: 204 })
      : Response.json({});
  });
  assert.equal(await api.logout("fixture-csrf"), undefined);
  await api.session();
  assert.equal(calls[0][1].method, "POST");
  assert.equal(calls[0][1].headers["X-CSRF-Token"], "fixture-csrf");
  assert.equal(calls[1][1].method, undefined);
  assert.equal(calls[1][1].body, undefined);
});

test("safe API errors retain their details; malformed proxy responses get a fixed message", async (t) => {
  const detail = {
    code: "forbidden",
    message: "The request is outside the permitted scope.",
    retry: false,
    audit_id: "fixture-id",
  };
  t.mock.method(globalThis, "fetch", async () =>
    Response.json(detail, { status: 403 }),
  );
  await assert.rejects(api.session(), (error) => {
    assert.ok(error instanceof ApiError);
    assert.deepEqual(error.detail, detail);
    assert.equal(error.message, detail.message);
    return true;
  });
  for (const body of [
    "<html>private proxy diagnostic</html>",
    '{"message":"private proxy diagnostic"}',
  ]) {
    globalThis.fetch.mock.mockImplementation(
      async () => new Response(body, { status: 503 }),
    );
    await assert.rejects(
      api.session(),
      (error) =>
        error instanceof ApiError &&
        error.detail.code === "unavailable" &&
        !error.message.includes("private"),
    );
  }
});

test("restart approval and queue use POST and CSRF, retaining the same retry key", async (t) => {
  const calls = [];
  const proposal = { plan: { id: "a".repeat(64) }, digest: "b".repeat(64) };
  t.mock.method(globalThis, "fetch", async (url, options) => {
    calls.push([url, options]);
    return Response.json({});
  });
  await api.planRestart("container:fixture", "csrf");
  await api.approveRestart(proposal, "csrf");
  await api.queueRestart(proposal, "private-approval", "same-key", "csrf");
  await api.queueRestart(proposal, "private-approval", "same-key", "csrf");
  for (const [url, options] of calls) {
    assert.equal(options.method, "POST");
    assert.equal(options.headers["X-CSRF-Token"], "csrf");
    assert.ok(!url.includes("private-approval"));
  }
  assert.deepEqual(JSON.parse(calls[2][1].body), JSON.parse(calls[3][1].body));
});
