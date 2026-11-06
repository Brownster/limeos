import test from "node:test";
import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ResolutionController, validateReason, disabledReason, asView, isJobId, NeedsAttention, ResolutionBody } from "./helpers/resolution.mjs";

const job = "e".repeat(64);
const evidence = "9f".repeat(32);
const container = c => c.repeat(64);
const expected = c => ({ resource: `container:${container(c)}`, image: `sha256:${"b".repeat(64)}`, started_at: "2026-10-05T07:00:00Z", running: true });
const measured = (c, over = {}) => ({ container: container(c), running: true, started_at: "2026-10-05T07:05:00Z", image: `sha256:${"b".repeat(64)}`,
  matches_expected: false, intended_state_reached: true, ...over });
const view = (over = {}) => ({
  job, state: "needs_intervention", kind: "stack", operation: "restart",
  targets: [{ expected: expected("a"), measured: measured("a") }, { expected: expected("c"), measured: measured("c") }],
  evidence_digest: evidence, allowed: ["verified_done", "accepted_divergence"],
  members: [{ container: container("a"), state: "verified", error: null }, { container: container("c"), state: "outcome_unknown", error: "unavailable" }],
  pending: null, resolution: null, ...over,
});
const record = declared => ({ job, resolved_by: "admin", declared, reason: "r", evidence: { targets: [] }, evidence_digest: evidence, requested_at: 1, resolved_at: 2 });
const reply = (status, body) => ({ status, body });
const envelope = (code, message = "No.") => ({ code, message, retry: false, audit_id: "a-1" });
const resolvedBody = { status: "resolved", message: "done", resolution: record("verified_done"), view: null, blocking: [] };
const fine = "Checked docker ps by hand; jellyfin is fine.";

function harness({ read, submit, session } = {}) {
  const calls = [];
  const client = {
    read: async (...a) => { calls.push(["read", ...a]); return read ? read(calls) : reply(200, view()); },
    submit: async (...a) => { calls.push(["submit", ...a]); return submit ? submit(calls) : reply(200, resolvedBody); },
    session: async () => { calls.push(["session"]); return session ?? "valid"; },
  };
  let changes = 0, expiries = 0;
  const controller = new ResolutionController(job, "csrf", client, () => { changes += 1; }, () => { expiries += 1; }, async () => {}, 3);
  return { controller, calls, changes: () => changes, expiries: () => expiries, state: () => controller.getSnapshot() };
}
async function ready(options, declared = "verified_done", reason = fine) {
  const h = harness(options);
  await h.controller.load();
  h.controller.choose(declared); h.controller.setReason(reason); h.controller.review();
  assert.equal(h.state().step, "confirm");
  return h;
}
const submits = h => h.calls.filter(c => c[0] === "submit");

test("reason validation mirrors the server rules", () => {
  assert.match(validateReason("not_done", ""), /Write what/);
  assert.equal(validateReason("not_done", "x"), "");
  assert.equal(validateReason("not_done", "x".repeat(500)), "");
  assert.match(validateReason("not_done", "x".repeat(501)), /limited to 500/);
  assert.match(validateReason("not_done", " leading"), /start or end/);
  assert.match(validateReason("not_done", "trailing\n"), /start or end/);
  assert.match(validateReason("not_done", "two\nlines"), /control characters/);
  assert.match(validateReason("not_done", "tab\there"), /control characters/);
  assert.match(validateReason("accepted_divergence", "x".repeat(19)), /at least 20/);
  assert.equal(validateReason("accepted_divergence", "x".repeat(20)), "");
  assert.equal(validateReason("accepted_divergence", "😀".repeat(20)), "");
  assert.equal(isJobId(job), true); assert.equal(isJobId("../x"), false);
});

test("load shows fresh evidence, only allowed outcomes can be chosen, and review sends nothing", async () => {
  const h = harness();
  await h.controller.load();
  assert.equal(h.state().view.evidence_digest, evidence);
  h.controller.choose("not_done");
  assert.equal(h.state().declared, null);
  h.controller.choose("accepted_divergence");
  h.controller.setReason("short"); h.controller.review();
  assert.equal(h.state().step, "choose"); assert.match(h.state().invalid, /at least 20/);
  h.controller.setReason(fine); h.controller.review();
  assert.equal(h.state().step, "confirm");
  assert.deepEqual(h.calls.map(c => c[0]), ["read"]);
  h.controller.back();
  assert.equal(h.state().step, "choose");
});

