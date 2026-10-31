import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { HealthPolicyController } from "./health-policy-controller";
import { healthKinds, validHealthPolicy } from "./health-policy-client";
import type { HealthPolicyDefinition, HealthPolicySilence, HealthPolicyView } from "./health-policy-client";

const useOwnerEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;
export function HealthPolicy({ principal, grantRevision, csrf, expired }: {
  principal: string; grantRevision: number; csrf: string; expired: () => void;
}) {
  return <OwnedHealthPolicy key={JSON.stringify([principal, grantRevision, csrf])} csrf={csrf} expired={expired} />;
}
function OwnedHealthPolicy({ csrf, expired }: { csrf: string; expired: () => void }) {
  const callback = useRef(expired); callback.current = expired;
  const controller = useMemo(() => new HealthPolicyController(() => callback.current()), []);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useOwnerEffect(() => {
    void controller.refresh(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  return <section id="health-policy-panel" aria-labelledby="health-policy-heading">
    <p className="eyebrow">HEALTH SETTINGS</p><h2 id="health-policy-heading">Alert policy</h2>
    <p className="notice">These settings apply to your health schedules and notifications. New automatic observations are not available yet. Poll timing is managed in health schedules.</p>
    <button disabled={state.loading || state.pending} onClick={() => { void controller.refresh(); }}>Refresh health settings</button>
    {state.loading && <p role="status">Reading saved health settings…</p>}
    {state.error && <p role="alert" className="error">{state.error}</p>}
    {state.notice && <p role="status">{state.notice}</p>}
    {state.stale && state.value && <p className="notice">These saved settings may be out of date. Refresh before saving.</p>}
    {state.value && <HealthPolicyForm key={state.value.revision} value={state.value}
      disabled={state.stale || state.loading || state.pending || state.denied} pending={state.pending}
      save={definition => { void controller.save(definition, csrf); }} />}
  </section>;
}
export function HealthPolicyForm({ value, disabled, pending, save }: {
  value: HealthPolicyView; disabled: boolean; pending: boolean; save: (definition: HealthPolicyDefinition) => void;
}) {
  const [definition, setDefinition] = useState(value.definition);
  const [mounts, setMounts] = useState(value.definition.required_mounts.join("\n"));
  const [error, setError] = useState("");
  const [silenceKind, setSilenceKind] = useState<HealthPolicySilence["kind"]>("mount");
  const [silenceKey, setSilenceKey] = useState(""); const [reason, setReason] = useState(""); const [expiry, setExpiry] = useState("");
  const categoryNames = { container: "Containers", smart: "Disk health (SMART)", mount: "Required mounts", snapraid: "SnapRAID protection", host: "Host health" };
  return <form className="health-policy-form" aria-busy={pending} onSubmit={event => {
    event.preventDefault(); if (disabled) return;
    const next = { ...definition, required_mounts: mounts.split("\n").filter(path => path !== "") };
    if (!validHealthPolicy(next)) { setError("Check the threshold, absolute paths and silence metadata. Remove duplicate paths or unsupported values before saving."); return; }
    setError(""); save(next);
  }}>
    <p>{value.revision === 0 ? "Using defaults; no explicit policy has been saved." : `Saved revision ${value.revision} · ${new Date(value.updated! * 1000).toISOString()}`}</p>
    <fieldset disabled={disabled}>
      <legend>Observation and delivery rules</legend>
      <label htmlFor="health-fail-threshold">Consecutive failures before an incident</label>
      <input id="health-fail-threshold" type="number" min={1} max={16} required value={definition.fail_threshold} onChange={event => setDefinition({ ...definition, fail_threshold: Number(event.target.value) })} />
      <p>Disabling a category or silencing a resource stops notification delivery. Existing incidents remain recorded.</p>
      <div className="health-category-grid">{healthKinds.map(kind => <label key={kind}>
        <input type="checkbox" checked={!definition.disabled.includes(kind)} onChange={event => setDefinition({ ...definition, disabled: event.target.checked ? definition.disabled.filter(value => value !== kind) : [...definition.disabled, kind] })} />{categoryNames[kind]}
      </label>)}</div>
      <label htmlFor="health-required-mounts">Required mount paths</label>
      <textarea id="health-required-mounts" rows={5} value={mounts} onChange={event => setMounts(event.target.value)} aria-describedby="health-mount-help" />
      <p id="health-mount-help">One absolute path per line, up to 256 paths. Case and spelling are preserved. An empty list is an explicit empty selection.</p>
    </fieldset>
    <fieldset disabled={disabled}>
      <legend>Resource silences</legend>
      {definition.silences.length === 0 && <p>No resource silences configured.</p>}
      <ul className="health-silences">{definition.silences.map((silence, index) => <li key={index}>
        <strong>{silence.key}</strong><p>{silence.reason || "No reason recorded"}</p>
        <small>Created {silence.created_at} · {silence.expires_at ? `Expires ${silence.expires_at}` : "No expiry"}</small>
        <button type="button" onClick={() => setDefinition({ ...definition, silences: definition.silences.filter((_, row) => row !== index) })} aria-label={`Remove silence for ${silence.key}`}>Remove silence</button>
      </li>)}</ul>
      <div className="health-silence-fields">
        <label htmlFor="health-silence-kind">Category</label><select id="health-silence-kind" value={silenceKind} onChange={event => setSilenceKind(event.target.value as HealthPolicySilence["kind"])}>{healthKinds.map(kind => <option key={kind} value={kind}>{categoryNames[kind]}</option>)}</select>
        <label htmlFor="health-silence-key">Exact alert key</label><input id="health-silence-key" value={silenceKey} onChange={event => setSilenceKey(event.target.value)} placeholder={`${silenceKind}:…`} maxLength={256} />
        <label htmlFor="health-silence-reason">Reason (optional)</label><input id="health-silence-reason" value={reason} onChange={event => setReason(event.target.value)} maxLength={400} />
        <label htmlFor="health-silence-expiry">Expiry (optional, with timezone)</label><input id="health-silence-expiry" value={expiry} onChange={event => setExpiry(event.target.value)} placeholder="2026-10-31T07:00:00Z" maxLength={32} aria-describedby="health-expiry-help" />
        <p id="health-expiry-help">Use whole seconds with Z or a timezone offset. Delivery resumes exactly at expiry. Leave empty for no expiry.</p>
        <button type="button" disabled={definition.silences.length >= 64} onClick={() => {
          const silence: HealthPolicySilence = { kind: silenceKind, key: silenceKey.trim(), created_at: new Date().toISOString(), expires_at: expiry || null, reason: reason.trim() };
          const next = { ...definition, silences: [...definition.silences, silence] };
          if (!validHealthPolicy(next)) { setError("Enter an exact category-prefixed alert key, a supported expiry and a reason of at most 200 characters."); return; }
          setDefinition(next); setSilenceKey(""); setReason(""); setExpiry(""); setError("");
        }}>Add silence to draft</button>
      </div>
    </fieldset>
    {error && <p role="alert" className="error">{error}</p>}
    <p>Changes take effect only when you save this policy.</p>
    <button disabled={disabled}>{pending ? "Saving health settings…" : "Save health settings"}</button>
  </form>;
}
