import { useEffect, useId, useMemo, useRef, useSyncExternalStore } from "react";
import { ageToolsStatus } from "./tools-status-client";
import type { Component, ConfigIssue, ConfigState, Freshness, PayloadState, ServiceIssue, ServiceState, ToolsStatus as Report } from "./tools-status-client";
import { ToolsStatusController } from "./tools-status-controller";

const freshnessLabels: Record<Freshness, string> = { fresh: "Current observation", unavailable: "No observation", stale: "Observation expired", future: "Observation time is in the future" };
const configIssues: Record<ConfigIssue, string> = { unavailable: "Configuration could not be read", incomplete: "Configuration read incomplete", invalid_document: "Configuration format unsupported", unsupported_options: "Legacy options require review", invalid_path: "Configured share path unsupported", invalid_port: "Configured port invalid", limit: "Configuration exceeds the supported limit" };
const payloadLabels: Record<PayloadState, string> = { absent: "Legacy payload absent", present: "Legacy payload present; authenticity unverified", unexpected_type: "Unexpected payload file type", unavailable: "Payload could not be checked" };
const serviceLabels: Record<Exclude<ServiceState["state"], "unknown">, string> = { active: "Service active", inactive: "Service inactive", failed: "Service failed", activating: "Service starting", deactivating: "Service stopping", reloading: "Service reloading", not_found: "Service unit missing", masked: "Service unit masked", load_error: "Service unit could not be loaded" };
const serviceIssues: Record<ServiceIssue, string> = { unavailable: "Service state could not be read", incomplete: "Service observation incomplete", limit: "Service observation exceeds the supported limit", invalid_properties: "Service properties unsupported" };
function Sample({ value }: { value?: Component<unknown> }) {
  return <><span className={`badge${value?.freshness === "fresh" ? "" : " warning"}`}>{value ? freshnessLabels[value.freshness] : "No current observation"}</span>
    <small style={{ display: "block", marginTop: 8 }}>{value?.sampled_at === null || value?.sampled_at === undefined ? "No sample" : <>Original sample: <time dateTime={new Date(value.sampled_at * 1000).toISOString()}>{new Date(value.sampled_at * 1000).toISOString()}</time></>}</small></>;
}
function Config({ value }: { value: ConfigState | null }) {
  if (!value) return <p>Current configuration unknown.</p>;
  if (value.state === "unknown") return <p>{configIssues[value.reason]}.</p>;
  return <><p>{value.state === "defaults" ? "Configuration file absent; defaults shown." : "Saved configuration"}</p>
    <dl className="plan-details"><dt>Share path</dt><dd>{value.config.share_path}</dd><dt>Port</dt><dd>{value.config.port}</dd></dl></>;
}
export function ToolsStatus({ value, loading = false, error = "", now = Date.now() / 1000, refresh }: {
  value: Report | null; loading?: boolean; error?: string; now?: number; refresh?: () => void;
}) {
  const headingId = useId();
  const current = value ? ageToolsStatus(value, now) : null;
  const payload = current?.payload.value; const service = current?.service.value;
  return <section aria-labelledby={headingId} aria-busy={loading}>
    <div className="page-heading"><div><p className="eyebrow">TOOLS</p><h2 id={headingId}>CopyParty</h2></div>
      {refresh && <button onClick={refresh} disabled={loading}>Refresh tools status</button>}</div>
    <p>Saved settings, legacy payload and service observations are checked separately.</p>
    {loading && <p role="status">Reading cached tools status…</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {!current && <p className="notice" role="status">Current tools status is unknown.</p>}
    <div className="source-list">
      <article><h3>Configuration</h3><Sample value={current?.config} /><Config value={current?.config.value ?? null} /></article>
      <article><h3>Legacy payload</h3><Sample value={current?.payload} /><p>{payload ? payloadLabels[payload] : "Current payload presence unknown"}.</p></article>
      <article><h3>Service</h3><Sample value={current?.service} /><p>{service ? service.state === "unknown" ? serviceIssues[service.reason] : serviceLabels[service.state] : "Current service state unknown"}.</p></article>
    </div>
    <p className="notice">Loaded configuration is unverified. An active service or matching saved settings does not confirm readiness or access to files.</p>
  </section>;
}
type PanelProps = { principal: string | null; grantRevision: number | null; hostLifetime: string | null; expired: () => void };
const owner = (v: string | null): v is string => typeof v === "string" && v.length > 0 && new TextEncoder().encode(v).length <= 256 && v.trim() === v && !/[\p{Cc}\uD800-\uDFFF]/u.test(v);
export function ToolsStatusPanel(props: PanelProps) {
  if (!owner(props.principal) || !owner(props.hostLifetime) || !Number.isSafeInteger(props.grantRevision) || props.grantRevision! < 1) return <ToolsStatus value={null} />;
  return <OwnedToolsStatus key={JSON.stringify([props.principal, props.grantRevision, props.hostLifetime])} expired={props.expired} />;
}
function OwnedToolsStatus({ expired }: { expired: () => void }) {
  const callback = useRef(expired); callback.current = expired;
  const controller = useMemo(() => new ToolsStatusController(() => callback.current()), []);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => {
    void controller.start(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  return <ToolsStatus value={state.value} loading={state.loading} error={state.error} refresh={state.expired || state.denied ? undefined : () => { void controller.refresh(); }} />;
}