test("a 200 resolves, sends the exact body and refreshes the jobs list", async () => {
  const h = await ready();
  await h.controller.submit("hunter2");
  const [[, id, input, csrf]] = submits(h);
  assert.equal(id, job); assert.equal(csrf, "csrf");
  assert.deepEqual(input, { declared: "verified_done", reason: fine, evidence_digest: evidence, password: "hunter2" });
  assert.equal(h.state().result, "resolved"); assert.equal(h.state().resolution.declared, "verified_done");
  assert.equal(h.changes(), 1);
  await h.controller.submit("hunter2");
  assert.equal(submits(h).length, 1);
});

test("the password is never kept in controller state", async () => {
  const h = await ready();
  await h.controller.submit("s3cret-password");
  assert.doesNotMatch(JSON.stringify(h.state()), /s3cret/);
  const refused = await ready({ submit: async () => reply(401, envelope("unauthenticated")) });
  await refused.controller.submit("s3cret-password");
  assert.doesNotMatch(JSON.stringify(refused.state()), /s3cret/);
});

test("a 202 says recorded and waiting; the identical submit may be repeated by the user only", async () => {
  const h = await ready({ submit: async () => reply(202, { status: "pending", message: "held", resolution: null, view: null, blocking: [] }) });
  await h.controller.submit("pw");
  assert.equal(h.state().result, "pending"); assert.match(h.state().notice, /Recorded, waiting for the executor/);
  assert.equal(h.state().step, "confirm"); assert.equal(h.changes(), 1);
  assert.equal(submits(h).length, 1);
  await h.controller.submit("pw");
  assert.equal(submits(h).length, 2);
  assert.deepEqual(submits(h)[0].slice(1), submits(h)[1].slice(1));
});

test("409 stale_evidence replaces the view, clears the confirmation and says the state changed", async () => {
  const fresh = view({ evidence_digest: "1".repeat(64), allowed: ["not_done", "accepted_divergence"] });
  const h = await ready({ submit: async () => reply(409, { status: "stale_evidence", message: "x", resolution: null, view: fresh, blocking: [] }) });
  await h.controller.submit("pw");
  const s = h.state();
  assert.equal(s.view.evidence_digest, "1".repeat(64)); assert.equal(s.step, "choose");
  assert.equal(s.declared, null); assert.match(s.notice, /state changed/);
  assert.equal(submits(h).length, 1);
  h.controller.review();
  assert.equal(h.state().step, "choose");
});

test("409 stale_evidence keeps a choice that the fresh evidence still allows", async () => {
  const fresh = view({ evidence_digest: "1".repeat(64) });
  const h = await ready({ submit: async () => reply(409, { status: "stale_evidence", message: "x", resolution: null, view: fresh, blocking: [] }) });
  await h.controller.submit("pw");
  assert.equal(h.state().declared, "verified_done"); assert.equal(h.state().step, "choose");
});

test("409 unsupported_declaration shows the per-target diff and the blocking containers", async () => {
  const diff = view({ targets: [{ expected: expected("a"), measured: measured("a", { intended_state_reached: false }) }], allowed: ["accepted_divergence"] });
  const h = await ready({ submit: async () => reply(409, { status: "unsupported_declaration", message: "a differs", resolution: null, view: diff, blocking: [container("a")] }) });
  await h.controller.submit("pw");
  assert.deepEqual(h.state().blocking, [container("a")]); assert.match(h.state().error, /does not support/);
  assert.equal(h.state().declared, null); assert.equal(h.state().step, "choose");
  const out = renderToStaticMarkup(React.createElement(ResolutionBody, { state: h.state() }));
  assert.match(out, /Blocks this outcome/); assert.match(out, /aaaaaaaaaaaa/);
});

test("409 conflict explains and re-reads the job", async () => {
  const h = await ready({ submit: async () => reply(409, envelope("conflict", "Another request is open.")) });
  await h.controller.submit("pw");
  assert.match(h.state().error, /Another request is open\. · reference a-1/);
  assert.equal(h.calls.filter(c => c[0] === "read").length, 2); assert.equal(h.state().step, "choose");
});

test("401 re-checks the session: wrong password versus expired session", async () => {
  const wrong = await ready({ submit: async () => reply(401, envelope("unauthenticated")), session: "valid" });
  await wrong.controller.submit("pw");
  assert.match(wrong.state().error, /password was not accepted/); assert.equal(wrong.expiries(), 0);
  assert.equal(wrong.state().step, "confirm"); assert.equal(wrong.calls.at(-1)[0], "session");
  const gone = await ready({ submit: async () => reply(401, envelope("unauthenticated")), session: "expired" });
  await gone.controller.submit("pw");
  assert.equal(gone.expiries(), 1); assert.equal(gone.state().expired, true);
  const unknown = await ready({ submit: async () => reply(401, envelope("unauthenticated")), session: "unknown" });
  await unknown.controller.submit("pw");
  assert.match(unknown.state().error, /could not be checked/); assert.equal(unknown.expiries(), 0);
});

