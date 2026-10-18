import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { networkInventoryExpired } from "./network-inventory-client";
import type { GroupState, LinkState, NetworkContainer, NetworkHealth, NetworkInventory, NetworkSource } from "./network-inventory-client";
import { NetworkInventoryController } from "./network-inventory-controller";

const sourceLabels: Record<NetworkSource, string> = { fresh: "Fresh complete observation", stale: "Observation expired", future: "Observation time is in the future", incomplete: "Collection incomplete", engine_mismatch: "Different Engine", unavailable: "Observation unavailable" };
const groupLabels: Record<GroupState, string> = { observed_running: "Provider running", provider_stopped: "Provider stopped", provider_unhealthy: "Provider health check failed", provider_starting: "Provider starting", health_unknown: "Provider health unknown", provider_network_unknown: "Provider network mode unknown", unsupported_topology: "Unsupported reference topology", orphan_hints: "Unresolved members with hints" };
const healthLabels: Record<NetworkHealth, string> = { healthy: "Healthy configured check", unhealthy: "Unhealthy configured check", starting: "Health check starting", not_configured: "No configured health check", unknown: "Health unknown" };
const linkLabels: Record<LinkState, string> = { resolved: "Reference resolved", missing_provider: "Provider missing", ambiguous_provider: "Multiple providers match", unsupported_container_prefix: "Short container ID unsupported", missing_project: "Project identity unavailable", self_reference: "Self reference", cyclic_provider: "Reference cycle", chained_provider: "Provider uses another reference" };
const sampled = (value: number | null) => value === null ? "No sample" : new Date(value * 1000).toISOString();
function Identity({ value }: { value: NetworkContainer }) {
  return <><strong>{value.name}</strong><small style={{ display: "block" }}><code style={{ overflowWrap: "anywhere" }}>{value.id}</code></small>
    {value.compose ? <small style={{ display: "block" }}>{value.compose.project.name} / {value.compose.service}{value.compose.one_off ? " · One-off" : " · Service / replica"}</small> : <small style={{ display: "block" }}>Outside Compose</small>}</>;
}
export function NetworkGroups({ value, loading = false, stale = false, error = "", now = Date.now() / 1000, refresh }: {
  value: NetworkInventory | null; loading?: boolean; stale?: boolean; error?: string; now?: number; refresh?: () => void;
}) {
  const current = value !== null && !stale && !networkInventoryExpired(value, now);
  const providers = new Map(value?.groups.map(g => [g.provider.id, g.provider.name]));
  const members = new Map(value?.members.map(m => [m.container.id, m]));
  return <section aria-labelledby="network-groups-heading" aria-busy={loading}>
    <div className="page-heading"><div><p className="eyebrow">CONTAINER NETWORKS</p><h2 id="network-groups-heading">Network groups</h2></div>
      {refresh && <button onClick={refresh} disabled={loading}>Refresh network groups</button>}</div>
    <p>Configured provider and member references. These observations do not confirm that traffic uses the provider or a VPN.</p>
    {loading && <p role="status">Reading cached network groups…</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {value && <p className="source-list"><span>{sourceLabels[value.source]}</span><span>Original sample: {sampled(value.sampled_at)}</span></p>}
    {!current && <p className="notice" role="status">Current network groups and counts are unknown.{value && value.source === "fresh" ? " This cached observation is no longer current." : ""}</p>}
    {current && value && <>
      <p>{value.observed_containers} {value.observed_containers === 1 ? "container" : "containers"} observed · {value.groups.length} configured {value.groups.length === 1 ? "group" : "groups"} · {value.independent_containers} independent {value.independent_containers === 1 ? "container" : "containers"} · {value.unknown_mode_containers} with unknown network mode</p>
      {value.observed_containers === 0 && <p>No containers were present in this complete observation.</p>}
      {value.observed_containers !== 0 && value.groups.length === 0 && <p>No configured shared-network groups were observed.</p>}
      {value.groups.map(group => <article className="stack-card" key={group.provider.id}>
        <h3>{group.provider.name} <span className={`badge${group.state === "observed_running" ? "" : " warning"}`}>{groupLabels[group.state]}</span></h3>
        <p>{group.provider.running ? "Running" : "Not running"} · {healthLabels[group.provider.health]}</p>
        <details><summary>Provider identity</summary><Identity value={group.provider} />
          {group.provider.compose && <p><code style={{ overflowWrap: "anywhere" }}>{group.provider.compose.project.working_directory}</code></p>}</details>
        <h4>Referenced members</h4>
        {group.member_ids.length === 0 ? <p>No resolved member association. Suggested members remain unverified.</p> : <ul className="member-list">{group.member_ids.map(id => {
          const member = members.get(id)!;
          return <li key={id}>{member.container.name}{member.container.compose?.one_off ? " · One-off" : ""} · {linkLabels[member.state]}</li>;
        })}</ul>}
        {group.hinted_orphan_ids.length > 0 && <div className="notice"><h4>Unverified orphan hints</h4><p>A dependency label suggests this provider. It does not repair the missing reference.</p>
          <ul>{group.hinted_orphan_ids.map(id => <li key={id}>{members.get(id)!.container.name} · Suggested provider only</li>)}</ul></div>}
      </article>)}
      {value.members.length > 0 && <div className="table-wrap"><table><caption>Observed network references</caption>
        <thead><tr><th scope="col">Member</th><th scope="col">Observed state</th><th scope="col">Configured reference</th><th scope="col">Association</th></tr></thead>
        <tbody>{value.members.map(member => <tr key={member.container.id}>
          <th scope="row"><Identity value={member.container} /></th><td>{member.container.running ? "Running" : "Not running"}<small>{healthLabels[member.container.health]}</small></td>
          <td>{member.reference_kind}:<code style={{ overflowWrap: "anywhere" }}>{member.target}</code></td>
          <td>{linkLabels[member.state]}{member.provider_id && <small>Provider: {providers.get(member.provider_id)}</small>}
            {member.hinted_provider_id && <small>Unverified hint: {providers.get(member.hinted_provider_id)}</small>}
            {member.dependency_hint && !member.hinted_provider_id && <small>Dependency hint (unverified): {member.dependency_hint}</small>}</td>
        </tr>)}</tbody>
      </table></div>}
    </>}
  </section>;
}

/** Mount inside the shared authenticated shell; every identity gets a new owner. */
export function NetworkGroupsPanel({ principal, grantRevision, expired }: { principal: string | null; grantRevision: number; expired: () => void }) {
  if (!principal || !Number.isSafeInteger(grantRevision) || grantRevision < 1) return null;
  return <AuthenticatedNetworks principal={principal} grantRevision={grantRevision} expired={expired} />;
}
function AuthenticatedNetworks({ principal, grantRevision, expired }: { principal: string; grantRevision: number; expired: () => void }) {
  const expiry = useRef(expired); expiry.current = expired;
  const controller = useMemo(() => new NetworkInventoryController(() => expiry.current()), [principal, grantRevision]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  useEffect(() => {
    void controller.start(); const refresh = () => { void controller.refresh(); };
    window.addEventListener("focus", refresh); window.addEventListener("online", refresh);
    return () => { window.removeEventListener("focus", refresh); window.removeEventListener("online", refresh); controller.dispose(); };
  }, [controller]);
  if (state.expired) return null;
  return <NetworkGroups value={state.value} loading={state.loading} stale={state.stale} error={state.error} refresh={state.denied ? undefined : () => { void controller.refresh(); }} />;
}