test("403, 429, 503 and 400 map to distinct messages and record nothing", async () => {
  const cases = [[403, /Only an administrator/], [429, /Too many password attempts/], [503, /evidence could not be gathered/], [400, /Bad reason/]];
  for (const [status, pattern] of cases) {
    const h = await ready({ submit: async () => reply(status, envelope("invalid_input", "Bad reason.")) });
    await h.controller.submit("pw");
    assert.match(h.state().error, pattern); assert.equal(h.state().result, null); assert.equal(h.state().submitting, false);
    assert.equal(submits(h).length, 1);
  }
});

test("a network or 5xx failure on POST is never retried automatically", async () => {
  const lost = await ready({ submit: async () => { throw new Error("network"); } });
  await lost.controller.submit("pw");
  assert.equal(lost.state().interrupted, true); assert.match(lost.state().error, /may have been recorded/);
  assert.equal(submits(lost).length, 1);
  const bad = await ready({ submit: async () => reply(502, null) });
  await bad.controller.submit("pw");
  assert.equal(bad.state().interrupted, true); assert.equal(submits(bad).length, 1);
});

test("GET retries only network and 5xx failures, never a definite refusal", async () => {
  let n = 0;
  const flaky = harness({ read: async () => { n += 1; if (n === 1) throw new Error("network"); if (n === 2) return reply(503, envelope("unavailable")); return reply(200, view()); } });
  await flaky.controller.load();
  assert.equal(n, 3); assert.equal(flaky.state().view.job, job); assert.equal(flaky.state().error, "");
  const down = harness({ read: async () => reply(503, envelope("unavailable")) });
  await down.controller.load();
  assert.equal(down.calls.length, 3); assert.match(down.state().error, /evidence could not be gathered/);
  for (const [status, pattern] of [[403, /Only an administrator/], [404, /no longer exists/]]) {
    const refused = harness({ read: async () => reply(status, envelope("forbidden")) });
    await refused.controller.load();
    assert.equal(refused.calls.length, 1); assert.match(refused.state().error, pattern);
  }
  const gone = harness({ read: async () => reply(401, envelope("unauthenticated")) });
  await gone.controller.load();
  assert.equal(gone.expiries(), 1);
});

test("submit is refused without a confirm step, a valid reason, a password or while submitting", async () => {
  const h = harness();
  await h.controller.load();
  await h.controller.submit("pw");
  assert.equal(submits(h).length, 0);
  h.controller.choose("accepted_divergence"); h.controller.setReason("too short"); h.controller.review();
  await h.controller.submit("pw");
  assert.equal(submits(h).length, 0);
  h.controller.setReason(fine); h.controller.review();
  await h.controller.submit("");
  assert.equal(submits(h).length, 0); assert.match(h.state().error, /Enter your password/);
  let release; const gate = new Promise(r => { release = r; });
  const slow = await ready({ submit: async () => { await gate; return reply(200, resolvedBody); } });
  const first = slow.controller.submit("pw");
  await slow.controller.submit("pw");
  release(); await first;
  assert.equal(submits(slow).length, 1);
});

test("a late answer after dispose is ignored", async () => {
  let release; const gate = new Promise(r => { release = r; });
  const h = harness({ read: async () => { await gate; return reply(200, view()); } });
  const loading = h.controller.load();
  h.controller.dispose(); release(); await loading;
  assert.equal(h.state().view, null);
});

const stateOf = async (v = view(), steps = () => {}) => {
  const h = harness({ read: async () => reply(200, v) });
  await h.controller.load(); steps(h.controller);
  return h.state();
};
const html = (state, props = {}) => renderToStaticMarkup(React.createElement(ResolutionBody, { state, ...props }));

test("the dialog shows plan against measured per target, receipt outcomes and the no-side-effects statement", async () => {
  const out = html(await stateOf());
  assert.match(out, /never\s+starts, stops or restarts anything/);
  assert.match(out, /Plan expected/); assert.match(out, /Measured now/);
  assert.match(out, /aaaaaaaaaaaa/); assert.match(out, /cccccccccccc/);
  assert.match(out, /outcome unknown \(unavailable\)/); assert.match(out, /What the executor recorded/);
  assert.match(out, /Differs from the plan/);
});

test("disallowed outcomes are disabled and explain why; divergence is distinct", async () => {
  const state = await stateOf(view({ targets: [{ expected: expected("a"), measured: measured("a", { intended_state_reached: false }) }], allowed: ["accepted_divergence"] }));
  const out = html(state);
  assert.match(out, /Intended end state is present/); assert.match(out, /Nothing changed — release the lock only/);
  assert.match(out, /Accept the present state as it is/);
  assert.match(out, /disabled="" aria-describedby="why-verified_done"/); assert.match(out, /disabled="" aria-describedby="why-not_done"/);
  assert.doesNotMatch(out, /disabled="" aria-describedby="why-accepted_divergence"/);
  assert.match(out, /Not available: aaaaaaaaaaaa is not in the intended end state/);
  assert.match(out, /Not available: aaaaaaaaaaaa differs from the plan/);
  assert.match(out, /class="outcome divergence"/);
  assert.equal(disabledReason(view(), "verified_done"), null);
  const missing = view({ targets: [{ expected: expected("a"), measured: { ...measured("a"), image: "", started_at: "", running: false, intended_state_reached: false } }], allowed: ["accepted_divergence"] });
  assert.match(html(await stateOf(missing)), /no longer has this container/);
});

test("divergence asks for at least 20 characters; the others for 1 to 500", async () => {
  assert.match(html(await stateOf(view(), c => c.choose("accepted_divergence"))), /at least 20, at most 500 characters/);
  assert.match(html(await stateOf(view(), c => c.choose("verified_done"))), /1 to 500 characters/);
  const short = await stateOf(view(), c => { c.choose("accepted_divergence"); c.setReason("tiny"); c.review(); });
  assert.match(html(short), /at least 20 characters \(4 written\)/);
  assert.equal(short.step, "choose");
});

test("the confirm step shows the digest-bound summary and a correctly typed password field", async () => {
  const state = await stateOf(view(), c => { c.choose("accepted_divergence"); c.setReason(fine); c.review(); });
  const out = html(state);
  assert.match(out, new RegExp(evidence)); assert.match(out, /bound to the evidence digest/);
  assert.match(out, /type="password"/); assert.match(out, /autoComplete="current-password"/);
  assert.match(out, /class="divergence summary"/); assert.match(out, /disabled=""[^>]*>Resolve job/);
  assert.match(html(state, { password: "x" }), /<button type="submit" class="danger">Resolve job/);
  assert.doesNotMatch(out, /value="[^"]*hunter/);
});

test("stale evidence refreshes the view in the rendered dialog and announces the change", async () => {
  const fresh = view({ evidence_digest: "1".repeat(64), allowed: ["not_done", "accepted_divergence"], targets: [{ expected: expected("a"), measured: measured("a", { matches_expected: true, intended_state_reached: false }) }] });
  const h = await ready({ submit: async () => reply(409, { status: "stale_evidence", message: "x", resolution: null, view: fresh, blocking: [] }) });
  await h.controller.submit("pw");
  const out = html(h.state());
  assert.match(out, /state changed since you reviewed it/); assert.match(out, /aria-live="polite"/);
  assert.doesNotMatch(out, /Confirm with your password/); assert.match(out, /Your decision/);
  assert.doesNotMatch(out, /aria-label="Container cccccccccccc"/);
});

test("pending and resolved records are shown read-only without a form", async () => {
  const resolved = html(await stateOf(view({ state: "resolved", targets: [], evidence_digest: null, allowed: [], resolution: record("not_done") })));
  assert.match(resolved, /Already resolved/); assert.doesNotMatch(resolved, /Your decision/);
  const pending = html(await stateOf(view({ targets: [], evidence_digest: null, allowed: [], pending: record("not_done") })));
  assert.match(pending, /waiting for the executor/); assert.doesNotMatch(pending, /Your decision/);
  const done = await ready();
  await done.controller.submit("pw");
  assert.match(html(done.state()), /Resolved/);
  assert.equal(asView({}), null);
});

test("the banner is information for everyone and a resolve button only for administrators", () => {
  const jobs = [{ id: job, title: "Restart · aaaaaaaaaaaa" }];
  const props = { csrf: "c", expired() {}, changed() {} };
  const admin = renderToStaticMarkup(React.createElement(NeedsAttention, { jobs, admin: true, ...props }));
  assert.match(admin, /Needs attention · 1 operation/); assert.match(admin, /Review and resolve Restart · aaaaaaaaaaaa/);
  assert.doesNotMatch(admin, /Ask an administrator/);
  const other = renderToStaticMarkup(React.createElement(NeedsAttention, { jobs, admin: false, ...props }));
  assert.match(other, /Needs attention/); assert.match(other, /Ask an administrator to review/);
  assert.doesNotMatch(other, /<button/);
  assert.equal(renderToStaticMarkup(React.createElement(NeedsAttention, { jobs: [], admin: true, ...props })), "");
});
